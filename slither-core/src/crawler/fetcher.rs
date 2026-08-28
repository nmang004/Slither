use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Ceiling on a single response body, applied to the decompressed bytes.
const MAX_BODY_SIZE: usize = 10_000_000; // 10 MB

/// Longest server-requested backoff we will actually wait out. Past this, the
/// polite thing is to stop asking rather than sleep for minutes.
const MAX_RETRY_AFTER_SECS: u64 = 30;

/// Whether appending `incoming` bytes to `read` would cross `max`. Saturating so
/// a hostile `Content-Length` or chunk size can't wrap the addition and slip
/// past the budget.
fn exceeds_limit(read: usize, incoming: usize, max: usize) -> bool {
    read.saturating_add(incoming) > max
}

#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub url: String,
    pub status: u16,
    pub body: String,
    pub content_type: Option<String>,
    pub response_time_ms: u64,
    pub redirect_location: Option<String>,
    pub headers: Vec<(String, String)>,
}

impl Fetcher {
    pub fn new(user_agent: &str, timeout_seconds: u64) -> Self {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            // Vet addresses at connect time, not just at check time: without
            // this a rebinding DNS answer could hand the connector a private
            // address after `check_url_allowed` saw a public one.
            .dns_resolver(std::sync::Arc::new(crate::net_guard::GuardedResolver))
            .timeout(Duration::from_secs(timeout_seconds))
            .default_headers({
                let mut headers = HeaderMap::new();
                if let Ok(val) = HeaderValue::from_str(user_agent) {
                    headers.insert(USER_AGENT, val);
                }
                headers
            })
            .build()
            .expect("Failed to build HTTP client");

        Self { client }
    }

    pub async fn fetch(&self, url: &str) -> Result<FetchResult> {
        let start = Instant::now();

        // SSRF guard: block loopback/private/link-local/metadata targets before
        // any connection is opened. This chokepoint covers seeds, discovered
        // links, robots.txt, sitemaps, and every redirect hop.
        if let Err(msg) = crate::net_guard::check_url_allowed(url).await {
            anyhow::bail!("{msg}");
        }

        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch {}", url))?;

        let elapsed = start.elapsed().as_millis() as u64;
        let status = response.status().as_u16();

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let redirect_location = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
            .collect();

        // Reject on the advertised length before reading a single byte, then
        // stream with a running budget. Buffering the whole body first and
        // checking afterwards means an oversized response (or a compression
        // bomb, since the body arrives decompressed) is already resident in
        // memory by the time the limit is enforced.
        if let Some(advertised) = response.content_length() {
            if advertised > MAX_BODY_SIZE as u64 {
                anyhow::bail!(
                    "Response body from {} exceeds {} byte limit ({} bytes)",
                    url,
                    MAX_BODY_SIZE,
                    advertised
                );
            }
        }

        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("Failed to read body from {}", url))?
        {
            if exceeds_limit(bytes.len(), chunk.len(), MAX_BODY_SIZE) {
                anyhow::bail!(
                    "Response body from {} exceeds {} byte limit (aborted after {} bytes)",
                    url,
                    MAX_BODY_SIZE,
                    bytes.len() + chunk.len()
                );
            }
            bytes.extend_from_slice(&chunk);
        }

        let body = decode_body(&bytes, content_type.as_deref());

        debug!("Fetched {} — {} in {}ms", url, status, elapsed);

        Ok(FetchResult {
            url: url.to_string(),
            status,
            body,
            content_type,
            response_time_ms: elapsed,
            redirect_location,
            headers,
        })
    }

    /// Fetch with backoff on 429 and 503, honouring `Retry-After` when the
    /// server sends one.
    ///
    /// Ignoring that header meant a rate-limited host received three requests
    /// per URL on a fixed 1s/2s schedule — the opposite of polite, at exactly
    /// the moment it had asked to be spared.
    pub async fn fetch_with_retry(&self, url: &str, max_retries: u32) -> Result<FetchResult> {
        let mut delay_ms = 1000u64;

        for attempt in 0..=max_retries {
            let result = self.fetch(url).await?;

            if (result.status == 429 || result.status == 503) && attempt < max_retries {
                let wait = match retry_after_secs(&result.headers) {
                    Some(secs) if secs > MAX_RETRY_AFTER_SECS => {
                        warn!(
                            "Got {} for {} with Retry-After: {}s — longer than we will wait, \
                             giving up on this URL",
                            result.status, url, secs
                        );
                        return Ok(result);
                    }
                    Some(secs) => Duration::from_secs(secs),
                    None => {
                        let d = Duration::from_millis(delay_ms);
                        delay_ms *= 2;
                        d
                    }
                };
                warn!(
                    "Got {} for {}, retrying in {:?} (attempt {}/{})",
                    result.status,
                    url,
                    wait,
                    attempt + 1,
                    max_retries
                );
                tokio::time::sleep(wait).await;
                continue;
            }

            return Ok(result);
        }

        self.fetch(url).await
    }

    /// As [`Self::fetch_with_redirects`], but pacing every request through a
    /// shared [`RequestGate`] and re-checking each redirect destination with
    /// `allow_hop` before following it.
    ///
    /// Both matter for hops specifically: redirect requests previously bypassed
    /// the crawl delay entirely (two requests landed on the same timestamp), and
    /// robots.txt was only consulted for the queued URL, so a redirect could
    /// carry the crawler into a `Disallow`ed path whose content was then filed
    /// under the allowed URL.
    pub async fn fetch_with_redirects_gated<F>(
        &self,
        url: &str,
        max_hops: u32,
        gate: &RequestGate,
        allow_hop: F,
    ) -> Result<(FetchResult, Vec<crate::models::page::RedirectHop>)>
    where
        F: Fn(&str) -> bool,
    {
        let mut chain = Vec::new();
        let mut current_url = url.to_string();
        let mut last_redirect: Option<FetchResult> = None;
        let mut elapsed_ms: u64 = 0;

        for _ in 0..=max_hops {
            gate.acquire().await;
            let mut result = self.fetch_with_retry(&current_url, 2).await?;
            elapsed_ms += result.response_time_ms;
            result.response_time_ms = elapsed_ms;

            if result.status >= 300 && result.status < 400 {
                if let Some(location) = result.redirect_location.clone() {
                    let base = url::Url::parse(&current_url)
                        .with_context(|| format!("Invalid URL: {}", current_url))?;
                    let next = base
                        .join(&location)
                        .with_context(|| format!("Invalid redirect location: {}", location))?
                        .to_string();

                    if !allow_hop(&next) {
                        debug!("Not following {current_url} -> {next}: disallowed by robots.txt");
                        return Ok((result, chain));
                    }

                    chain.push(crate::models::page::RedirectHop {
                        status: result.status,
                        url: current_url.clone(),
                    });
                    current_url = next;
                    debug!("Following redirect {} -> {}", result.url, current_url);
                    last_redirect = Some(result);
                    continue;
                }
            }

            return Ok((result, chain));
        }

        warn!("Max redirect hops ({}) reached for {}", max_hops, url);
        match last_redirect {
            Some(mut result) => {
                result.response_time_ms = elapsed_ms;
                Ok((result, chain))
            }
            None => Ok((self.fetch_with_retry(&current_url, 2).await?, chain)),
        }
    }

    /// Fetch a URL following up to `max_hops` redirects, recording the chain.
    ///
    /// Each entry in the returned chain is one hop's *source*, so the full path
    /// is `chain[0].url -> chain[1].url -> ... -> result.url`, with the final
    /// element being the URL that actually served the response.
    pub async fn fetch_with_redirects(
        &self,
        url: &str,
        max_hops: u32,
    ) -> Result<(FetchResult, Vec<crate::models::page::RedirectHop>)> {
        let mut chain = Vec::new();
        let mut current_url = url.to_string();
        let mut last_redirect: Option<FetchResult> = None;
        // Time spent on the whole chain. Reporting only the final hop meant a
        // URL that took over a second through three redirects was recorded as
        // taking 0 ms, which then fed the average-response figure.
        let mut elapsed_ms: u64 = 0;

        // `max_hops` redirects need `max_hops + 1` requests: one per redirect
        // plus the final one that returns the document. Budgeting requests
        // instead of hops meant a documented cap of 5 followed only 4, and the
        // fifth-hop URL was handed back as an unresolved 3xx — which then
        // *scored better* than a page that resolved, since a 3xx carries no
        // content for the quality checks to fault.
        for _ in 0..=max_hops {
            let mut result = self.fetch_with_retry(&current_url, 2).await?;
            elapsed_ms += result.response_time_ms;
            result.response_time_ms = elapsed_ms;

            if result.status >= 300 && result.status < 400 {
                if let Some(location) = result.redirect_location.clone() {
                    chain.push(crate::models::page::RedirectHop {
                        status: result.status,
                        url: current_url.clone(),
                    });

                    // Resolve relative redirect URLs
                    let base = url::Url::parse(&current_url)
                        .with_context(|| format!("Invalid URL: {}", current_url))?;
                    current_url = base
                        .join(&location)
                        .with_context(|| format!("Invalid redirect location: {}", location))?
                        .to_string();

                    debug!("Following redirect {} -> {}", result.url, current_url);
                    last_redirect = Some(result);
                    continue;
                }
            }

            return Ok((result, chain));
        }

        warn!("Max redirect hops ({}) reached for {}", max_hops, url);

        // Hand back the last response already in hand. Issuing another request
        // here spent an extra hop beyond the cap and returned a response whose
        // hop was never recorded in the chain.
        match last_redirect {
            Some(mut result) => {
                result.response_time_ms = elapsed_ms;
                Ok((result, chain))
            }
            // Only reachable with max_hops == 0, where nothing has been fetched.
            None => Ok((self.fetch_with_retry(&current_url, 2).await?, chain)),
        }
    }
}

/// Serialises the start of every outbound request so a configured delay (or a
/// robots.txt `Crawl-delay`) describes the rate the *site* experiences, not the
/// rate each worker experiences independently.
pub struct RequestGate {
    delay: std::time::Duration,
    next_allowed: tokio::sync::Mutex<Option<std::time::Instant>>,
}

impl RequestGate {
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: std::time::Duration::from_millis(delay_ms),
            next_allowed: tokio::sync::Mutex::new(None),
        }
    }

    /// Wait until this crawl is allowed to issue its next request.
    ///
    /// The slot is claimed while the lock is held, so concurrent workers queue
    /// up one delay apart instead of all sleeping and then firing at once.
    pub async fn acquire(&self) {
        if self.delay.is_zero() {
            return;
        }
        let wait = {
            let mut next = self.next_allowed.lock().await;
            let now = std::time::Instant::now();
            let slot = match *next {
                Some(t) if t > now => t,
                _ => now,
            };
            *next = Some(slot + self.delay);
            slot.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Seconds requested by a `Retry-After` header, when given in delta-seconds
/// form. The HTTP-date form is legal but rare in rate-limiting responses and is
/// treated as absent.
fn retry_after_secs(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
}

/// Decode a response body to a `String`, honoring the character encoding. The
/// encoding is chosen from (1) the `charset` in the Content-Type header, then
/// (2) a `<meta charset>` / `<meta http-equiv>` declaration in the first ~1 KB,
/// then (3) UTF-8. Without this, non-UTF-8 pages (Windows-1252, Shift_JIS, …)
/// become U+FFFD soup, corrupting titles, hashes, and word counts.
fn decode_body(bytes: &[u8], content_type: Option<&str>) -> String {
    // 1. charset from Content-Type header. The parameter name is
    //    case-insensitive (RFC 9110 §5.6.6), so `Charset=` must work too —
    //    matching it case-sensitively mojibaked the whole document.
    let header_charset = content_type.and_then(|ct| {
        ct.split(';').skip(1).find_map(|p| {
            let p = p.trim();
            let (name, value) = p.split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['"', '\'']))
        })
    });
    if let Some(enc) = header_charset.and_then(|c| encoding_rs::Encoding::for_label(c.as_bytes())) {
        return decode_with(enc, bytes);
    }

    // 2. <meta charset> in the document prologue.
    let prologue = &bytes[..bytes.len().min(2048)];
    if let Some(enc) = sniff_meta_charset(prologue) {
        return decode_with(enc, bytes);
    }

    // 3. Default to UTF-8.
    decode_with(encoding_rs::UTF_8, bytes)
}

/// Decode with the chosen encoding, noting when the bytes did not actually fit
/// it. Silently returning U+FFFD soup meant a mis-declared page was analysed as
/// though its mojibake were real content, with nothing anywhere saying so.
fn decode_with(enc: &'static encoding_rs::Encoding, bytes: &[u8]) -> String {
    let (cow, used, had_errors) = enc.decode(bytes);
    if had_errors {
        warn!(
            "body did not decode cleanly as {} (used {}); some characters were replaced",
            enc.name(),
            used.name()
        );
    }
    cow.into_owned()
}

/// Look for a `<meta charset=...>` or `<meta http-equiv="content-type" ...>`
/// declaration in the document prologue.
///
/// Follows the WHATWG "prescan a byte stream to determine its encoding" rules
/// rather than searching for the first literal `charset` anywhere in the bytes.
/// That naive search took its label from a comment, a `<script charset>`
/// attribute, or a `?charset=` query string in an `href`, and silently decoded
/// the entire document with it.
fn sniff_meta_charset(prologue: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let text = String::from_utf8_lossy(prologue).to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }

        // A charset mentioned inside a comment is not a declaration.
        if text[i..].starts_with("<!--") {
            match text[i + 4..].find("-->") {
                Some(off) => {
                    i = i + 4 + off + 3;
                    continue;
                }
                None => break,
            }
        }

        // Only a <meta> element carries an encoding declaration.
        let is_meta = text[i..].starts_with("<meta")
            && bytes
                .get(i + 5)
                .is_some_and(|c| !c.is_ascii_alphanumeric() && *c != b'-');
        if is_meta {
            let end = text[i..].find('>').map(|o| i + o).unwrap_or(bytes.len());
            if let Some(enc) = charset_from_meta_tag(&text[i..end]) {
                return Some(enc);
            }
            // An unusable label is not fatal: a later <meta> may still declare a
            // valid encoding, so keep scanning.
            i = end.max(i + 1);
            continue;
        }

        i += 1;
    }

    None
}

/// Extract an encoding from a single lowercased `<meta ...>` tag.
fn charset_from_meta_tag(tag: &str) -> Option<&'static encoding_rs::Encoding> {
    if let Some(label) = attr_value(tag, "charset") {
        if let Some(enc) = resolve_meta_charset(&label) {
            return Some(enc);
        }
    }

    if attr_value(tag, "http-equiv")?.trim() == "content-type" {
        let content = attr_value(tag, "content")?;
        let pos = content.find("charset")?;
        let after = content[pos + "charset".len()..].trim_start_matches([' ', '=', '"', '\'']);
        let label: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        return resolve_meta_charset(&label);
    }

    None
}

/// Value of `name` in a lowercased tag, honouring quoted and bare forms.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(off) = tag[from..].find(name) {
        let at = from + off;
        // Must start an attribute, not sit inside a longer one.
        let boundary = at > 0 && tag.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + name.len()..].trim_start();
        if boundary {
            if let Some(v) = rest.strip_prefix('=') {
                let v = v.trim_start();
                let value: String = if let Some(q) = v.strip_prefix('"') {
                    q.chars().take_while(|c| *c != '"').collect()
                } else if let Some(q) = v.strip_prefix('\'') {
                    q.chars().take_while(|c| *c != '\'').collect()
                } else {
                    v.chars()
                        .take_while(|c| !c.is_ascii_whitespace() && *c != '>')
                        .collect()
                };
                return Some(value);
            }
        }
        from = at + name.len();
    }
    None
}

/// Map a meta-declared label to the encoding actually used.
///
/// WHATWG prescan: a UTF-16 label from a `<meta>` means UTF-8, because a real
/// UTF-16 document could not have contained that ASCII tag; `x-user-defined`
/// maps to windows-1252. Honouring `<meta charset="utf-16">` literally decoded
/// a perfectly good UTF-8 page into CJK noise and then reported its title,
/// description and H1 as missing.
fn resolve_meta_charset(label: &str) -> Option<&'static encoding_rs::Encoding> {
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    let enc = encoding_rs::Encoding::for_label(label.as_bytes())?;
    if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
        return Some(encoding_rs::UTF_8);
    }
    if enc == encoding_rs::X_USER_DEFINED {
        return Some(encoding_rs::WINDOWS_1252);
    }
    Some(enc)
}

#[cfg(test)]
mod gate_tests {
    use super::RequestGate;
    use std::time::Instant;

    /// The delay must describe the rate the *site* sees. Sleeping inside each
    /// worker let `concurrency` requests fire simultaneously every interval —
    /// under `Crawl-delay: 2` with 3 workers the host saw 3x the permitted rate
    /// in bursts, the exact pattern the directive exists to prevent.
    #[tokio::test]
    async fn concurrent_acquirers_are_spaced_not_batched() {
        let gate = std::sync::Arc::new(RequestGate::new(50));
        let start = Instant::now();

        let mut handles = Vec::new();
        for _ in 0..4 {
            let gate = gate.clone();
            handles.push(tokio::spawn(async move {
                gate.acquire().await;
                start.elapsed()
            }));
        }
        let mut times: Vec<_> = Vec::new();
        for h in handles {
            times.push(h.await.unwrap());
        }
        times.sort();

        // Four slots at 50ms apart: the last must not land in the first window.
        assert!(
            times[3].as_millis() >= 140,
            "4 requests at 50ms spacing should take >=150ms, took {:?}",
            times[3]
        );
    }

    #[tokio::test]
    async fn a_zero_delay_does_not_block() {
        let gate = RequestGate::new(0);
        let start = Instant::now();
        for _ in 0..10 {
            gate.acquire().await;
        }
        assert!(start.elapsed().as_millis() < 50);
    }
}

#[cfg(test)]
mod limit_tests {
    use super::{exceeds_limit, MAX_BODY_SIZE};

    #[test]
    fn allows_bytes_up_to_the_limit() {
        assert!(!exceeds_limit(0, 100, 1000));
        assert!(!exceeds_limit(900, 100, 1000));
    }

    #[test]
    fn rejects_the_chunk_that_crosses_the_limit() {
        assert!(exceeds_limit(900, 101, 1000));
        assert!(exceeds_limit(1000, 1, 1000));
    }

    /// A hostile Content-Length or chunk size must not wrap the addition and
    /// report "within budget".
    #[test]
    fn saturates_instead_of_overflowing() {
        assert!(exceeds_limit(usize::MAX, 1, MAX_BODY_SIZE));
        assert!(exceeds_limit(1, usize::MAX, MAX_BODY_SIZE));
    }
}

#[cfg(test)]
mod decode_tests {
    use super::{decode_body, sniff_meta_charset};

    #[test]
    fn decodes_windows_1252_from_header() {
        // 0x92 is a curly apostrophe in Windows-1252.
        let bytes = b"It\x92s";
        let out = decode_body(bytes, Some("text/html; charset=windows-1252"));
        assert_eq!(out, "It\u{2019}s");
    }

    #[test]
    fn sniffs_meta_charset() {
        let html = b"<html><head><meta charset=\"windows-1252\"></head>";
        assert_eq!(
            sniff_meta_charset(html).map(|e| e.name()),
            Some("windows-1252")
        );
    }

    /// The prescan must read `charset` only from a `<meta>` element. Taking the
    /// first literal "charset" anywhere let a comment, a `<script charset>`
    /// attribute, or a `?charset=` query string in an href decide how the whole
    /// document was decoded.
    #[test]
    fn charset_is_read_only_from_meta_elements() {
        let commented = b"<!-- we used to send charset=iso-8859-1 --><meta charset=\"utf-8\">";
        assert_eq!(
            sniff_meta_charset(commented).map(|e| e.name()),
            Some("UTF-8")
        );

        let in_href =
            b"<link rel=alternate href=\"/legacy?charset=shift_jis\"><meta charset=\"utf-8\">";
        assert_eq!(sniff_meta_charset(in_href).map(|e| e.name()), Some("UTF-8"));

        let other_tag = b"<script charset=\"shift_jis\"></script><meta charset=\"utf-8\">";
        assert_eq!(
            sniff_meta_charset(other_tag).map(|e| e.name()),
            Some("UTF-8")
        );
    }

    /// WHATWG prescan: a UTF-16 label in a meta means UTF-8, since a real UTF-16
    /// document could not have carried that ASCII tag. Honouring it literally
    /// decoded a good UTF-8 page into noise and reported its title as missing.
    #[test]
    fn meta_declared_utf16_is_treated_as_utf8() {
        let html = "<meta charset=\"utf-16\"><title>Caf\u{e9}</title>".as_bytes();
        assert_eq!(sniff_meta_charset(html).map(|e| e.name()), Some("UTF-8"));
    }

    /// An unusable label must not stop the scan — a later valid declaration wins.
    #[test]
    fn an_invalid_label_falls_through_to_a_later_meta() {
        let html = b"<meta charset=\"not-an-encoding\"><meta charset=\"shift_jis\">";
        assert_eq!(
            sniff_meta_charset(html).map(|e| e.name()),
            Some("Shift_JIS")
        );
    }

    /// Content-Type parameter names are case-insensitive (RFC 9110 §5.6.6).
    #[test]
    fn header_charset_parameter_is_case_insensitive() {
        let bytes = b"It\x92s";
        assert_eq!(
            decode_body(bytes, Some("text/html; Charset=windows-1252")),
            "It\u{2019}s"
        );
    }

    #[test]
    fn http_equiv_content_type_still_works() {
        let html =
            b"<meta http-equiv=\"content-type\" content=\"text/html; charset=windows-1252\">";
        assert_eq!(
            sniff_meta_charset(html).map(|e| e.name()),
            Some("windows-1252")
        );
    }

    #[test]
    fn defaults_to_utf8() {
        let bytes = "caf\u{e9}".as_bytes();
        assert_eq!(decode_body(bytes, Some("text/html")), "café");
    }
}
