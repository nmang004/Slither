# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Fixed

- **A page whose `<head>` contains invalid markup is no longer reported as having no title.**
  Anything the HTML spec disallows in the head — a tracking pixel `<img>`, a stray text node, a
  `<div>` from a mis-templated partial — closes the head early, and the `<title>` after it is parsed
  into `<body>`. Title selection was scoped to `head title`, so those pages were reported as
  **Missing Title** while their markup plainly carried one. Found on a real site, where three pages
  were flagged and all three had titles. The title is now found wherever the parser put it, inline
  SVG accessibility labels are still excluded, and the misplaced title is reported as what it
  actually is. This also activates `page_titles::outside_head`, a check that could never fire before:
  it requires a title to be present in order to report it as misplaced, and the scoping guaranteed
  there wasn't one.
- **`slither_summary` reported `affected_urls` as a bare count while `slither_query` uses that name for
  the list of affected URLs.** One field name with two types across two tools in the same server read
  as an empty list to a caller that had seen the other, sending it looking for the URLs in the
  per-page data instead. The summary field is now `affected_url_count`, matching the name
  `slither_query` already uses for the count beside its list.
- **A seed that redirects no longer collapses the crawl to one page.** Pages are recorded at the
  URL that finally served them, but the crawl scope was still established from the URL the user
  typed. Seeding `https://www.example.com/` on a site that canonicalises to the apex — or `http://`
  on a site that redirects to `https://` — classified every discovered link as external, and the
  audit ended after one page with a passing grade, exit 0, and no warning. The seed's own redirect
  is now resolved first and the destination is treated as the site under audit.
- The page a seed redirects to is no longer dropped from the report. It deduped against a visited
  key seeded from its own destination, which removed the homepage and left everything it links to
  reported as an orphan.
- **`<template>` contents are no longer counted as body text.** The markup inside a `<template>` is
  inert — the browser never renders it — so counting it inflated word counts (a page with five
  visible words reported 21), pushed soft-404 pages past the length guard that suppresses the
  check, and polluted both the duplicate-content hash and the readability score.
- **Links that redirect into a 404 are reported broken again.** Page identity moved to the URL that
  finally served a page, but the link joins still compared against the URL as written in the `href`,
  so a link to `/moved` that 301s to a missing page matched nothing and was dropped. On a site whose
  only bad link was of that shape, the critical "Broken Internal Links" issue did not appear at all —
  the worst possible outcome on a recently-migrated site, which is exactly when an audit is ordered.
- **Redirect destinations are no longer reported as orphan pages, and the link graph keeps those
  edges.** A page linked only through a redirect was reported as having no inbound links, and
  `slither link` printed the homepage as "(2 in / 0 out)" — inverted. The dropped edges corrupted
  `total_edges`, `avg_out_degree` and every PageRank value; on one fixture the graph went from
  2 edges and 2 components to 8 edges and 1.
- **The "3xx Redirects" list names the URL that redirects, not its destination.** It reported the
  destination — a URL returning 200 — as the redirect, so the guidance told the user to repoint links
  at the address that was already correct, while the URL that actually needed changing appeared in no
  issue and no CSV column. Redirect chains now also print their final destination instead of ending
  in a dangling arrow, and a 4xx/5xx reached through a redirect names the link that has to change.
- **The HTML report no longer prints impossible affected-URL counts.** It summed issue rows instead of
  counting distinct URLs, so a 10-page crawl claimed "14 URLs affected" — and at client scale a
  2,600-page crawl claimed 13,000. JSON, CSV, CLI and MCP were all correct; the HTML was the outlier.
- **Site-level findings are no longer hidden from the HTML.** Issues that legitimately have no URL list
  (no sitemap found, AI crawlers blocked) were filtered out, so the category rendered a green dot and
  "All checks passed — impressive!" while the severity bar beside it counted the issue. They now render
  with a `Site-wide` marker, in both the category panel and the Overview's top-issues list.
- **404, 500 and 403 pages are no longer labelled Indexable** in the CSV, the HTML badge, the structure
  detail panel and `slither inspect`, which printed "Indexable yes" directly above "Status 404". All
  four now use the same shared rule the sitemap generator uses, so the artifacts agree.
- **`slither sitemap` no longer writes a schema-invalid empty `<urlset>` and reports success.** It called
  the unguarded single-file helper, so a crawl with no pages array (`--summary-only`) or nothing
  indexable produced a file `xmllint --schema` rejects and Search Console reads as empty — while
  printing `✓ Wrote …` and exiting 0. It now uses the guarded generator, writes nothing when there
  is nothing to write, and exits non-zero saying which of the two causes applied. Crawls past the
  50,000-URL protocol cap now emit a `<sitemapindex>` plus chunk files instead of one oversized file.
- **URL fragments no longer become page identity.** `/docs/#toc` was recorded, and emitted into
  `<loc>`, as a distinct page. It also produced false findings on the fragment's own text: "URL
  contains spaces" for a `%20` after the `#`, a `url_length` longer than the URL, and "has
  parameters" on a page with no query string.
- **`googlebot-news` no longer suppresses general Search checks.** Google scopes that token to News,
  but a `<meta name="googlebot-news" content="noindex">` marked the page non-indexable site-wide:
  it dropped out of the generated sitemap and silently stopped being eligible for around twenty
  gated checks, so duplicate titles involving it simply stopped firing. Scoped directives are now
  kept but bucketed separately, matching how `X-Robots-Tag` was already handled.
- **`<meta name="robots" content="none">` now stops link discovery.** Google documents `none` as
  shorthand for `noindex, nofollow`, but only the literal `nofollow` token was matched, so the
  crawler kept following links the site had asked it not to.
- **`alt=""` is no longer reported as a missing alt attribute.** It is the spec-correct way to mark
  an image decorative, so flagging it told authors to undo the right thing and made screen-reader
  output worse if followed. Only a genuinely absent attribute now counts as missing, and the
  distinction survives into the JSON instead of being flattened to `null`.

- **A second Slither process no longer kills the first one's running jobs.** Startup recovery marked
  every in-flight job `failed` with "Server restarted during execution" while the crawl was still
  going, then blocked the real completion — so `completed_at` landed 25 seconds before the crawl
  actually finished, next to a fully populated result summary. Attaching an MCP client to the default
  `~/.slither`, which is the documented Claude Desktop setup, did this to a running REST crawl, and two
  concurrent Claude sessions did it to each other. Jobs now carry an owner and a heartbeat, and are
  reclaimed only when their owner is genuinely gone. **Trade-off:** a crashed process's jobs are
  reclaimed after 120–180 seconds rather than instantly at the next startup.
- **Crawl settings are no longer silently replaced by defaults.** `max_pages: 3` was honoured but
  `max_pages: 3.0` was discarded — and `3.0` is valid against the tool's own published
  `"type": "integer"` schema. The same applied to `delay_ms` and `concurrency`, so an operator who
  asked for 3 pages at one request per second could get up to 500 pages at four per second against a
  client's production site. Integral floats are now accepted and anything genuinely out of range is
  rejected with a message naming the parameter.
- **Unknown or misplaced arguments are rejected instead of ignored.** REST took crawl settings under
  `options`, so `{"type":"crawl","url":"…","max_pages":3}` returned 201 and crawled to the 500-page
  default; MCP behaved the same way for a misspelled `maxpages`. Both now refuse the call and name the
  offending argument alongside the accepted set, which is derived from each tool's own schema so it
  cannot drift.
- **Job types that cannot execute are refused.** `inspect`, `extract` and `screenshot` were accepted
  with a 201 and then sat `queued` forever; because REST and MCP share one queue cap, fifty of them
  wedged job creation on both transports until the process was restarted.
- **A rejected `webhook_url` no longer strands a queued job.** The job row and its output directory
  were committed before validation, and the error path returned before the caller ever received an id
  — so each retry of a mistyped webhook burned a queue slot permanently. Validation now happens before
  anything is written, and user errors return 4xx with the reason instead of a retryable 500.
- **MCP `resources/read` is bounded.** See the note above; it previously returned the whole artifact
  in a single line.
- **Webhooks fire on every terminal state.** A cancelled job delivered nothing at all on the REST path,
  and on the MCP path delivered `{"event":"job.completed","status":"cancelled"}` — burning the
  one-shot on an event that contradicted itself. `job.cancelled` and `job.queued` were accepted at
  registration but emitted by nothing, and a job reclaimed after a crash left its one-shot stranded
  forever.

### Security

- **`slither serve` silently ignored `SLITHER_API_KEY`.** The documented environment variable was
  never read by the CLI — only by the standalone `slither-server` binary — so
  `SLITHER_API_KEY=… slither serve` started a fully unauthenticated server while the operator
  believed it was protected. `--api-key` worked and masked the gap.

### Changed

- **Cloudflare is now opt-in.** It reaches a third-party API and needs credentials, so it is a
  feature flag like `playwright` rather than part of every build. The default build contains no
  Cloudflare code at all. Two modules were misplaced under it and are now where they belong:
  static single-page `inspect` (which needs no rendering service) and the security-header HEAD
  requests (which use the local fetcher). Both consequently work in the default build for the
  first time, and their tests now run there too. `--features cloudflare` restores `screenshot`,
  `extract`, and `inspect --rendered` / `--compare`.
- The CLI and server previously depended on `slither-core` without `default-features = false`, so
  the `cloudflare` feature was compiled in regardless of what was selected — a `--no-default-features`
  build still carried all of it while refusing to run the commands.

A second audit pass (see `AUDIT.md`) found two P0s and fourteen P1s; all are fixed.

### Security

- **robots.txt SSRF.** robots.txt was fetched by its own HTTP client that neither disabled
  redirects nor re-applied the SSRF guard per hop, so a site whose `/robots.txt` returned a 302
  to `http://169.254.169.254/…` pulled cloud metadata into `crawl.json` and the report. It now
  goes through the shared guarded `Fetcher`.
- **DNS-rebinding TOCTOU.** The guard resolved and vetted a host, then reqwest re-resolved it at
  connect time, so the address checked was never the address used. A `GuardedResolver` is now
  installed on the crawl and webhook clients, vetting the addresses the connector dials.
- The PageSpeed API key no longer reaches logs: it travels in the query string and reqwest's
  error `Display` includes the full URL, which was printed on any transport failure.
- Linux builds link rustls instead of system OpenSSL (the published artifact previously failed
  to start without a compatible `libssl.so.3`). CI actions are pinned by commit SHA,
  `contents: write` is scoped to the release job, and builds use `--locked`.

### Changed

- **MCP requests are now handled concurrently.** A slow `slither_inspect` or
  `slither_screenshot` previously blocked `ping` and every other call for the duration.
  Responses may now arrive out of request order (legal JSON-RPC — the `id` correlates);
  each response is still written as one complete newline-delimited line, stdout stays pure
  JSON-RPC, and shutdown drains outstanding handlers rather than dropping their replies.
- Each analyzer check is now capped at 60% of its category's severity allowance before the
  category cap applies, so one high-frequency check can no longer mask every other check in
  its category.
- **Health-score deductions are weighted by the share of the site affected.** "15 pages are
  missing a meta description" previously cost the same on a 15-page site (every page broken)
  as on a 1000-page one (1.5% broken). A proportional term now reaches the full allowance
  only when the whole site is affected, while a bounded absolute term keeps a single serious
  finding from rounding to nothing at scale — one broken page costs the same whether the
  crawl is 10 pages or 100,000. Site-level findings keep their flat weight.
- **`notifications/cancelled` now actually cancels.** In-flight work is aborted, the
  cancelled request emits no response (per spec), `initialize` is never cancellable, and a
  cancellation for an unknown or already-completed id is a silent no-op.

### Fixed

- **`--output <name>` without a `.json` suffix destroyed the crawl JSON** — the HTML report was
  written to the same path under default flags.
- **A cancelled job could resurrect itself**: the Running transition was unguarded, so cancelling
  a queued job was silently reversed and the full crawl ran anyway.
- **`--concurrency 0` hung forever** (zero-permit semaphore); it is now rejected, and configs
  arriving from REST/MCP are sanitized.
- The visited set is claimed atomically, so a URL discovered by two pages at once is no longer
  crawled twice and double-counted.
- robots.txt is fetched with the seed's scheme — an http-only host previously failed the fetch
  and fell through to disallow-all, yielding a silent 0-page crawl.
- Response bodies stream against a byte budget rather than being fully buffered before the 10 MB
  cap is checked; `fetch_with_redirects` no longer issues one request past its cap.
- **Analysis false positives:** duplicate/length checks are gated to indexable HTML, so noindex
  pages no longer generate "Duplicate Titles"/"Duplicate Descriptions"/"Duplicate H1" against
  indexable ones (a page excluded from the index cannot cause a duplicate-content problem); hreflang and canonical joins normalize both sides; only
  executable scripts count toward "Excessive JavaScript" (JSON-LD and `__NEXT_DATA__` no longer
  do); self-links no longer mask orphan pages; single-hop redirects are detected from the
  redirect chain; "No Sitemap Found" only fires when discovery actually ran; console errors are
  reported once; 401/403/429 are Warnings rather than Critical; `X-Robots-Tag` values such as
  `max-snippet:50` keep their value.
- The grade verdict is composition-aware — an F with zero critical issues no longer reads
  "Critical".
- **MCP/REST:** `page_data` paginates against its own total (a filter matching no issues reported
  `total_pages: 0` while pages existed); malformed JSON returns `-32700` instead of hanging the
  client; batches are handled; the transport size limit binds while reading; the queue cap
  applies to MCP; REST validates backend and pre-checks targets as MCP already did.
- Sitemap data is retained on `CrawlResult`, so the server executor's unconditional
  re-pipeline (and the CLI's PageSpeed re-run) no longer discards sitemap coverage analysis
  from every REST/MCP crawl.
- The job queue cap is enforced in a single transaction rather than as a count-then-insert
  race, and a refused job no longer leaves an output directory behind.
- The SSRF guard decodes IPv4 tunnelled inside IPv6 (6to4, Teredo, NAT64), closing a path
  where a literal like `2002:7f00:1::` reached loopback through a relay.
- Duplicate-content grouping excludes noindex pages, matching the title/description/H1 checks.
- **Report:** the per-page issue map is built once instead of rescanned for every page (quadratic
  on large crawls); the structure tree is depth-capped against stack overflow; a 0-page crawl no
  longer renders a fabricated all-green chart; `crawl_date` is a checked prefix, not a panicking
  byte slice.

## 0.3.0

This release consolidates the REST API, MCP server, alternate rendering backends, and
PageSpeed/CWV work that accumulated after 0.2.0, and includes a security and correctness audit
(see `AUDIT.md` and `ROADMAP.md`).

### Added

- **MCP server** (`slither serve --mcp`) — a Model Context Protocol server over stdio with seven
  tools: `slither_crawl`, `slither_status`, `slither_summary`, `slither_query`, `slither_compare`,
  `slither_inspect`, `slither_screenshot`. Negotiates the protocol version, advertises only
  implemented capabilities, and returns `structuredContent`.
- **REST API** (`slither serve`) — job creation, status, listing, deletion, result download, and
  webhook registration, backed by a SQLite job store.
- **Cloudflare Browser Rendering** integration (default `cloudflare` feature): `slither screenshot`,
  `slither extract` (AI structured extraction with an SEO preset), and `slither inspect` with
  static / rendered / compare modes.
- **Playwright backend** (opt-in `playwright` feature) — local Chrome rendering via chromiumoxide
  with `--chrome-path`, `--no-headless`, `--render-wait-ms`.
- **PageSpeed Insights / Core Web Vitals** enrichment (`--pagespeed`, `--pagespeed-key`,
  `--pagespeed-sample`, `--pagespeed-strategy`). LCP, CLS, FCP, TTFB from lab data; INP from CrUX
  field data. A Performance analyzer scores CWV thresholds (LCP 2.5/4.0s, INP 200/500ms, CLS
  0.1/0.25).
- **JavaScript analyzer** — render-aware checks (JS-injected title/description/canonical/H1/schema,
  render-blocking scripts, excessive scripts, console errors).
- Core Web Vitals in the HTML report, the CSV export, and the overview dashboard.
- **Internal link-graph analysis** in Rust (`slither-core::link_graph`): PageRank, orphan pages,
  navigation hubs, weakly connected components, and silo distribution — exposed as the
  `slither_link_graph` MCP tool and the `slither link <crawl.json>` CLI.
- **robots.txt / AI-crawler analyzer** (17th analyzer): reports which AI/LLM crawlers (GPTBot,
  Google-Extended, ClaudeBot, CCBot, PerplexityBot, Bytespider, Meta AI, …) the site blocks, and
  flags `noai`/`noimageai` opt-out directives.
- **MCP resources**: completed crawls expose `crawl.json` / `report.html` / `export.csv` as
  `slither://job/{id}/{file}` resources (`resources/list` + `resources/read`).
- **`slither sitemap <crawl.json>`**: generate a sitemap.xml from a crawl's indexable pages.
- Server-latency Performance check (signal without `--pagespeed`); charset-aware body decoding.
- **Dark mode in the HTML report** — a full dark palette that follows the OS via
  `prefers-color-scheme`, plus a sidebar toggle that persists the choice to `localStorage` and
  restores it before first paint (no flash). The inline SVG charts are theme-aware, and the report
  stays fully offline and self-contained.

### Changed

- All crates aligned to `0.3.0`; the CLI version and default User-Agent derive from the crate
  version.
- Repository URLs point at `github.com/nmang004/Slither`.
- `robots.txt` parsing switched to the `texting_robots` crate: full RFC 9309 support (`*`/`$`
  wildcards, longest-match precedence, group selection, BOM), and 5xx/429/transport errors now
  fail closed (disallow-all) instead of allowing everything.
- The MCP crawl tool rejects non-`local` backends (the server executor only runs local); use the
  CLI for JS-rendered crawls.
- `slither setup cloudflare` writes credentials to `$SLITHER_HOME/.env` at `0600` (not the working
  directory) and merges with existing keys.
- CLI logs now go to stderr at `WARN` by default, so the crawler's internal progress logs no longer
  interleave with the styled summary or corrupt piped stdout (e.g. JSON). `--verbose` still emits
  the full `DEBUG` trace.

### Fixed

**Security**

- Closed a stored XSS in the HTML report — crawled content (titles, meta descriptions, URLs) is
  now HTML-escaped, the embedded JSON island is script-context-safe, and the raw `format!` sites
  are escaped.
- Added an SSRF guard that blocks loopback/private/link-local/metadata targets for crawls and
  webhooks (opt out with `SLITHER_ALLOW_PRIVATE_TARGETS=1`); webhooks disable redirects and resolve
  DNS before delivery.
- API CORS is closed by default (allowlist via `SLITHER_CORS_ORIGINS`); the server refuses to start
  unauthenticated on a non-loopback host unless `SLITHER_ALLOW_ANONYMOUS=1`.
- Neutralized CSV formula injection in the export.
- The Cloudflare API token is marked sensitive and never printed via `Debug`.
- Updated `rustls-webpki`, `anyhow`, and `quick-xml` (0.37 → 0.41) to clear RUSTSEC advisories,
  including two sitemap-parsing DoS advisories.

**Correctness**

- Redirect-loop detection now requires a genuinely repeated URL in the chain (every ordinary
  `/x` → `/x/` redirect was previously flagged as a critical loop).
- Site-level issues now deduct points and appear in totals instead of scoring as zero.
- Self-referencing canonicals are no longer reported as an issue.
- Heading-sequence analysis uses the true document order (including H1).
- Title/meta/H1/H2/alt length checks count characters, not bytes.
- INP is read from CrUX field data and left unset when unmeasured (was always reported "Good").
- `slither inspect` no longer panics on non-ASCII meta descriptions.
- `link[rel~=…]` matching so multi-token `rel` values (e.g. `canonical shortlink`) are recognized;
  `meta[name=… i]` so capitalized `name="Description"` is found.
- Sitemap `<loc>` values in CDATA and with entity escapes (`&amp;`) are parsed correctly.
- `<base href>` is honored when resolving relative URLs; non-UTF-8 pages are decoded by charset
  instead of corrupted; relative canonicals and `Link: rel=canonical` headers are recognized.
- Presence checks (missing title/description/H1/canonical) no longer fire on non-HTML assets or
  error pages; link/orphan/sitemap-coverage joins compare normalized URLs.
- Hreflang validation is BCP-47-aware (accepts `zh-Hant`, `es-419`, `en-GB`; rejects `us-en`);
  `X-Robots-Tag` is treated as a noindex source everywhere; directive conflicts are only flagged
  for genuine contradictions.
- JSON-LD `@type` is extracted from `@graph`/arrays; JS render-blocking counts only head external
  scripts; PageSpeed enrichment gains timeouts, bounded concurrency, and quota-abort.
- Server/jobs hardening: SQLite `busy_timeout`/`synchronous`, conditional terminal status
  transitions, report-endpoint security headers, and a Playwright navigation timeout.
- `--max-pages` is now an exact cap. The budget was checked against a counter bumped only after
  each fetch finished, so up to `concurrency` extra tasks could spawn while requests were in flight
  (e.g. `--max-pages 12` returning 15 pages).
- The HTML report's Issues tab opens on the first category that actually has issues (critical →
  warning → info → clean) instead of an empty panel; summary counts are pluralized correctly
  ("1 issue", not "1 issues").

### Removed

- The unreachable `slither crawl --html` flag (only `--no-html` had any effect).
- The never-written `entities` SQLite table and the corresponding MCP `include` option.
- The Cloudflare whole-site crawl backend (`--backend cloudflare`) and its dead `/crawl` job
  client; Cloudflare now powers `screenshot`, `extract`, and `inspect --rendered` only.
- The shipped-but-dead `--pagespeed-local` flag and the `pagespeed/local.rs` /
  `playwright/screenshot.rs` stubs.
- The companion Python packages (`slither-entity`, `slither_link`, `slither-common`, `slither-seo`)
  and the `slither entity` command. Entity analysis is handled by Claude over MCP; the internal
  link-graph analysis (PageRank, orphans, hubs, components, silos) is reimplemented in Rust
  (`slither-core::link_graph`), exposed as the `slither_link_graph` MCP tool and a rewritten
  `slither link <crawl.json>` CLI. `slither setup` no longer installs a Python toolchain.

## 0.2.0

- REST API, MCP server, SQLite job management, and output organization (initial).

## 0.1.0

- Initial crawler, analyzers, scoring, and HTML/JSON/CSV reports.
