# Slither Roadmap

Derived from `AUDIT.md`. Organized as: **what shipped in this pass**, then the prioritized
backlog (P0 → P1 → P2), the **MCP redesign** plan, and **decision items** that need the owner's
sign-off before working features are removed.

The north star: **Slither is an MCP server for tech-savvy SEOs.** Every item is weighed by how
much it improves the conversational, structured-output surface — not the HTML dashboard.

---

## Shipped in this pass (P0s + high-value P1s)

**Build / hygiene**
- `rustfmt` normalized; `clippy -D warnings` clean; dead-code removed.
- Network-dependent `fetcher_tests` gated behind `--ignored` so `cargo test` is deterministic.
- Added CI (`test.yml`): `cargo test --workspace` + `clippy` + `fmt --check` on push/PR.

**Version & docs truth**
- All crates → **0.3.0**; `main.rs` version strings use `env!("CARGO_PKG_VERSION")`; default
  User-Agent → `Slither/0.3.0`.
- All `codeberg.org/ZenithDevHQ` → `github.com/nmang004/Slither`.
- `install.sh`: correct version, `v`-prefixed artifact name, checksum verification, temp-file download.
- `README.md` rewritten for accuracy (real flags, real install path, Playwright/PageSpeed/CWV/JS
  documented, scoring table regenerated, 16 analyzers, GitHub URLs).
- `CHANGELOG.md` 0.3.0 rewritten to cover everything actually shipped (REST/MCP/SQLite/Playwright/
  PageSpeed/CWV/JS), not just Cloudflare.

**Security**
- SSRF guard (`net_guard`) blocking loopback/private/link-local/ULA/metadata targets, enforced at
  crawl-target and webhook dispatch; opt-out `SLITHER_ALLOW_PRIVATE_TARGETS=1`.
- Webhook filter rewritten (IPv6 brackets, `0.0.0.0`, DNS resolution, redirect `Policy::none()`).
- HTML-report XSS closed (auto-escape on, `|safe` allowlist, `serde_json` JSON island, escaped raw
  HTML sites).
- CSV formula-injection neutralized.
- API CORS locked down (`SLITHER_CORS_ORIGINS` allowlist; anonymous start requires
  `SLITHER_ALLOW_ANONYMOUS=1`).
- `.env` credential write → `0600` under `$SLITHER_HOME`, non-clobbering; CF token `set_sensitive`.

**Correctness**
- robots.txt parser replaced with `texting_robots` (wildcards, `$`, 5xx=disallow, BOM, groups),
  routed through the SSRF guard, matched against path+query.
- JSON-LD `@graph` / array / array-`@type` extraction for `schema_types`.
- Redirect-loop false positive fixed (requires a genuinely repeated URL in the chain).
- Scoring: site-level issues score `max(1, urls)` (deduct + count); self-referencing canonical
  no longer emitted as a fault.
- Byte→char length counts (title/meta/H1/H2/alt); multi-token `rel~=` selectors; case-insensitive
  `meta[name=… i]`; heading-sequence evaluated in true document order (parser now includes H1).
- Sitemap `<loc>` in CDATA and with entity escapes parsed correctly (new quick-xml event model).
- PSI INP read from CrUX field data (Option; skipped when absent); score rounded, error/inspect
  truncation panic-safe.
- Server rejects non-`local` backends with a clear error (was silently downgrading to local).

**Dependencies**
- `cargo update` applied for the RUSTSEC advisories (quick-xml, rustls-webpki, and the unsound
  `anyhow`/`rand` where a compatible update exists).

**Dead code**
- Removed the unreachable `--html` flag, the never-written `entities` table (+ MCP enum value),
  and fixed the structurally-unreachable queue DoS guard to a real `COUNT(*)`.

**MCP compliance**
- Protocol-version negotiation (echo the client's version when supported); honest capability
  declaration; `structuredContent` on tool results; tightened input schemas; sharper,
  model-facing tool descriptions.

---

## Deferred from the pre-ship blocker sprint

Nineteen ship blockers were fixed (see `CHANGELOG.md`). These were found alongside them, verified,
and deliberately not fixed — none of them makes the audit wrong, and each is a change of shape
rather than a defect.

**Known trade-off, already shipped.** Job ownership uses a heartbeat rather than a pid liveness
check, because a recycled pid reads as alive (job stranded forever) and a pid from another
namespace reads as dead (a live job reclaimed). The cost is that a crashed process's jobs are
reclaimed 120-180s after the crash rather than at the next startup.

**Verified as NOT defective — a shared code shape, no observable bug.** `canonicals.rs`,
`hreflang.rs` and `sitemaps.rs` all look declared URLs up against page records without resolving
redirects, which is the same shape that caused the redirect-identity cluster. I tested all three
against fixtures where the declared URL redirects: a canonical resolving through a 301 back to its
own page produces no finding, hreflang return links through a redirect produce no finding, and the
sitemap case produces an `info` note that is literally true. Worth routing through `UrlAliases` for
consistency, but there is nothing to fix today.

**Report wording and presentation**
- The Pages tab and structure panel label any non-indexable page `Noindex`, including 404s and
  500s, where there is no robots directive at all. "Not indexable" is the accurate word; the two
  call sites must change together (`templates/components/tab-pages.html`, `report/html.rs`).
- A category whose findings are all site-level now shows no sidebar count badge, because the badge
  is gated on the affected-URL count and those issues legitimately have none. The severity dot still
  carries the colour, so nothing is hidden.

**Crawler**
- When two requested URLs land on the same final URL, the second page is dropped entirely and its
  redirect chain is lost with it (`crawler/mod.rs`). Correct as dedup; it does mean one redirect
  source can go unreported.
- `rel=next`/`rel=prev` is parsed but never analyzed. Arguably correct — Google dropped it as an
  indexing signal in 2019 — but a `<link rel=next>`-only archive audits as a healthy one-page site
  with no comment.
- UA cloaking is undetectable by design: one user agent, one fetch, no second request to compare.

**Library hygiene**
- `report::sitemap_gen::generate_sitemap` now has no callers outside its own tests. It is the
  unguarded helper that caused the empty-sitemap bug; deprecate or delete it.
- `sitemap_gen.rs` restates the indexability rule instead of calling `analysis::is_indexable_html`.
  Semantically identical today, which is exactly how the two copies drifted last time.

---

## P1 backlog (should fix next)

### MCP-native surface (highest leverage — this is the product)
- **Adopt the official `rmcp` SDK (3.1.2)** in place of the hand-rolled transport. Gains:
  current-spec version negotiation, schema derivation from Rust types, Streamable HTTP for remote
  use, and progress notifications for long crawls — for a net **reduction** in maintained protocol
  code. Decision item D-1.
- **Expose crawl artifacts as MCP resources** (`resources/list` + `resources/read`) — `crawl.json`
  and `report.html` as `slither://job/{id}/crawl.json` etc. — instead of stuffing them into tool
  results. This is the correct home for the HTML report in an MCP world.
- **`query_crawl` backed by SQLite**, not whole-blob JSON parsing. Add `pages` and `issues` tables
  written alongside `crawl.json`, indexed on `(job_id,url)` and `(job_id,category,severity)`, so
  filters and cross-crawl queries ("URLs that lost their title in the last 5 crawls") are real SQL.
- **Progress notifications** for `crawl_site` using the existing jobs/executor machinery, so an
  agent can stream "142/500 pages" instead of polling.
- **Fix the MCP `query` category doc** (`js` → `javascript`; add `performance`).

### Crawler correctness
- **`<base href>` handling** (C-BASE) — resolve relative URLs (links, images, canonical, hreflang)
  against the document base, not the page URL. Also resolve discovered links against the final
  post-redirect URL (C-REDIR).
- **Charset-aware decoding** (`encoding_rs`) — stop corrupting non-UTF-8 pages (breaks dup hashes).
- **Sitemaps**: gzip support; don't stop discovery on an HTML-200 that isn't a sitemap; add a
  fetch budget + visited-set for index loops; normalize `<loc>` before coverage comparison.
- Seed sitemap URLs into the crawl queue so real orphans become discoverable (A-ORPHAN).

### Analyzer correctness & 2026 relevance
- **Normalize both sides of every URL join** (A-NORM) — broken-link, canonical-target, hreflang,
  and sitemap-coverage checks join raw `link.url`/`canonical`/`<loc>` against normalized `p.url`,
  so trailing-slash/query-order mismatches produce false results.
- **Gate presence checks on `is_analyzable()`** (A-NONHTML) — stop flagging PDFs, images, 404s,
  and status-0 timeouts for "missing title/description/H1/canonical".
- **Parse the `Link: rel=canonical` HTTP header** and resolve relative canonicals (A-CANONDEAD)
  so header-canonicalized and relatively-canonicalized pages aren't reported "missing canonical".
- **BCP-47-aware hreflang validation** (A-HREFLANG) — accept `zh-Hant`, `es-419`, `en-GB` casing;
  reject reversed `us-en`.
- **Per-check scoring caps** before the per-category cap (A-SCORECAP) so one high-frequency check
  can't consume a category's whole budget.
- **New robots.txt / AI-crawler analyzer** — report `GPTBot`, `Google-Extended`, `ClaudeBot`,
  `CCBot`, `PerplexityBot` policy, `noai`/`noimageai`, and IndexNow presence. Requires threading
  robots.txt into `AnalysisContext` (currently dropped after fetch). Biggest single 2026 gap.
- Treat `X-Robots-Tag` as a noindex source everywhere (add `effective_robots_directives()`).
- Score by **affected/total ratio**, not absolute counts, so large sites aren't all pinned to 0–40.
- Structured-data required-field tables aligned to Google 2026 (`Organization` needs name only;
  `Product` needs name + one of offers/review/rating); flag FAQ/HowTo rich-result deprecations;
  prefer JSON-LD signal.
- Analyze the already-extracted-but-ignored `og_tags` and `PaginationData`.
- Server-latency check driven by `response_time_ms` so Performance isn't silently empty without
  `--pagespeed`; add a "not measured" state distinct from "verified clean".
- JS analyzer: detect JS-injected `meta robots`/hreflang (most dangerous JS-SEO failure);
  static-vs-rendered content delta; count only head-position external scripts as render-blocking.

### Rendering & server
- PSI client: add timeout, `buffer_unordered` concurrency, and 429/quota backoff (2–4 h → minutes).
- Wire server-side rendering backends (or keep rejecting them — currently rejected cleanly).
- Playwright: bound `wait_for_navigation` with `config.timeout_seconds` (P0 hang if the feature is
  used); pass `--no-sandbox` only as root or under `SLITHER_NO_SANDBOX=1`.

### SQLite / jobs hardening
- `busy_timeout` + `PRAGMA synchronous=NORMAL`; a read-only connection for the dashboard.
- Conditional terminal status transitions (`WHERE status NOT IN ('cancelled','failed')`) and a
  real per-job `CancellationToken` so cancel actually stops the crawl.
- Heartbeat-based orphan recovery (don't nuke another process's in-flight jobs).

---

## P2 backlog (nice-to-have)
- Webhook HMAC signing (`X-Slither-Signature`) + persisted delivery queue that survives restart.
- HMAC/`subtle` for the API-key comparison; `X-Content-Type-Options: nosniff` +
  `Content-Disposition: attachment` on the report endpoint.
- `spawn_blocking`/`tokio::fs` for the synchronous rusqlite + `std::fs` calls in async handlers.
- Dashboard: merge DB rows with the CWD scan instead of short-circuiting; record CLI crawls.
- Sitemap **generation** (`slither sitemap`) — a real Screaming Frog parity gap.
- Pixel-width model for CJK/fullwidth glyphs; anchor-text and link-equity analysis.
- Configurable global crawl-concurrency (currently hardcoded `Semaphore::new(3)`).

---

## MCP redesign (Phase 3 target)

Current surface (7 tools) is close to right. Target design:

| Tool | Shape | Notes |
|---|---|---|
| `crawl_site` | async → `{job_id, summary}` | Uses jobs/executor; emits progress notifications. |
| `get_crawl_status` | `{job_id}` → status/progress | Also lists recent jobs when `job_id` omitted. |
| `query_crawl` | filtered + **paginated** | The workhorse. SQLite-backed. Filter by status, issue category, severity, URL glob, depth. Token-budget-aware (summaries + pages, never a 500-page dump). |
| `inspect_page` | single URL, static/rendered/compare | Fast, no crawl. |
| `compare_crawls` | baseline vs current | New/resolved issues, score delta. |
| `list_crawls` | recent jobs | Discovery. |
| `export_report` | → resource link | Returns a `slither://` resource URI, not an inline blob. |

Every tool: a description written **for the model**, JSON-Schema inputs with `enum`/range
constraints, `outputSchema` + `structuredContent`, and pagination on anything unbounded.

**Transport:** stdio stays first-class (that's how Claude Code/Desktop launch it). Adopt `rmcp`
(D-1) to get Streamable HTTP + version negotiation for free.

**DX front door — the two-minute setup (tested, works today):**
```
claude mcp add slither -- slither serve --mcp
```
Then: *"Crawl example.com and tell me what to fix first."*

---

## Decision items

Status legend: **[DONE]** implemented in this work · **[DEFERRED]** not done, with rationale ·
**[OWNER]** needs a human action this session can't perform.

- **D-1 — Adopt `rmcp`, retire the hand-rolled MCP transport.** **[DEFERRED — rationale below.]**
  The hand-rolled stdio layer was brought to current-spec compliance instead: protocol-version
  negotiation, honest capability declaration, `structuredContent`, and MCP resources, all verified
  end-to-end. Since it works over stdio (how Claude launches it) and is demonstrated, a full rmcp
  rewrite was judged too risky to do autonomously for its main marginal benefit (Streamable HTTP
  remote transport, which isn't currently used). rmcp adoption remains the recommended path when
  remote HTTP transport is needed — a discrete future migration, not a blocker.

- **D-2 — Cut the Python entity/NER package (`slither-entity`).** **[DONE.]** NER,
  entity density, and silo comparison are things **Claude does natively and better**; the package
  hardcodes a pest-control gazetteer, owns the only heavyweight model dependency (spaCy + 42 MB
  model), and is 100% unreachable in MCP mode. Replace it by exposing crawled `body_text` through
  MCP and letting Claude do entity analysis. Removes the largest install-footprint item.

- **D-3 — Keep the link-graph capability but move it into Rust; cut the Python
  `slither_link` package.** **[DONE.]** Reimplemented in `slither-core::link_graph` (PageRank via
  power iteration, orphans, hubs, weakly connected components, silos — no new dependency), exposed
  as the `slither_link_graph` MCP tool and a rewritten `slither link <crawl.json>` CLI. PageRank/
  centrality over thousands of pages is the one
  thing Claude *can't* do cheaply and whose output (ranked URLs by internal authority) is exactly
  what an LLM wants. But the shipped Python version ships **PageRank turned off**, drags in 97 MB of
  unused `scipy`, and emits a D3 visualization Claude can't read. Rebuild as a `query_crawl`-style
  MCP tool returning `{pagerank_top_n, orphans, hubs, components}` as JSON from the graph the Rust
  side already builds.

- **D-4 — Cut `slither-common` and `slither-seo`.** **[DONE.]** Both packages removed along with the
  entire `python/` tree and the `slither setup` Python venv/pip/spaCy flow. `slither-common` was 142 lines of
  Rich terminal styling nobody sees under MCP; `slither-seo` is a meta-installer pointing at a dead
  404 URL at a stale version. If D-2/D-3 land, both have zero consumers. Fold the one useful piece
  (Python 3.10+ gate) into `slither setup` if the Python path survives at all.

- **D-5 — Drop `--backend cloudflare` for whole-site crawls; keep CF only for `screenshot`,
  `extract`, and `inspect --rendered`.** **[DONE.]** The dead `/crawl` client and the hybrid
  `run_cf_crawl` backend were removed; `--backend` now accepts `local|playwright`. The CF crawl path dropped pages on error,
  loses depth/redirect/header data, burns the 10-min/day free tier one browser session per page,
  and hits the target 3× per page. Its `/crawl` job client (~150 lines) is already dead code.
  Playwright is the better default rendering backend (free, captures console errors, degrades
  gracefully) once its timeout bug is fixed. Removing the CF crawl backend deletes the most broken
  path with no capability loss.

- **D-6 — Delete `--pagespeed-local` and `playwright/screenshot.rs`.** **[DONE.]** Both removed
  (`--pagespeed-local` gated a validation error onto a permanent `bail!`; the Playwright screenshot
  module had zero callers), along with `pagespeed/local.rs`.

- **D-7 — Distribution story.** **[OWNER — code side DONE.]** A GitHub-native
  `.github/workflows/release.yml` now builds the CLI, generates `SHA256SUMS`, and publishes a
  release with `v`-tagged artifact names matching `install.sh` (which verifies checksums). The
  remaining action is the owner's: **make the repo public** (recommended — then the release
  binaries and `install.sh` work), or rename the crate and publish to crates.io. Until then the
  README documents build-from-source as the truthful path.

- **D-8 — TUI dashboard "Recent Crawls" pane.** **[DEFERRED.]** Kept as-is. CLI crawls still don't
  enter the DB, so once any server job exists the pane shows only server jobs. Low-value plumbing
  (merge DB + CWD scan, or have `slither crawl` append a lightweight record); deferred in favor of
  higher-value MCP-first work.

---

## Follow-up implementation pass — what shipped beyond the initial P0s

After the P0 pass, a second pass implemented the decision items and most of the deferred P1
correctness backlog. Now `[FIXED]` and verified (build/clippy/fmt/tests green, both feature sets):

- **Cuts:** D-2/D-3/D-4 (Python packages → Rust link-graph), D-5 (CF crawl backend), D-6 (dead flags).
- **Crawler/analyzers:** `<base href>` (C-BASE), charset decoding (C-ENC), relative-canonical +
  `Link` header (A-CANONDEAD), `is_html_page` presence gating (A-NONHTML), normalized URL joins
  (A-NORM), BCP-47 hreflang (A-HREFLANG), X-Robots-Tag everywhere + genuine-conflict detection
  (A-XROBOTS), JSON-LD `@graph`, the new robots.txt/AI-crawler analyzer (17th analyzer), the JS
  render-blocking fix, and a server-latency Performance check.
- **Rendering/server/jobs:** PSI timeout + bounded concurrency + quota-abort; Playwright
  navigation timeout + sandbox gating; SQLite `busy_timeout`/`synchronous`; conditional terminal
  status transitions; report-endpoint security headers; configurable crawl concurrency.
- **MCP-native:** crawl artifacts exposed as MCP resources (`slither://job/{id}/…`); the
  `slither_link_graph` tool; artifact-return polish. (SQLite-backed `query_crawl` and progress
  notifications remain in the backlog below — the async-job + poll pattern already fits long
  crawls, so streaming progress is not required.)
- **Features/CI:** `slither sitemap` generation; GitHub-native release workflow with checksums.

### Second-pass audit — closed

Every item the second audit raised is fixed, including all of the ones initially deferred:
concurrent MCP dispatch (a slow tool call no longer blocks `ping` — verified live),
**enforced MCP cancellation** (a cancelled request emits no response and its work stops —
verified live), the atomic transactional queue cap, the server's request-level REST/MCP test
suite, A-SCORECAP per-check caps, **proportional affected/total scoring**, `sitemap_data`
retention across the re-pipeline, and 6to4/Teredo/NAT64 decoding in the SSRF guard.

### MCP response duplication — resolved

Every tool response used to carry its payload twice: once as `structuredContent` and once as an
escaped JSON copy in `content[0].text`, together 96% of the response. The text copy exists for
clients that predate `structuredContent` (protocol revision `2025-06-18`), so it is now sent
only to clients that actually need it: the negotiated revision is retained per session, and a
client on `2025-06-18` or later gets a one-line summary in the text block instead of the
duplicate. Measured live on a 25-page crawl, `slither_query` went from 32,603 to 16,340 chars
(~8,150 → ~4,085 tokens) for a modern client, with the legacy shape byte-identical to before.

### Still deferred (backlog)

Both remaining second-pass items are now done: proportional (affected/total) scoring and
enforced MCP cancellation. What follows is older backlog, unrelated to that audit.

- `query_crawl` on a SQLite page/issue index (works today via whole-blob JSON; a scale/cross-crawl
  improvement).
- rmcp adoption (D-1) for Streamable HTTP.
- Redirect-base link resolution (C-REDIR);
  seeding sitemap URLs for full orphan detection (A-ORPHAN); structured-data required-field tables
  vs Google 2026 + FAQ/HowTo deprecations; `og_tags`/`PaginationData` analysis; JS-injected robots
  detection; CJK pixel-width model; webhook HMAC + persisted delivery queue; `spawn_blocking`/
  `tokio::fs` for the synchronous DB/FS calls; heartbeat-based orphan recovery; dashboard merge (D-8).
