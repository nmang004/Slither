# Slither documentation

Slither is a local-first SEO toolkit. It crawls a site, analyses it against 123 checks in 16
categories, and reports the result as JSON, CSV, a self-contained HTML report, or structured
output over MCP.

The design assumption behind everything here: **the primary surface is the MCP server**, and the
conversation is the report. The HTML dashboard exists, but token-efficient structured output for
an agent matters more.

## Start here

| Document | What it covers |
|---|---|
| [commands.md](commands.md) | Every CLI command — what it does, how it works, and the flags that matter |
| [checks.md](checks.md) | The full check catalogue: all 123 checks by category, what each detects and why |
| [link-graph.md](link-graph.md) | How the internal link graph is built, what PageRank/orphans/hubs/silos mean |
| [scoring.md](scoring.md) | How the health score and letter grade are computed |
| [mcp.md](mcp.md) | The MCP server and REST API — tools, arguments, bounding, job lifecycle |
| [cloudflare.md](cloudflare.md) | The optional Cloudflare Browser Rendering integration |
| [releasing.md](releasing.md) | Cutting a release, the install script, Homebrew, and the naming situation |
| [scoring-recalibration.md](scoring-recalibration.md) | Research and citations behind the current scoring model |

## The shape of a session

Almost everything starts with a crawl, which produces a `crawl.json`. The other commands are
consumers of that file rather than separate crawlers:

```
slither crawl https://example.com/ -o crawl.json
                     |
                     +--> report.html      self-contained, no network at open time
                     +--> export.csv       one row per page (with --csv)
                     |
                     +--> slither link crawl.json      internal link graph
                     +--> slither sitemap crawl.json   XML sitemap of indexable pages
```

`slither inspect` is the exception: it audits a single URL directly and needs no crawl.

## A note on reading these docs

Where a check or a number is described here, it is the one the code actually applies — thresholds
are quoted from the source, not from general SEO advice. Where Slither deliberately disagrees with
another tool, or where a check is knowingly approximate, the document says so rather than
implying more precision than exists.
