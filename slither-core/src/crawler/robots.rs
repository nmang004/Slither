use anyhow::Result;
use texting_robots::Robot;
use tracing::{debug, warn};

/// Checks URLs against robots.txt rules using the `texting_robots` crate, which
/// implements the Robots Exclusion Protocol (RFC 9309): `*`/`$` wildcards,
/// longest-match precedence, group selection by user-agent product token, BOM
/// handling, and crawl-delay.
pub struct RobotsChecker {
    robot: Option<Robot>,
    /// Set when the fetch failed in a way that RFC 9309 says means "disallow
    /// everything" (5xx / 429 / network error).
    force_disallow_all: bool,
    sitemap_urls: Vec<String>,
    crawl_delay_secs: Option<u64>,
    rules_count: usize,
    /// The raw robots.txt content, so the analysis layer can inspect
    /// AI-crawler policy (GPTBot, Google-Extended, etc.).
    raw: String,
}

/// Google stops reading robots.txt after 500 KiB and ignores everything beyond.
///
/// Without this bound a rule sitting several megabytes in still removed URLs
/// from the audit — understating the crawlable site against a rule no search
/// engine would ever read — and the whole file was copied verbatim into the
/// crawl JSON, making a 2-page crawl a 6.5 MB artifact that was 97% robots.txt.
const MAX_ROBOTS_BYTES: usize = 500 * 1024;

/// Pick the robots.txt group that applies to our User-Agent.
///
/// RFC 9309 §2.2.1 matches on the *product token*, not the whole header: a
/// crawler identifying as `Mozilla/5.0 (compatible; Googlebot/2.1; +http://...)`
/// belongs to the `User-agent: Googlebot` group. `texting_robots` compares the
/// string it is given against each declared group verbatim, so handing it the
/// full header matched nothing and silently fell through to the wildcard group —
/// the crawler then fetched exactly the paths a site had forbidden it, and
/// skipped the ones meant for everyone else.
///
/// Returns the declared token to match on, or the original string when no
/// declared group applies (which correctly lands on `*`).
fn select_group_token<'a>(content: &str, user_agent: &'a str) -> std::borrow::Cow<'a, str> {
    // Product tokens in our UA: the part before any '/', split on the
    // punctuation that separates products and comments.
    let ua_tokens: Vec<String> = user_agent
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '(' | ')' | ',' | '+'))
        .filter_map(|part| part.split('/').next())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect();

    for line in content.lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("user-agent") {
            continue;
        }
        // Strip an inline comment, which RFC 9309 permits.
        let declared = value.split('#').next().unwrap_or("").trim();
        if declared.is_empty() || declared == "*" {
            continue;
        }
        if ua_tokens
            .iter()
            .any(|t| t == &declared.to_ascii_lowercase())
        {
            return std::borrow::Cow::Owned(declared.to_string());
        }
    }

    std::borrow::Cow::Borrowed(user_agent)
}

impl RobotsChecker {
    /// Parse robots.txt content for the given user agent.
    pub fn from_str(content: &str, user_agent: &str) -> Self {
        // Truncate on a character boundary so the slice stays valid UTF-8.
        let content = if content.len() > MAX_ROBOTS_BYTES {
            let mut end = MAX_ROBOTS_BYTES;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            warn!(
                "robots.txt is {} bytes; only the first {} are read, matching Google",
                content.len(),
                end
            );
            &content[..end]
        } else {
            content
        };

        let rules_count = content
            .lines()
            .map(str::trim)
            .filter(|l| {
                let l = l.to_ascii_lowercase();
                l.starts_with("disallow:") || l.starts_with("allow:")
            })
            .count();

        let group_token = select_group_token(content, user_agent);

        let (robot, sitemap_urls, crawl_delay_secs) =
            match Robot::new(&group_token, content.as_bytes()) {
                Ok(robot) => {
                    let sitemaps = robot.sitemaps.clone();
                    let delay = robot.delay.map(|d| d.ceil() as u64);
                    (Some(robot), sitemaps, delay)
                }
                Err(e) => {
                    warn!("Failed to parse robots.txt: {e}; allowing all paths");
                    (None, Vec::new(), None)
                }
            };

        debug!(
            "Parsed robots.txt: {} rules, crawl-delay: {:?}, {} sitemaps",
            rules_count,
            crawl_delay_secs,
            sitemap_urls.len()
        );

        Self {
            robot,
            force_disallow_all: false,
            sitemap_urls,
            crawl_delay_secs,
            rules_count,
            raw: content.to_string(),
        }
    }

    /// A checker that disallows every path — used when robots.txt could not be
    /// fetched due to a server error (RFC 9309 §2.3.1.4).
    fn disallow_all() -> Self {
        Self {
            robot: None,
            force_disallow_all: true,
            sitemap_urls: Vec::new(),
            crawl_delay_secs: None,
            rules_count: 0,
            raw: String::new(),
        }
    }

    /// The raw robots.txt content (empty when none was fetched).
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Fetch and parse robots.txt for a domain.
    ///
    /// Status handling per RFC 9309:
    /// - 2xx → parse the rules.
    /// - 4xx (except 429) → treat as "no robots.txt", allow everything.
    /// - 5xx / 429 / transport error → disallow everything (fail closed).
    ///
    /// `seed_url` supplies the scheme and authority: an http-only host must be
    /// asked over http, or the fetch fails and we would fail closed on a site
    /// that is perfectly crawlable.
    pub async fn fetch(seed_url: &str, user_agent: &str, timeout_seconds: u64) -> Result<Self> {
        let robots_url = match Self::robots_url_for(seed_url) {
            Some(u) => u,
            None => {
                warn!("Could not derive robots.txt URL from {seed_url}; allowing all paths");
                return Ok(Self::from_str("", user_agent));
            }
        };

        // Go through the shared Fetcher rather than a bare reqwest client: it
        // disables automatic redirects and re-applies the SSRF guard on every
        // hop, so a robots.txt that 302s to the cloud metadata endpoint cannot
        // pull an internal response into the crawl output. It also enforces the
        // response-size cap.
        let fetcher = crate::crawler::fetcher::Fetcher::new(user_agent, timeout_seconds);

        match fetcher.fetch_with_redirects(&robots_url, 5).await {
            Ok((response, _chain)) => {
                let status = response.status;
                if (200..300).contains(&status) {
                    Ok(Self::from_str(&response.body, user_agent))
                } else if status == 429 || (500..600).contains(&status) {
                    warn!("robots.txt returned {status}; disallowing all paths (fail closed)");
                    Ok(Self::disallow_all())
                } else {
                    warn!("robots.txt returned {status}; allowing all paths");
                    Ok(Self::from_str("", user_agent))
                }
            }
            Err(e) => {
                warn!("Failed to fetch robots.txt: {e}; disallowing all paths (fail closed)");
                Ok(Self::disallow_all())
            }
        }
    }

    /// Build the robots.txt URL for a seed, preserving its scheme, host and
    /// port. Returns `None` if the seed has no host.
    fn robots_url_for(seed_url: &str) -> Option<String> {
        let parsed = url::Url::parse(seed_url).ok()?;
        let host = parsed.host_str()?;
        let scheme = parsed.scheme();
        Some(match parsed.port() {
            Some(port) => format!("{scheme}://{host}:{port}/robots.txt"),
            None => format!("{scheme}://{host}/robots.txt"),
        })
    }

    /// Check if a path (optionally with query string) is allowed.
    pub fn is_allowed(&self, path: &str) -> bool {
        if self.force_disallow_all {
            return false;
        }
        match &self.robot {
            Some(robot) => robot.allowed(path),
            None => true,
        }
    }

    pub fn sitemap_urls(&self) -> &[String] {
        &self.sitemap_urls
    }

    pub fn crawl_delay(&self) -> Option<u64> {
        self.crawl_delay_secs
    }

    pub fn rules_count(&self) -> usize {
        self.rules_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_disallow_is_honored() {
        let r = RobotsChecker::from_str(
            "User-agent: *\nDisallow: /*.pdf$\nDisallow: /private/\n",
            "Slither",
        );
        assert!(!r.is_allowed("/files/report.pdf"));
        assert!(!r.is_allowed("/private/x"));
        assert!(r.is_allowed("/public/page"));
    }

    #[test]
    fn disallow_all_blocks_everything() {
        let r = RobotsChecker::disallow_all();
        assert!(!r.is_allowed("/"));
        assert!(!r.is_allowed("/anything"));
    }

    #[test]
    fn bom_prefixed_group_still_applies() {
        let r = RobotsChecker::from_str("\u{feff}User-agent: *\nDisallow: /admin\n", "Slither");
        assert!(!r.is_allowed("/admin"));
        assert!(r.is_allowed("/home"));
    }

    #[test]
    fn empty_allows_all() {
        let r = RobotsChecker::from_str("", "Slither");
        assert!(r.is_allowed("/anything"));
    }

    #[test]
    fn sitemaps_and_crawl_delay_extracted() {
        let r = RobotsChecker::from_str(
            "User-agent: *\nCrawl-delay: 2\nSitemap: https://ex.com/sitemap.xml\n",
            "Slither",
        );
        assert_eq!(r.crawl_delay(), Some(2));
        assert_eq!(r.sitemap_urls(), ["https://ex.com/sitemap.xml"]);
    }
}

#[cfg(test)]
mod url_tests {
    use super::RobotsChecker;

    /// Regression: the robots URL was hardcoded to https, so an http-only host
    /// failed the fetch and fell through to disallow-all — a 0-page crawl.
    #[test]
    fn robots_url_preserves_the_seed_scheme() {
        assert_eq!(
            RobotsChecker::robots_url_for("http://legacy.example/some/page"),
            Some("http://legacy.example/robots.txt".to_string())
        );
        assert_eq!(
            RobotsChecker::robots_url_for("https://example.com/"),
            Some("https://example.com/robots.txt".to_string())
        );
    }

    #[test]
    fn robots_url_preserves_a_non_default_port() {
        assert_eq!(
            RobotsChecker::robots_url_for("http://example.com:8080/x"),
            Some("http://example.com:8080/robots.txt".to_string())
        );
    }

    #[test]
    fn robots_url_is_none_without_a_host() {
        assert_eq!(RobotsChecker::robots_url_for("not a url"), None);
    }
}

#[cfg(test)]
mod group_selection_tests {
    use super::{select_group_token, RobotsChecker, MAX_ROBOTS_BYTES};

    const ROBOTS: &str =
        "User-agent: Googlebot\nDisallow: /nogoogle/\n\nUser-agent: *\nDisallow: /noeveryone/\n";

    /// RFC 9309 §2.2.1 matches the product token, not the whole header. Passing
    /// the full User-Agent matched no group, so the crawler silently obeyed the
    /// wildcard group — fetching exactly the paths addressed to it and skipping
    /// the ones meant for everyone else.
    #[test]
    fn a_full_user_agent_header_still_selects_its_group() {
        let ua = "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
        assert_eq!(select_group_token(ROBOTS, ua), "Googlebot");

        let checker = RobotsChecker::from_str(ROBOTS, ua);
        assert!(!checker.is_allowed("/nogoogle/x"), "own group must apply");
        assert!(
            checker.is_allowed("/noeveryone/x"),
            "wildcard group must not apply once a specific group matched"
        );
    }

    /// Slither's own default UA against a group addressed to it.
    #[test]
    fn the_default_user_agent_matches_a_slither_group() {
        let robots =
            "User-agent: Slither\nDisallow: /noslither/\n\nUser-agent: *\nDisallow: /other/\n";
        let checker = RobotsChecker::from_str(robots, "Slither/0.3.0 (SEO audit tool)");
        assert!(!checker.is_allowed("/noslither/x"));
        assert!(checker.is_allowed("/other/x"));
    }

    /// A UA with no matching group falls through to the wildcard.
    #[test]
    fn an_unmatched_agent_uses_the_wildcard_group() {
        let checker = RobotsChecker::from_str(ROBOTS, "SomeOtherBot/1.0");
        assert!(!checker.is_allowed("/noeveryone/x"));
        assert!(checker.is_allowed("/nogoogle/x"));
    }

    /// Partial tokens must not match: a `User-agent: bot` group does not apply
    /// to a crawler calling itself `Robot`.
    #[test]
    fn matching_is_on_whole_tokens() {
        let robots = "User-agent: bot\nDisallow: /x/\n";
        assert_eq!(select_group_token(robots, "Robot/1.0"), "Robot/1.0");
    }

    /// Google stops reading at 500 KiB; a rule beyond that must not remove URLs
    /// from the audit, and the oversized file must not be copied into the crawl
    /// artifact verbatim.
    #[test]
    fn robots_is_truncated_at_the_google_limit() {
        let mut content = String::from("User-agent: *\n");
        while content.len() < MAX_ROBOTS_BYTES + 5_000 {
            content.push_str("Disallow: /junk-path-that-pads-the-file/\n");
        }
        content.push_str("Disallow: /beyond-the-limit/\n");

        let checker = RobotsChecker::from_str(&content, "Slither");
        assert!(
            checker.is_allowed("/beyond-the-limit/x"),
            "a rule past 500 KiB is not read by search engines"
        );
        assert!(checker.raw().len() <= MAX_ROBOTS_BYTES);
    }
}
