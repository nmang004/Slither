# MCP server and REST API

```bash
slither serve --mcp     # MCP over stdio — the primary surface
slither serve           # REST API on 127.0.0.1:3001
```

Both transports share one SQLite job store under `SLITHER_HOME` (default `~/.slither`), so a job
started over MCP is visible over REST and vice versa.

---

## The MCP tools

Eight tools. The crawl ones are asynchronous: `slither_crawl` returns a `job_id` immediately, and
you poll `slither_status` until it completes before querying results.

| Tool | Purpose |
|---|---|
| `slither_crawl` | Start a crawl. Returns a `job_id`. |
| `slither_status` | Check a job, list recent jobs, or cancel a running one. |
| `slither_summary` | Health score, grade, issue breakdown, top categories, worst pages. |
| `slither_query` | Filter results by URL pattern, category and severity; paginated. |
| `slither_link_graph` | PageRank, orphans, hubs, silos — see [link-graph.md](link-graph.md). |
| `slither_compare` | Diff two crawls: score change, new issues, resolved issues. |
| `slither_inspect` | Single-page audit, synchronous, no job needed. |
| `slither_screenshot` | Screenshot via Cloudflare. Requires the `cloudflare` feature and credentials. |

### A typical session

```
slither_crawl  { "url": "https://example.com/", "max_pages": 200 }   -> job_id
slither_status { "job_id": "..." }                                    -> running | completed
slither_summary{ "job_id": "..." }                                    -> the headline
slither_query  { "job_id": "...", "severity": "critical" }            -> what to fix first
slither_query  { "job_id": "...", "url_pattern": "*/blog/*" }         -> narrow to a section
```

`slither_compare` against a previous `job_id` is the one to reach for on a re-audit: it answers
"what changed" directly instead of making the model diff two summaries itself.

---

## Responses are bounded

This is the part that matters most for agent use, and the part most likely to bite you if you
assume otherwise.

A site-wide check affects every page, so an unbounded list grows with the crawl. Before bounding,
a single default call on a 500-page crawl returned well over 100k tokens.

- `slither_query` lists at most `max_urls_per_issue` URLs per issue (default 5, max 500), and always
  reports `affected_url_count` with the true total plus an `affected_urls_truncated` flag. **Narrow
  with `url_pattern` rather than raising the cap.**
- `slither_link_graph` bounds every list by `top_n`, including orphans and silos, and reports the
  true totals beside them.
- `slither_compare` bounds its regression list.
- `resources/read` is windowed: 100 KB per read by default (1 MB maximum), with a truncation notice
  carrying byte counts, `totalBytes`/`truncated` metadata, and a `nextUri` that pages through the
  rest via `?offset=&limit=`. `resources/list` advertises each artifact's `size` so a client can
  decide before reading.

### Field naming

`affected_urls` means **a list** everywhere. `affected_url_count` means **a number** everywhere.
These were briefly inconsistent between `slither_summary` and `slither_query`, which read as an empty
list to a caller that had seen the other tool — if you have client code written against the older
shape, `slither_summary` now returns `affected_url_count`.

### Argument validation

Arguments outside a tool's published `inputSchema` are **rejected**, not ignored, and the error
names the offending argument alongside the accepted set. A misspelled `maxpages` used to be
silently dropped and the crawl ran to the 500-page default — against someone's production site.
Integral floats (`3.0`) are accepted where an integer is expected; genuinely out-of-range values
are refused with a message naming the parameter.

---

## Protocol details

- Version negotiation on `initialize`; only implemented capabilities are advertised.
- Requests are handled **concurrently**, so responses may arrive out of request order — correlate
  by `id`, as JSON-RPC intends. Each response is one complete newline-delimited line.
- `notifications/cancelled` is enforced: in-flight work is aborted and, per spec, no response is
  sent for the cancelled request. `initialize` is never cancellable.
- Results are returned as `structuredContent`.
- In MCP mode **all logging goes to stderr** — stdout is reserved for JSON-RPC.

---

## REST API

| Method | Route | Purpose |
|---|---|---|
| GET | `/api/v1/health` | Liveness. The only route that never requires auth. |
| POST | `/api/v1/jobs` | Create a crawl job |
| GET | `/api/v1/jobs` | List jobs |
| GET | `/api/v1/jobs/{id}` | Job status and result summary |
| DELETE | `/api/v1/jobs/{id}` | Cancel/delete a job |
| GET | `/api/v1/jobs/{id}/results/{filename}` | Download an artifact |
| POST/GET | `/api/v1/webhooks` | Register/list webhooks |
| DELETE | `/api/v1/webhooks/{id}` | Remove a webhook |

Creating a job — note that crawl settings live under `options`, and unknown top-level fields are
rejected rather than ignored:

```bash
curl -X POST http://127.0.0.1:3001/api/v1/jobs \
  -H 'Content-Type: application/json' \
  -d '{"type":"crawl","url":"https://example.com","options":{"max_pages":50}}'
```

`crawl` is the only executable job type. The others are refused with a 4xx naming what is
supported, rather than accepted and left queued forever.

### Webhooks

Registered webhooks fire on terminal job states: `job.queued`, `job.completed`, `job.failed`,
`job.cancelled`. Cancelling a running crawl delivers `job.cancelled` immediately — the crawl itself
is not abortable mid-flight, so waiting would delay the notification by the length of the crawl —
and again when the crawl actually stops. One-shot webhooks remain exactly-once.

---

## Security

- **Auth is off unless a key is set.** `--api-key` or `SLITHER_API_KEY`. Without one, every route
  except `/health` is open to anything that can reach the port. The default bind is `127.0.0.1`.
- **CORS is closed by default.** Allow specific origins with `SLITHER_CORS_ORIGINS`.
- **SSRF protection** on crawl targets and webhook URLs: private, loopback, link-local and
  cloud-metadata ranges are refused, including IPv4 tunnelled inside IPv6, and the address vetted is
  the one actually dialled. Override for intranet audits with `SLITHER_ALLOW_PRIVATE_TARGETS=1`.
- Request bodies are capped at 1 MB; artifact downloads are path-traversal safe.

---

## Running more than one process

Every Slither process sharing a `SLITHER_HOME` shares the job store. Jobs carry an owner and a
heartbeat, so an MCP server starting up no longer reclaims a REST server's running crawls.

The trade-off: a job orphaned by a crashed process is reclaimed after **120–180 seconds**, not
instantly. If you want full isolation — separate job stores per client, say — point each process at
its own `SLITHER_HOME`.
