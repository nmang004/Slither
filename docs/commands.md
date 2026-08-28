# CLI commands

Every command is `slither <command>`. Run `slither <command> --help` for the exact flag list; this
document explains what each command *does* and how it works, which the help text does not.

| Command | Needs network | Input | Purpose |
|---|---|---|---|
| [`crawl`](#slither-crawl) | yes | a URL | Crawl a site and analyse it |
| [`link`](#slither-link) | no | `crawl.json` | Internal link graph: PageRank, orphans, hubs, silos |
| [`sitemap`](#slither-sitemap) | no | `crawl.json` | XML sitemap of the indexable pages |
| [`inspect`](#slither-inspect) | yes | a URL | Single-page audit, no crawl needed |
| [`serve`](#slither-serve) | yes | — | REST API and/or MCP server |
| [`screenshot`](#slither-screenshot) | yes | a URL | Full-page screenshot (Cloudflare feature) |
| [`extract`](#slither-extract) | yes | a URL | AI structured extraction (Cloudflare feature) |
| [`setup`](#slither-setup) | no | — | Store integration credentials |
| [`version`](#slither-version) | no | — | Component versions |

---

## `slither crawl`

```bash
slither crawl https://example.com/ -o crawl.json
```

Crawls a site breadth-first from the seed URL, then runs every analyser over the collected pages.
This is the command the others consume.

### How it works

**The seed is resolved first.** Before anything else, Slither follows the seed's own redirect and
treats where it *lands* as the site under audit. Seeding `https://www.example.com/` on a site that
canonicalises to the apex used to classify every discovered link as external and end the crawl
after one page — with a passing grade and no warning. The URL that finally served a page is that
page's identity throughout; the URL as requested is preserved in its redirect chain.

**Scope** is the seed's host and port. A link to a different host is external and is recorded but
not crawled. `--follow-subdomains` widens this to subdomains of the same registrable domain.
Scheme is deliberately *not* compared, so an `http://` link on an HTTPS site is still internal.

**Politeness** is a single shared gate, not a per-worker sleep: `--concurrency` workers share one
`--delay` interval, so three workers with a 250 ms delay produce one request per 250 ms, not three.
`robots.txt` is fetched and honoured (RFC 9309), and a `Crawl-delay` larger than `--delay` wins.
`--ignore-robots` exists and is labelled *not recommended* for good reason: it is for auditing
sites you control.

**Budget.** `--max-pages` is an exact upper bound, not approximate. `--depth` bounds distance from
the seed.

**Safety.** Crawl targets are checked against private, loopback, link-local and cloud-metadata
ranges to prevent SSRF, with the DNS resolution that is vetted being the one actually dialled.
Set `SLITHER_ALLOW_PRIVATE_TARGETS=1` to audit an intranet or a local fixture.

### Output

By default writes the JSON *and* a self-contained HTML report beside it, sharing the stem of
`--output`: `-o crawl.json` produces `crawl.json`, `crawl.html`, and `crawl.csv` with `--csv`.

| Flag | Effect |
|---|---|
| `--no-html` | JSON only — what you want when an agent or a script is the consumer |
| `--csv` | Also write a CSV, one row per page |
| `--summary-only` | Metadata, issues and summary; drops the per-page array |
| `--include-body-text` | Include page text in the JSON (excluded by default; it dominates file size) |
| `--json-compact` | Minified JSON |

The HTML report is genuinely self-contained: no CDN, no external fonts, nothing fetched when it is
opened. It is safe to email to a client. Reports cap at 2,000 pages and 100 URLs per issue, and
say so in the file when they truncate.

### Rendering and Core Web Vitals

`--backend playwright` renders pages in local Chrome instead of fetching raw HTML — needed for
sites that inject their content, title or canonical client-side. Requires the `playwright` feature.
`--pagespeed` adds Core Web Vitals via the Google PageSpeed API (`--pagespeed-key`, or
`PAGESPEED_API_KEY`); `--pagespeed-sample N` limits it to N pages, since the API is rate-limited
and one call per page on a large crawl is slow.

---

## `slither link`

```bash
slither link crawl.json            # terminal summary
slither link crawl.json --json     # full report as JSON
```

Builds the internal link graph from a completed crawl and reports PageRank, orphan pages, hubs,
silos and connected components. It performs no network I/O.

See [link-graph.md](link-graph.md) for what each number means and how it is computed — that
document is the one to read before acting on the output.

`--top N` bounds every ranked list (default 15). The report always carries the true totals
(`orphan_page_count`, `silo_count`) alongside the truncated lists.

---

## `slither sitemap`

```bash
slither sitemap crawl.json -o sitemap.xml
```

Writes an XML sitemap containing the crawl's **indexable** pages: 2xx HTML that is not `noindex`.
A 404, a redirect, a PDF or a `noindex` page is excluded.

If nothing qualifies — or the crawl was run with `--summary-only`, so there is no page array to
read — it writes nothing and exits non-zero, saying which of the two applies. It will not produce
a schema-valid-looking empty `<urlset>`, which Search Console reads as an empty sitemap.

Above the 50,000-URL protocol limit it emits a `<sitemapindex>` plus chunk files rather than one
oversized file, which Google rejects whole.

---

## `slither inspect`

```bash
slither inspect https://example.com/pricing
```

Audits one URL and prints a single-page report: title, meta description, canonical, indexability
with the directives that decided it, status, word count, readability, links, images, schema types,
security headers, and the findings that apply to that page.

Useful for checking one page quickly, and for confirming a finding from a crawl without re-running
it. Inspecting a URL that redirects reports on the page that served it and still tells you about
the redirect.

With the `cloudflare` feature: `--rendered` audits the JS-rendered DOM, and `--compare` shows
static vs rendered side by side — the fastest way to see what a crawler misses on a JS-heavy site.

---

## `slither serve`

```bash
slither serve                 # REST API on 127.0.0.1:3001
slither serve --mcp           # MCP server over stdio
```

See [mcp.md](mcp.md) for the tools, the job lifecycle and the REST endpoints.

Two things worth knowing here:

- **Auth is off unless you set a key.** `--api-key`, or `SLITHER_API_KEY` in the environment. With
  no key, every route except `/api/v1/health` is open to anything that can reach the port — which
  is why the default bind is `127.0.0.1`, not `0.0.0.0`.
- **Jobs are shared state.** Every Slither process on the same `SLITHER_HOME` (default `~/.slither`)
  reads the same SQLite job store. Running an MCP server and a REST server at once is supported;
  jobs are owned and heartbeated, so one process no longer reclaims another's running work.

---

## `slither screenshot`

Full-page or element screenshots through Cloudflare Browser Rendering. Requires the `cloudflare`
feature and credentials. `--full-page`, `--selector`, `--viewport WxH`, `--format png|jpeg`.

## `slither extract`

AI structured extraction from a page through Cloudflare, with an SEO preset by default or your own
`--prompt` / `--schema`. Requires the `cloudflare` feature and credentials.

## `slither setup`

```bash
slither setup cloudflare
```

Writes credentials to `$SLITHER_HOME/.env` with `0600` permissions — not to the working directory,
so they cannot be committed by accident.

## `slither version`

Prints the versions of the installed components.

---

## Environment variables

| Variable | Purpose |
|---|---|
| `SLITHER_HOME` | Data directory (default `~/.slither`): job database, job output, credentials |
| `SLITHER_API_KEY` | Bearer token required by the REST API and MCP HTTP routes |
| `SLITHER_ALLOW_PRIVATE_TARGETS` | Allow crawling private/loopback addresses (intranet audits, local fixtures) |
| `SLITHER_CORS_ORIGINS` | Comma-separated CORS allowlist; CORS is closed by default |
| `SLITHER_MAX_CONCURRENT_CRAWLS` | Concurrent crawl cap for the server (default 3) |
| `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN` | Cloudflare rendering credentials |
| `PAGESPEED_API_KEY` | Google PageSpeed API key |
| `RUST_LOG` | Log filter (`info` by default; in MCP mode logs go to stderr, never stdout) |
