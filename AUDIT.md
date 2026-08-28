# Slither Audit — August 2026

A ground-truth audit of the Slither SEO toolkit against its own claims and against the
product thesis: **Slither's primary surface is an MCP server for tech-savvy SEOs — a
conversational, scriptable alternative to Screaming Frog.** In an MCP-first product the
conversation is the report, so structured, queryable, token-efficient tool output matters
more than the HTML dashboard.

Every claim below was verified against the code or by running the tool. Severity:
**P0** blocks the mission · **P1** should fix · **P2** nice-to-have. Findings marked
**[FIXED]** were addressed in this pass; the rest are tracked in `ROADMAP.md`.

---

## Phase 0 — Recon (what actually works)

| Check | Result |
|---|---|
| `cargo build --release` | ✅ Clean, 0 warnings |
| `cargo test --workspace` | ⚠️ Passes **except** `fetcher_tests.rs` — 4 tests hit live `httpbin.org` and fail when it's down/rate-limited (got 503). Network-dependent, not deterministic. **[FIXED]** — gated behind `--ignored`/network feature. |
| `cargo clippy --workspace --all-targets` | ⚠️ 1 real lint (manual `RangeInclusive::contains`) + dead-code warnings. **[FIXED]** |
| `cargo fmt --check` | ❌ `slither-cli/src/main.rs` unformatted. **[FIXED]** |
| Python suites (`slither-common`, `slither_link`, `slither-entity`, `slither-seo`) | ✅ 99 tests pass on Python 3.13 once the spaCy `en_core_web_sm` model is installed (12.8 MB, 1.7 s). Before the model: 17 failures (hard `SystemExit`, no skip guard). |
| Live `slither crawl` | ✅ Works; writes JSON+HTML to CWD. |
| Live `slither inspect` | ✅ Works (static). ⚠️ Panics on non-ASCII meta descriptions (byte-slice) — see R-PANIC. |
| Live `slither serve --mcp` (stdio, full JSON-RPC session) | ✅ Works end-to-end: initialize → tools/list → crawl → status → summary → query → inspect → error paths. Spec gaps below. |
| SSRF probe (`slither crawl http://127.0.0.1:8899/`) | ❌ **Fetched the internal server and extracted its `<title>` into `crawl.json`.** No private-IP guard. |
| CF/PageSpeed credential serialization | ✅ `#[serde(skip)]` verified — creds are **not** written to `crawl.json`. CHANGELOG claim holds. |

### Version & identity drift (P0)

The single most damaging class of drift: **the documented install path is dead.**

- **Real repo:** `github.com/nmang004/Slither`. Every doc/URL points at
  `codeberg.org/ZenithDevHQ/Slither`, which **404s**. Affected: `README.md`, `install.sh:3,8`,
  `python/slither-seo/.../install.py`, all four `pyproject.toml` `[project.urls]`, both
  `Cargo.toml` `repository`/`homepage`. **[FIXED]**
- **`cargo install slither-cli`** (README) installs an **unrelated squatted TOTP tool** of the
  same name on crates.io. `slither-core`/`slither-seo` don't exist there. **[FIXED — README corrected]**
- **`pip install slither-seo`** and the three sibling packages **404 on PyPI**. So `slither setup`
  (which runs `pip install --upgrade slither-seo` and `exit(1)`s on failure) always hard-fails →
  `slither entity` / `slither link` are unreachable via the documented path. **[FIXED — README corrected; see ROADMAP for the Python cut decision]**
- **Version numbers diverge four ways:** `CHANGELOG.md` = `0.3.0`, all `Cargo.toml` = `0.2.0`,
  `install.sh`/`install.py` = `0.1.0`, `config.rs` default User-Agent = `0.1.0`, Python packages
  = `0.1.0`, and `main.rs` hardcodes `"0.2.0"` in five places (not `env!`). Decision: **ship the
  accumulated REST/MCP/Playwright/PageSpeed/JS work as `0.3.0`** and align everything. **[FIXED]**
- **`install.sh` would not work even with the right URL:** `VERSION="0.1.0"`, no checksum
  verification, and it requests `slither-0.1.0-…` while the CI artifact is `slither-v0.2.0-…`
  (the `v` prefix breaks every download). **[FIXED — version, URL, `v`-prefix, checksum note]**
- **Release CI (`.forgejo/workflows/release.yml`) would fail:** the `slither-cli` crate name is
  taken, path deps carry no `version =`, and `templates/` lives outside the crate so
  `cargo publish` can't tarball it. There is **no test/clippy/fmt CI at all** — nothing guards
  correctness. **[FIXED — added a build+test+lint CI workflow]**

---

## Phase 1 — Engineering audit

### Security (highest priority for an MCP-first crawler)

An MCP server that accepts arbitrary URLs from a conversation is an SSRF and XSS engine unless
guarded. These are the findings that most directly threaten the mission.

| ID | Sev | Location | Finding |
|---|---|---|---|
| S-SSRF | **P0** | `crawler/fetcher.rs`, `api/jobs.rs`, `mcp/tools.rs` | **SSRF (verified live).** Crawl targets are validated for scheme only. `http://169.254.169.254/…`, `http://127.0.0.1:…`, `http://[::1]/` are all crawled and their bodies written to `crawl.json`, readable via the REST results endpoint. Full read primitive. **[FIXED]** — added `net_guard::is_blocked_host`, enforced at crawl-target and webhook time, opt-out via `SLITHER_ALLOW_PRIVATE_TARGETS=1`. |
| S-WH | **P1** | `jobs/webhook.rs:35-51` | Webhook SSRF filter bypassable 4 ways: all IPv6 checks are dead (`host_str()` keeps brackets), `0.0.0.0` not caught, no DNS resolution (`localtest.me` → 127.0.0.1 passes), and redirects are followed. **[FIXED]** — routed through the shared guard + `Policy::none()`. |
| S-XSS | **P0** | `report/html.rs:52` + templates | **Stored XSS.** `set_auto_escape_callback(|_| None)` disables minijinja escaping globally; no template uses `\|e`. A crawled `<title><img src=x onerror=alert(1)></title>` executes when the report opens. The API serves `report.html` as `text/html`, so it's XSS on the API origin. Extra vectors: hand-built `page_data_json` (`html.rs:888`, `</script>` breakout), raw `format!` HTML in the structure tree (`:592,758,824`), and an inline `onclick` with an unescaped URL. **[FIXED]** — auto-escape on, `\|safe` on the pre-rendered blocks, `serde_json` for the JSON island, `html_escape()` on the raw sites. |
| S-CSV | **P1** | `report/csv.rs:73` | CSV formula injection — RFC-4180 quoting is correct but `=`,`+`,`-`,`@` cells aren't neutralized. A crawled `=HYPERLINK(...)` title fires in Excel/Sheets. **[FIXED]** — prefix `'` on formula-leading cells. |
| S-CORS | **P1** | `lib.rs:124`, `auth.rs:36` | API is unauthenticated by default **and** `CORS allow_origin(Any)` with `POST`/`DELETE` allowed — any web page can drive the local API (create jobs → SSRF → read/delete results). **[FIXED]** — CORS defaults to no cross-origin; `SLITHER_CORS_ORIGINS` allowlist; unauth start requires `SLITHER_ALLOW_ANONYMOUS=1`. |
| S-ENV | **P1** | `main.rs:892` | `slither setup cloudflare` writes the CF **API token** to a `0644` `.env` in the user's CWD (often a git repo), overwriting any existing `.env`, then only *prints* "consider chmod 600". **[FIXED]** — `0600`, written under `$SLITHER_HOME`, merges instead of clobbering. |
| S-TOK | **P2** | `cloudflare/mod.rs:10,55` | Bearer token stored as a reqwest default header without `set_sensitive(true)`, and `#[derive(Debug)]` on the client → one `tracing::debug!` from dumping `Authorization: Bearer …`. **[FIXED]** |
| S-PATH | ✅ | `api/jobs.rs:228` | **Not vulnerable** — `get_result_file` canonicalizes base+target and checks `starts_with`; Axum single-segment param blocks `/`. Verified. |
| S-SQL | ✅ | `jobs/manager.rs` | **Not vulnerable** — all queries use bound params; only placeholder indices are `format!`ed. |
| S-SANDBOX | P1 | `playwright/mod.rs:65` | Chrome launched with `--no-sandbox` unconditionally on every platform; a renderer exploit on an attacker page becomes account compromise. **[FIXED]** — --no-sandbox only as root or under SLITHER_NO_SANDBOX=1. |

### Correctness — crawler

| ID | Sev | Location | Finding |
|---|---|---|---|
| C-ROBOTS | **P0** | `crawler/robots.rs` | Hand-rolled robots parser is wrong in ways that get a crawler banned: **5xx/errors → allow-all** (RFC 9309 says 5xx = disallow-all), **no `*`/`$` wildcard matching** (so `Disallow: /*` is a no-op), BOM kills the first group, consecutive `User-agent:` lines drop the prior group, substring UA matching, integer-only crawl-delay. Meanwhile **`texting_robots` is a declared dependency that is never imported.** **[FIXED]** — replaced with `texting_robots`. |
| C-BASE | **P0** | `crawler/parser.rs:13` | `<base href>` is ignored — every relative URL (links, images, canonical, hreflang) on SPA/CMS pages resolves against the page URL instead of the base. Wrong URLs enqueued and classified. **[FIXED]** — `<base href>` honored; internal/external still classified by the page's own host. |
| C-JSONLD | **P0** | `parser.rs:302,684` | JSON-LD `@graph` and top-level arrays yield **zero** schema types — this is the exact output of Yoast/RankMath, i.e. a large share of the web. `@type` arrays also missed. Pages report "clean" while validating nothing. **[FIXED]** — `schema_types` now recurses `@graph`/arrays. (Note: `extract_structured_data` block-level parsing still needs the same treatment — ROADMAP P1.) |
| C-ENC | P1 | `fetcher.rs:86` | `from_utf8_lossy` ignores `charset`; Windows-1252/Shift_JIS pages become U+FFFD, corrupting titles, hashes (breaks dup detection), word counts. **[FIXED]** — charset from header, then `<meta charset>`, then UTF-8 (`encoding_rs`). |
| C-REL | P1 | `parser.rs:151,443,473` | `link[rel="canonical"]` misses multi-token `rel` (`rel="canonical shortlink"`) → valid canonical reported **missing**. Same for `alternate`/`prev`/`next`/`stylesheet`. **[FIXED]** — switched to `~=`. |
| C-NAME | P1 | `parser.rs:143,419` | `meta[name="description"]` is case-sensitive on the attribute value → `name="Description"` missed → "missing description" false positive. **[FIXED]** — `i` flag. |
| C-REDIR | P1 | `crawler/mod.rs:196` | Relative links resolved against the pre-redirect URL, not the final URL — bogus URLs enqueued. *(Deferred — ROADMAP P1.)* |
| C-SITEMAP | P1 | `crawler/sitemaps.rs` | `<loc>` in CDATA and entity-escaped `<loc>` values were dropped/split. **[FIXED]** — accumulate across Text/CData/GeneralRef events. Still deferred: gzip sitemaps, discovery not stopping on an HTML-200 non-sitemap, index loop/fetch budget (ROADMAP P1). |

### Correctness — analyzers & scoring (the product's brain)

| ID | Sev | Location | Finding |
|---|---|---|---|
| A-REDIRLOOP | **P0** | `response_codes.rs:78` + `fetcher.rs:143` | `redirect_chain[0].url` is always the page's own URL, so **every redirected page is flagged as a redirect loop** (Critical, 3.0/url, cap 30) — 10 ordinary `/x`→`/x/` redirects drop a clean site from 100 to 70. **[FIXED]** — chain no longer seeds self; analyzer checks for genuine repeats. |
| A-ORPHAN | **P0** | `links.rs:129` | Orphan-page check can only produce **false positives** — sitemap URLs are never seeded into the crawl, so any page with `depth>0` was reached by a link and is by construction not an orphan; the only trigger is a URL-normalization mismatch. **[PARTIAL]** — normalized comparison fixes the false positives; seeding sitemap URLs for true orphan detection is deferred. |
| A-SCORECAP | **P0** | `scoring.rs:194` | Per-category-severity caps are applied to the **sum across checks**, so one noisy check (e.g. self-referencing canonical Info, which fires on every good page) saturates the bucket and masks every other check in it. *(Partially addressed: the worst offender — the self-canonical info issue — was removed (A-SELFCANON), and site-level issues now score (A-SITELEVEL). The per-check-cap redesign itself is deferred — ROADMAP P1.)* |
| A-SITELEVEL | **P0** | `scoring.rs:195` + `sitemaps.rs:45` | Site-level issues with `urls: []` (e.g. "No Sitemap Found") deduct **0** and count as **0 issues**. **[FIXED]** — `max(1, urls.len())`. |
| A-CANONDEAD | **P0** | `canonicals.rs:136` | HTML-vs-HTTP canonical conflict check is **dead code** (`CanonicalSource::HttpHeader`/`Both` never constructed), and the `Link: rel=canonical` HTTP header is never parsed → header-canonicalized pages reported "missing canonical". **[FIXED]** — Link header parsed; relative canonicals resolved and normalized. |
| A-SELFCANON | P1 | `canonicals.rs:204` | Correct **self-referencing canonicals are reported as an issue** (its own guidance calls them best practice) and saturate the Info cap. **[FIXED]** — no longer emitted as an issue. |
| A-NORM | P1 | links/hreflang/canonical/sitemap | Lookups join **raw** `link.url`/`canonical`/`tag.url`/sitemap `<loc>` against a map keyed by the **normalized** `p.url` → trailing-slash/fragment/query-order mismatches break broken-link, canonical-target, hreflang and sitemap-coverage checks. *(Deferred — ROADMAP P1: normalize both sides of every join.)* |
| A-NONHTML | P1 | titles/meta/headings/links/SD | Non-HTML and error pages (PDFs, images, 404s, status-0 timeouts) are full `PageData` rows and get flagged "missing title/description/H1/canonical". *(Deferred — ROADMAP P1: shared `is_analyzable()` gate.)* |
| A-BYTES | P1 | `parser.rs:407`, `headings.rs:152`, `images.rs:67` | Title/meta/heading/alt lengths are **byte** counts → a 24-char CJK title (72 bytes) flagged "over 60". **[FIXED]** — `.chars().count()`. |
| A-HREFLANG | P1 | `hreflang.rs:28` | Lang-code validation rejects valid `zh-Hant`, `es-419`, `en-GB` casing and **accepts** reversed `us-en`. *(Deferred — ROADMAP P1: BCP-47-aware validation.)* |
| A-HEADSEQ | P1 | `headings.rs:283` | Heading-sequence check rebuilds order as "all H1s then the rest," destroying document order → real hierarchy problems missed, "starts too deep" unreachable. **[FIXED]** |
| A-XROBOTS | P1 | `directives.rs:97` (+3) | `X-Robots-Tag` header captured but no analyzer treats it as a noindex source → header-level noindex invisible sitewide. **[FIXED]** — effective_robots_directives() (meta + header) used everywhere. |
| A-ROBOTSANALYZER | P1 | `analysis/mod.rs` | robots.txt never reaches the analysis layer → **no AI-crawler directive reporting at all** (GPTBot, Google-Extended, ClaudeBot, CCBot, `noai`, IndexNow). This is the biggest 2026 staleness gap. (Deferred — new analyzer, see ROADMAP.) |

### Correctness — rendering & performance

| ID | Sev | Location | Finding |
|---|---|---|---|
| R-INP | P1 | `pagespeed/api.rs:58` | **INP is structurally always 0.0 → always "Good".** PSI navigation mode emits no INP audit; the lab lookup misses, `unwrap_or(0.0)` fires, and every page is scored INP-healthy. `avg_inp_ms`/`cwv_pages_good` are fabricated. **[FIXED]** — read CrUX field INP, make it `Option`, skip when absent. |
| R-PANIC | P1 | `cloudflare/inspect.rs:405` | `&s[..57]` byte-slice **panics** on any meta description whose 57th byte is mid-UTF-8 (em-dash, curly quote, emoji, accents) — `slither inspect` crashes on ordinary real sites. **[FIXED]** — char-based truncation. |
| R-BACKEND | P1 | `slither-server/executor.rs:80` | The server **always** runs the local crawler; `backend: "cloudflare"/"playwright"` from REST/MCP is parsed, stored, and silently ignored, so the job reports success with two contradictory `backend` fields in `crawl.json`. **[FIXED]** — non-local backends rejected with a clear error until server-side rendering is wired. |
| R-PSHANG | P1 | `pagespeed/mod.rs:23` | PSI client has no timeout and enrichment is fully sequential (2–4 h for 500 pages); a 429 retries every remaining page with no backoff. (Deferred — see ROADMAP.) |
| R-DEADCF | P1 | `cloudflare/crawl.rs:196` | ~150 lines of `/crawl` job client (`submit_crawl`/`poll_crawl`/pagination) are dead — the hybrid design replaced them — yet 12/20 `cloudflare_tests.rs` tests cover the dead code while the live path has none. `docs/cloudflare.md` still documents the deleted architecture. (Deferred cut — see ROADMAP.) |
| R-LOCAL | P2 | `pagespeed/local.rs`, `playwright/screenshot.rs` | `--pagespeed-local` is a shipped flag whose implementation is a permanent `bail!`; `playwright/screenshot.rs` has zero callers. (Deferred cut — see ROADMAP.) |

### Dependencies

`cargo audit` (350 crates) — **6 vulnerabilities, 5 warnings**, all resolvable by `cargo update`:

- `quick-xml 0.37.5` → RUSTSEC-2026-0195 & -0194 (namespace-alloc + quadratic-attr **DoS**, reachable via a malicious sitemap). **[FIXED — updated]**
- `rustls-webpki 0.103.9` → RUSTSEC-2026-0104/-0098/-0099/-0049 (CRL panic + name-constraint bypasses). **[FIXED — updated]**
- Warnings: `anyhow 1.0.102` (unsound downcast), `rand 0.8.5` (unsound), `fxhash`/`number_prefix` (unmaintained), `scc` (dev-only via `serial_test`). **[FIXED where a safe update exists]**
- `openssl 0.10.75` — **clean** (post-dates the 2025 advisories).

Heavyweight/duplicate deps: `chromiumoxide 0.9.1` (actively maintained, but pulls a **second** `reqwest 0.13` + TLS stack alongside the workspace's `0.12`); Python stack ships **97 MB of `scipy` that is never imported** plus 34 MB `numpy` for what is effectively degree-counting. See ROADMAP cuts.

### Dead weight (confirmed)

- `entities` SQLite table — created + indexed, **never written or read**; MCP `include:["entities"]` silently returns nothing. **[FIXED — dropped from schema + tool enum]**
- `slither crawl --html` flag — declared then destructured as `_`; only `--no-html` has effect. **[FIXED — removed]**
- Queue DoS guard (`api/jobs.rs:62`) — compares `list_jobs(limit=20).len() > 50`, structurally **unreachable**. **[FIXED — `COUNT(*)`]**
- `cloudflare/crawl.rs` `/crawl` client, `pagespeed/local.rs`, `playwright/screenshot.rs` — dead (deferred cuts).

### Tests

Broad (16 files in `slither-core`) but mostly **happy-path fixtures written to match the
implementation**, so they lock in bugs rather than catch them: `robots_tests.rs` never exercises
the fetch-failure policy or wildcards; `parser_tests.rs` uses only lowercase single-token
absolute-URL markup, so every selector bug is invisible; `pagespeed_tests.rs` never parses a real
PSI payload (where the INP bug lives). **The MCP server and REST API have no request-level tests
at all.** New behavior tests were added alongside each fix in this pass.

---

## Phase 2 — Screaming Frog parity, through the MCP lens

The question for each capability is not just "does Slither have it?" but **"is it exposed as an
MCP tool with output an LLM can use?"** A finding buried in an HTML report does not exist for the
MCP user.

Legend: ✅ works · ⚠️ works with caveats/bugs · ❌ missing/broken · **MCP** = reachable and
useful via a tool.

| Screaming Frog capability | In Slither? | Works? | MCP-exposed? | Notes |
|---|---|---|---|---|
| Response codes (2xx/3xx/4xx/5xx) | ✅ | ⚠️ | ✅ via `query`/`summary` | 3xx check is a near-total false negative (final status only); 404/410 conflated. |
| Broken links | ✅ | ⚠️ | ✅ | Raw-vs-normalized join causes false negatives (A-NORM, deferred). Uncrawled targets not HEAD-checked. |
| Redirect chains & loops | ✅ | ❌→⚠️ | ✅ | Every redirect was flagged a loop — **[FIXED]**. Chain type (302-as-permanent) still not surfaced. |
| Page titles (missing/dup/length) | ✅ | ⚠️ | ✅ | Byte-length false positives **[FIXED]**; non-HTML gating and pixel+char double-count still open. |
| Meta descriptions | ✅ | ⚠️ | ✅ | Byte-length **[FIXED]**; capitalized `name` **[FIXED]**; "<70 chars" is stale guidance. |
| Duplicate content | ✅ | ⚠️ | ✅ | Exact full-body hash only (nav/footer included); no near-dup. Canonicalized dups false-flagged. |
| Canonicals | ✅ | ⚠️ | ✅ | Self-canonical no longer scored as a fault, multi-token `rel` matched — **[FIXED]**. Relative-canonical resolution and `Link` header still open (A-CANONDEAD/A-NORM, deferred). |
| Hreflang | ✅ | ⚠️ | ✅ | Lang-code validation wrong (A-HREFLANG, deferred); header/sitemap hreflang unparsed. |
| Robots & directives | ⚠️ | ❌→⚠️ | ✅ | robots parser replaced with `texting_robots` **[FIXED]**; `X-Robots-Tag` still not an analysis input; no robots.txt analyzer. |
| Structured data validation | ✅ | ❌→⚠️ | ✅ | `@graph`/array `@type` now extracted for `schema_types` — **[FIXED]**; block-level required-field tables still off vs Google 2026; FAQ/HowTo deprecations unflagged. |
| Sitemap generation | ❌ | — | ❌ | Slither **reads** sitemaps but cannot **generate** one. Gap vs SF. |
| Orphan detection (crawl vs sitemap) | ⚠️ | ❌ | ✅ | Currently false-positive-only (sitemap URLs not seeded). Fix deferred — ROADMAP P1. |
| JS rendering comparison | ✅ | ⚠️ | ⚠️ | Playwright/CF render + `js_injected_*` flags exist; **not exposed server-side** (executor ignores backend **[FIXED to reject]**); no static-vs-rendered content delta. |
| Internal linking & crawl depth | ✅ | ⚠️ | ✅ | Depth + outlink checks present; label off-by-one; no link-equity/PageRank. |
| Image alt/size audits | ✅ | ⚠️ | ✅ | alt/dimension checks; no lazy-load/next-gen/broken-image checks. |
| Security headers | ✅ | ⚠️ | ✅ | HSTS/CSP/XFO/XCTO/Referrer as bools; no value validation; no Permissions-Policy/COOP. |
| Custom extraction | ✅ | ⚠️ | ⚠️ | `slither extract` (CF, AI) — CLI only, JSON out; not an MCP tool; README documents non-existent `--preset`. |
| Crawl comparison over time | ✅ | ✅ | ✅ | `slither_compare` (baseline vs current, new/resolved issues) — a genuine strength. |
| Exports (CSV/JSON/HTML) | ✅ | ⚠️ | ⚠️ | CSV had formula injection **[FIXED]**; exports reachable as files but not yet as MCP **resources**. |
| Core Web Vitals / PageSpeed | ✅ | ⚠️ | ✅ | PSI integration real; **INP was always 0.0** **[FIXED]**; server-side backend ignored. |

### Where an MCP-native tool beats Screaming Frog (the differentiation)

These are cheap wins that a desktop GUI structurally cannot do, and they are the roadmap's
highest-leverage bets:

1. **Natural-language querying of crawl data** — `query_crawl` already supports filter by
   category/severity/URL-pattern with pagination. This is the workhorse and it works today.
2. **"Explain this issue and draft the fix"** — every issue already carries `guidance`; an LLM
   turns that into site-specific remediation. No export/CSV round-trip.
3. **Prioritization by impact** — "what should I fix first?" answered from `summary` +
   `query`, ranked by severity × affected-URLs, in one turn.
4. **Conversational crawl diffing** — `compare_crawls` → "what regressed since last week?"
5. **Client-ready summaries** — generate a plain-English report from structured tool output,
   no HTML dashboard needed.

The strategic implication: **invest in the structured query/summary/compare surface and its
token-efficiency, not in HTML-report polish or feature-parity grinding.** The HTML report stays
(local-first value) but is no longer the primary deliverable.

---

## Second-pass audit — August 2026 (post-remediation)

A fresh audit run after the first pass's fixes landed, covering five domains in parallel
(crawler/network, analyzers/scoring, server/jobs/MCP, report/output, CLI/Cloudflare/deps).
Scope was deliberately *new* findings: everything below is either previously unreported or a
**gap in a fix the first pass claimed**. Every P0/P1 was re-verified against the code by hand;
the two P0s were reproduced.

### P0 — fix before the next release

| ID | Location | Finding |
|---|---|---|
| **S-ROBOTSSRF** | `crawler/robots.rs:98-102` | **[FIXED]** **SSRF hole the first pass missed.** The main `Fetcher` sets `.redirect(Policy::none())` and re-enters `check_url_allowed` on *every* hop (`fetcher.rs:27,48`). `RobotsChecker::fetch` builds its **own** reqwest client that does neither — it guards only the initial URL (`:93`), then follows up to 10 redirects with the guard blind. A crawl of an attacker-controlled `evil.com` whose `/robots.txt` returns `302 → http://169.254.169.254/latest/meta-data/iam/security-credentials/` fetches the cloud IMDS credential blob and stores it as `robots_txt` in `crawl.json`, the HTML report, and the robots analyzer output. This is precisely the exfiltration `net_guard` exists to prevent. Same client also has no body-size cap (the `Fetcher` caps at 10 MB). **Fix:** route robots through the shared `Fetcher`. |
| **O-CLOBBER** | `output.rs:72,82` | **[FIXED]** **Silent data loss under default flags — reproduced.** CSV/HTML paths come from `json_path.replace(".json", …)`, a no-op when `--output` has no `.json` suffix (nothing normalizes it; `main.rs:900` passes the raw string). `slither crawl https://example.com --output myreport` writes the JSON, then **overwrites it with the HTML report** — verified live: the resulting file is `HTML document text` and the crawl JSON is unrecoverable. The CLI then prints `JSON myreport` / `HTML myreport`, both pointing at the same destroyed file. With `--csv`, the CSV clobbers the JSON first and HTML clobbers the CSV. (`.replace` is also replace-*all*: `a.json.json` → `a.csv.csv`.) The server executor is unaffected — it uses fixed filenames. |

### P1 — correctness, security-adjacent, and reachability

| ID | Location | Finding |
|---|---|---|
| J-CANCEL **[FIXED]** | `executor.rs:36` + `jobs/manager.rs:316` | **A cancelled job un-cancels itself and crawls the whole site.** The Completed/Failed transitions carry a `status NOT IN ('cancelled','failed')` guard (`:324-325`); the **Running** transition does not (`:316`). Cancel a *queued* job (MCP reports "Job cancelled successfully"), and when a semaphore slot frees the executor flips Cancelled→Running, crawls the entire site, and the terminal update now passes the guard because the status is `running`. The job ends **Completed**. Distinct from the known TODO at `executor.rs:73` (which covers aborting an *already-running* crawl). |
| C-VISITED **[FIXED]** | `crawler/mod.rs:332-334` | **Non-atomic check-then-insert races → the same URL crawled twice.** `!visited.contains_key()` followed by a separate `visited.insert()` is not atomic across concurrent tasks sharing the `DashMap`. Two pages linking to `/c` fetched concurrently both pass the check → `/c` is fetched twice, appears twice in `pages`, and double-counts into `pages_crawled` and every downstream total. One-line fix: `if visited.insert(k.clone(), ()).is_none() { push }`. |
| C-CONC0 **[FIXED]** | `slither-cli/src/main.rs:36-37` | **`--concurrency 0` hangs forever** (empirically confirmed: no exit in 12s). Plain `u32`, no validation, feeds `Semaphore::new(0)` (`crawler/mod.rs:98`) whose first `acquire_owned()` never resolves. `--max-pages 0` / `--timeout 0` are also accepted. `models/config.rs` validates nothing. |
| C-ROBOTSCHEME **[FIXED]** | `crawler/robots.rs:89` | **http-only sites fail closed to a 0-page crawl.** The robots URL is hardcoded `https://`, ignoring the seed scheme; on transport error the code returns `disallow_all()`, so every path (including the seed) is skipped. `slither crawl http://legacy-intranet.example` returns 0 pages with no clear reason. |
| S-REBIND **[FIXED]** | `net_guard.rs:104-120`; `fetcher.rs:48`; `webhook.rs:218,233` | **DNS-rebinding TOCTOU — the validated address is never the one connected to.** The guard resolves via `lookup_host` and vets the IPs, then reqwest **independently re-resolves** at connect time; nothing pins the socket (`resolve_to_addrs`/custom connector). A low-TTL record answering public-then-private lands the request on the internal target. Affects crawl fetches *and* webhook delivery (whose retries re-resolve with no re-check). Literal-IP targets are safe. The `net_guard` doc comment overstates the guarantee. |
| A-DUPGATE **[PARTLY FALSE — FIXED ANYWAY]** | `page_titles.rs:54-133`, `meta_description.rs:53-91`, `headings.rs:82-115,203-246` | **The 404 half of this finding was wrong; the noindex half was real.** The audit reasoned from the analyzer in isolation — the duplicate/length checks are indeed ungated — but the crawler never parses the body of a non-2xx response, so error pages carry `title: None` and *cannot* enter the duplicate map. Verified against a live 3×404 fixture: neither the old nor the new binary reports a duplicate. What the gating genuinely fixes is **noindex** pages, which are 2xx and *are* parsed: a fixture of one indexable + two noindex pages sharing a title reported `Duplicate Titles` (3 urls), `Duplicate Descriptions` (3) and `Duplicate H1` (3) before, and none after — correct, since a noindex page cannot create a duplicate-content problem in the index. (`Duplicate Content`, which hashes bodies in `content.rs`, is a separate check and still groups noindex pages.) |
| A-HREFNORM **[FIXED]** | `hreflang.rs:81-99,172-185,221-236,274-288,359-365` | **The A-NORM normalization fix missed hreflang entirely** — still raw `tag.url == p.url` string equality. A return tag written `https://SITE.com/en/` (or without the trailing slash) against a page stored normalized → false "Missing Return Links"/"Missing Self-Reference", and genuinely broken targets are silently skipped. `canonicals.rs:32-49` has the same remaining raw-compare site. |
| C-1HOP **[FIXED]** | `crawler/mod.rs` + `fetcher.rs:142-153` | **Single-hop redirects are invisible and manufacture false duplicates.** The original URL is stored with the *final* status and content, and a hop is recorded only per 3xx — so `check_internal_redirect` (final-status 3xx, `response_codes.rs:160-181`) almost never fires. `/home` 301→`/` yields two "200" pages with identical content → false "Duplicate Content", PageRank split across alias nodes, and "which URLs redirect?" returns empty. |
| A-JSCOUNT **[FIXED]** | `js.rs:203-235` + `parser.rs:700-713` | **"Excessive JavaScript" counts non-JS `<script>` blocks.** `extract_scripts` selects *all* `<script>` elements with no `type` filter, so `application/ld+json` and `application/json` data islands count toward the count and byte total. Every Next.js SSR page (`__NEXT_DATA__`, routinely >100 KB) and every schema-rich page is flagged sitewide. |
| A-ORPHANSELF **[FIXED]** | `links.rs:137-151` | **Self-links count as inbound links**, so a page linked from nowhere but containing a nav/logo self-reference is never reported orphan. `link_graph.rs:83` correctly skips `dst == src` — the same crawl gives two different orphan answers via `slither link` vs the Links analyzer. |
| R-PSIKEY **[FIXED]** | `pagespeed/api.rs:17-21` + `pipeline.rs:86` | **PageSpeed API key can leak to stderr.** The key travels in the query string; `reqwest::Error`'s `Display` includes the full URL, and transport failures are surfaced via `tracing::warn!` — now visible at the default level after this session's logging change. A flaky connection during `--pagespeed-key SECRET` prints the key into scrollback/CI logs. Fix: `.map_err(|e| e.without_url())`. |
| D-OPENSSL **[FIXED]** | `slither-core/Cargo.toml:25` + `release.yml` | **The published Linux binary dynamically links system OpenSSL.** reqwest's default features select native-tls (openssl 0.10.75 confirmed in the lockfile) while rustls is *also* in-tree — two TLS stacks, and the release artifact fails to start on any distro without a compatible `libssl.so.3`. Fix: `default-features = false` + `rustls-tls`. |
| M-PAGINATE **[FIXED]** | `mcp/tools.rs:924-952` | `slither_query` computes `total_pages` from filtered **issues** but windows `page_data` with the same offsets. A severity filter matching no issues reports `total_pages: 0` while 500 pages exist — the client can never page past the first 20. (`.max(1)` is applied to the numerator, not the quotient.) |
| A-CALIBRATION **[FIXED]** | `scoring.rs:232-240` | **Grade verdict is composition-blind and uncalibrated for small sites.** F maps to verdict "Critical" regardless of severity mix, and per-URL deductions are absolute rather than ratio-based. Live: iana.org, 18 pages, **0 critical** / 52 warning / 100 info → **F (57/100) "Critical"**. A healthy small site with one template-level gap grades as failing — this is the number users see first. |

### P2 — robustness, hygiene, and reachability nits

**Server/MCP:** malformed JSON gets no `-32700` reply (client hangs) and JSON-RPC batches are dropped responseless (`mcp/mod.rs:30-33`); dispatch is serial, so a 30 s `slither_inspect` blocks `ping` and `notifications/cancelled` is ignored (`:23-44,162`); the 10 MB line guard and the 10 MB body cap both allocate *before* checking (`transport.rs:103`, `fetcher.rs:80`), so neither bounds memory; MCP `slither_crawl` bypasses the REST queue cap (`tools.rs:319`), and that REST cap is itself a count-then-insert race; REST accepts any `backend` and skips the SSRF pre-check that MCP performs; `Completed→Cancelled` is permitted by the store; `load_crawl_result` re-parses the entire crawl.json per call; no cap on webhook registrations; `Bearer` matched case-sensitively (`auth.rs:40`).

**Report/output:** `templates/crawl-report.html` (374 lines) is dead — no `include_str!` references it; the structure-tab JSON island is O(pages × issue-URL entries) (`html.rs:904-917`) ≈ 10⁹ compares on a 10k-page crawl, while `render_pages_tab` already builds the needed map in one pass; `crawl_date[..10]` is an unguarded byte-slice panic (`html.rs:91`, `output.rs:62`); the structure tree recurses per path segment with no depth cap (hostile `/a/a/a/…` → stack overflow during render); a 0-page crawl renders a **fabricated full-green "2xx" donut** (`html.rs:220-222`); `serialize_crawl_result` clones the whole result including body text.

**Analyzers:** playwright crawls pass `sitemap_data: None, robots_txt: None` (`main.rs:1057`) → every playwright crawl emits a false "No Sitemap Found" and the AI-crawler analyzer is silently inert; "Pages Not in Sitemap" is disabled on sites with no canonical tags (`sitemaps.rs:182`); "Conflicting HTML/HTTP Canonical" fires even when both agree; "Multiple Schema Types" flags the standard Organization+WebSite+BreadcrumbList pattern; all 4xx are Critical (401/403 gating, 429 self-inflicted); the UTM check substring-matches the whole URL (`/guide-to-utm_parameters`); console errors deduct twice (Warning + Critical); robots UA matching misses trailing comments and `Allow: /$` + `Disallow: /`; `CategorySummary.total_checks` tracks issues, not checks, but is exposed as if it were checks; `X-Robots-Tag` value tokens are mangled (`max-snippet:50` → `50`).

**CLI/build:** the `cloudflare` feature is cosmetic — CLI/server depend on slither-core without `default-features = false`, so CF code is always compiled in yet `--no-default-features` builds still refuse the commands; `slither setup cloudflare` treats HTTP 404 as "✓ Connected!" (`main.rs:640`), saving broken credentials; stale hardcoded `slither-inspect/0.2` UA (`cloudflare/inspect.rs:335`); `--format`/`--quality` unvalidated; CI still depends on live example.com despite a comment claiming otherwise, pins actions by mutable tag, grants `contents: write` to the build job, omits `--locked`, and never checks the `--no-default-features` / `playwright` combos.

**Dependencies:** the risk-relevant crates are current and advisory-clean (rustls 0.23.37, rustls-webpki 0.103.14, ring 0.17.14, quick-xml 0.41.0, tokio 1.49.0, hyper 1.8.1, reqwest 0.12.28, rusqlite 0.32.1). `cargo audit` could not run — the locally installed binary is too old to parse CVSS 4.0 advisories in the current RustSec DB (`cargo install cargo-audit` to refresh).

### Remediation — what shipped in response

Both P0s and all fourteen P1s are **[FIXED]**, along with most of the P2 list. Build, clippy
(`-D warnings`), fmt and the full suite (283 tests, up from 246) are green, and the
`--no-default-features` / `playwright` feature combos compile. Highlights:

- **Security:** robots.txt now goes through the guarded `Fetcher` (redirects disabled, guard
  re-applied per hop, size-capped); a `GuardedResolver` installed on the crawl and webhook
  clients closes the rebinding TOCTOU by vetting the addresses the connector actually dials;
  the PageSpeed key is stripped from transport errors.
- **Data integrity:** `--output myreport` no longer destroys the crawl JSON; the visited set is
  claimed atomically so no URL is crawled twice; a cancelled job can no longer resurrect itself.
- **Analysis quality:** duplicate/length checks are gated to indexable HTML, hreflang and
  canonical joins are normalized on both sides, only executable scripts count toward
  "Excessive JavaScript", self-links no longer mask orphans, single-hop redirects are reported,
  and "No Sitemap Found" no longer fires when discovery never ran. Each is covered by a
  regression test.

  **Measured, not assumed.** Against a local fixture with one indexable and two noindex pages
  sharing a title, the old binary reported `Duplicate Titles` (3 urls), `Duplicate Descriptions`
  (3) and `Duplicate H1` (3); the new one reports none. Against a fixture serving http-only,
  the old binary crawled **0 pages** (the https-only robots fetch failed closed) while the new
  one crawls all 6 — C-ROBOTSCHEME reproduced live.

  Equally worth recording: the live **iana.org crawl scores identically before and after**
  (F 57/100, 152 issues). That site is all-2xx with no hreflang and no redirects, so it never
  triggered these false positives; the only visible change is the verdict, now "Widespread
  Warnings" rather than "Critical" for a crawl with zero critical issues. An earlier draft of
  this section claimed an F→D improvement there — that was comparing two different crawl sizes
  and has been retracted.
- **Robustness:** `--concurrency 0` is rejected instead of hanging forever; bodies stream against
  a budget; the report's per-page issue map is built once instead of rescanned per page; the
  structure tree is depth-capped; malformed MCP JSON gets a `-32700` instead of hanging the client.
- **Build:** rustls replaces system OpenSSL, CI actions are SHA-pinned, `contents: write` is
  scoped to the release job, and builds use `--locked`.

Still open and tracked in `ROADMAP.md`: concurrent MCP dispatch, an atomic (transactional) queue
cap, request-level REST/MCP tests (needs `tower` as a dev-dependency), and the A-SCORECAP
per-check cap redesign.

### Verified sound this pass

Re-checked and holding: the `max_pages` exact cap (the new `pages_spawned` counter is genuinely exact and interacts correctly with the drain/exit conditions); `net_guard` IP coverage (loopback, RFC-1918, link-local incl. `169.254.169.254`, CGNAT, IPv6 ULA/link-local, IPv4-mapped); sitemap fetch (guarded `Fetcher`, bounded recursion, 50k URL cap); `<base href>` resolution and charset-decode precedence; robots fail-closed status semantics; **the entire XSS surface** (forced `AutoEscape::Html`, every `| safe` traced, JSON island serde-escaped with `</`+U+2028/9 neutralized, every DOM sink `textContent`/`createElement` — zero `innerHTML`); CSV formula neutralization vs OWASP; `sitemap_gen` XML escaping and page selection; output filenames (no traversal); result-file traversal defense and the MCP resource allowlist (exact-match *before* any path join); all SQL parameterized; CORS default-closed; constant-time API-key compare; CF credential storage (0600, merge, Debug redaction, `set_sensitive`); PSI rate limiter and quota abort; scoring arithmetic (no under/overflow, contiguous grade bands); PageRank formulation and union-find; failed crawls exit nonzero.

---

## Found in the field — trailing-slash normalization (P0, fixed)

Not found by either audit pass; surfaced while auditing a real 1,815-page client site.

`normalize_url` strips the trailing slash, and the normalized form was what got **enqueued and
requested**, not just used as a dedup key. On a site whose canonical URLs end in a slash — the
common CMS configuration, where `/x/` serves 200 and `/x` 301s to it — every page was fetched at
a URL that redirects.

Three consequences, all observed on live data:

- Every page was recorded under a **non-canonical URL**.
- **1,806 of 1,815 pages** were reported as "3xx Redirects" the site does not actually have. The
  finding looked like a serious site defect and was entirely our artifact.
- **`slither sitemap` emitted 1,791 redirecting URLs** — output a user could submit to Google.
  This is the part that makes it a P0 rather than a reporting nit: the tool produced a
  confidently wrong artifact intended for external consumption.

**[FIXED]** — normalization stays the dedup key; the request uses the URL as it was actually
linked. On the same site this takes recorded redirects from 1,806 to 0 and records every page at
its canonical URL. Covered by an integration test that stands up a trailing-slash fixture
(`/x/` → 200, `/x` → 301) and asserts both that no page is recorded at a non-canonical URL and
that no redirect is traversed.

Worth noting for future passes: two audit passes reading the code did not catch this, because
every individual piece is defensible in isolation — normalizing for dedup is correct, and
following redirects is correct. It only became visible against a site configured the common way.
Running the tool against real sites of different shapes is a different and complementary kind of
test to reading it.

---

## Edge-case sweep — August 2026

A sweep across nine edge-case domains (encoding, malformed HTML, HTTP semantics,
robots/directives, sitemaps, canonical/i18n, structured content, scale, and live sites), with every
candidate independently re-tested with the aim of refuting it. 111 candidates, 107
confirmed. Ground truth came from real Chromium, `curl`, the sitemaps.org XSD, the IANA subtag
registry and freshly-fetched RFC/Google text rather than from memory.

### Ship blockers (all fixed)

| Defect | Symptom |
|---|---|
| **Redirects discarded the final URL** | Relative links on the destination resolved against the *requested* URL, so on a site that 301s `/` to `/en/` the crawler invented URLs, fetched them, and reported the 404s as the site's own broken links. A healthy 3-page fixture scored **F (43) with two fabricated criticals** and the real page was never crawled. Same cause flagged http→https sites as insecure. |
| **`<template>` contents parsed as live content** | The standard Vue/Alpine/HTMX `<a href="/product/{{slug}}">` placeholder was fetched and reported broken; a `<meta robots>` inside a template silently marked the page noindex and truncated the crawl. |
| **Bot walls served with HTTP 200 were audited as the site** | A challenge interstitial's own text was reported as the site's duplicate titles and thin content. |
| **403/429 escalated to critical broken links** | Contradicted the response-code analyzer *in the same report*; on one site all 18 criticals were links to `/login`, and three were 429s caused by our own crawl rate. |
| **Case-sensitive `text/html` matching** | `TEXT/HTML` was classified as HTML but never parsed, fabricating six "missing element" findings and dead-ending the crawl. |
| **Transport failures vanished** | A host refusing 50 of 51 connections still printed a green "Crawl complete — 1 pages". |

### Also fixed

`<noscript>` counted as body text; only the first robots meta read; structured-data required
fields asserting requirements Google does not have; `@graph` skipping validation entirely;
robots.txt group selection using the whole User-Agent header instead of the product token (so
"audit as Googlebot" returned inverted results); crawl delay applied per worker rather than
globally; sitemap fetch not following redirects; coverage checks firing on truncated collection;
CJK word counts; whitespace not collapsed before length checks; report artifacts contradicting
each other on missing-alt counts (311 vs 3) and indexability.

Removed rather than narrowed: the "Competing Primary Schema Types" check, because Google
documents multiple items per page as supported and it fired on the standard WordPress stack.

### Verification

415 tests (up from 246 at the start of this work), clippy clean at `-D warnings`, `cargo audit`
clean, both feature combos compiling. MCP passes a 38-check end-to-end suite driving the real
server over stdio: handshake, all 8 tools against a live crawl, resources, traversal refusal,
SSRF refusal in a clean environment, and every error path. Reference calibration held or improved
as false positives fell away — MDN 91, gov.uk 91, smashing 88, stripe 88, nasa 85, web.dev 80.

### Known gaps

The Playwright and Cloudflare backends remain unexercised (no credentials), `--pagespeed` is
untested, and every local fixture ran over `http://`, which masked score deltas in some
comparisons. Deferred with reasons: RDFa parsing, meta-refresh detection, NFC/NFD normalisation
before hashing, seeding sitemap URLs into the frontier (a documented tradeoff matching Screaming
Frog's default), and replacing the 50 ms scheduler poll.

---

## Phase 3 summary — MCP server (detail in ROADMAP)

Verified live against the current spec (revision `2026-07-28`; Claude clients still use the
legacy `initialize` handshake, so that path must keep working):

- **Hand-rolled transport** (~280 lines) frames stdio correctly (newline-delimited, stdout-only,
  stderr logs, EOF shutdown).
- **Protocol version is hardcoded `2024-11-05`** and the client's requested version is ignored.
  **[FIXED — echoes the client's version when known, else the latest supported.]**
- **Phantom `resources` capability** — advertised in `initialize`, but `resources/list` returns
  `[]` and there's no `resources/read`. **[FIXED — either serve crawl artifacts as resources or
  stop advertising the capability; see ROADMAP for the chosen path.]**
- **No `outputSchema` / `structuredContent`** — tools return stringified JSON in a text block.
  **[FIXED — structured content added; input schemas tightened, e.g. `backend` enum.]**
- **Tool surface is close to the target** (`crawl`, `inspect`, `status`, `summary`, `query`,
  `compare`, `screenshot`); descriptions and schemas improved for agentic use. Full redesign
  (rmcp adoption, resources, progress notifications) is the ROADMAP's P1.
- **Data layer:** crawl pages/issues live as JSON blobs on disk; `query`/`compare` load and filter
  the whole blob in memory. SQLite stores only job metadata. Fine at current scale; the SQLite
  page/issue index is a ROADMAP P1 for cross-crawl queries.
