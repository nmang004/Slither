# Security policy

## Reporting a vulnerability

Please report security issues privately rather than opening a public issue.

Use GitHub's [private vulnerability reporting](https://github.com/nmang004/Slither/security/advisories/new)
on this repository. Include what you did, what happened, and what you expected — a proof of concept
is welcome but a clear description is enough to start.

You can expect an acknowledgement within a few days. Slither is maintained by one person, so please
allow reasonable time for a fix before disclosing publicly.

## What's in scope

Slither fetches URLs supplied by the user, parses hostile HTML, writes reports that embed crawled
content, and optionally exposes an HTTP API. The areas most worth attention:

- **SSRF.** Crawl targets and webhook URLs are checked against private, loopback, link-local and
  cloud-metadata ranges, including IPv4 tunnelled inside IPv6, and the address that is vetted is the
  one actually dialled. A way to reach an internal address without setting
  `SLITHER_ALLOW_PRIVATE_TARGETS=1` is a vulnerability.
- **Report injection.** Crawled content — titles, URLs, descriptions — is embedded in the HTML
  report and the CSV export. Content that escapes HTML context, or a CSV cell that executes as a
  spreadsheet formula, is a vulnerability.
- **Path traversal.** The server serves crawl artifacts by filename. Reaching a file outside a
  job's output directory is a vulnerability.
- **Authentication.** With `SLITHER_API_KEY` or `--api-key` set, every route except
  `/api/v1/health` must require it. A route that doesn't is a vulnerability.
- **Credential leakage.** Cloudflare and PageSpeed credentials must never appear in `crawl.json`,
  a report, or a log line, including in error messages.
- **Resource exhaustion.** Response bodies are capped and streamed against a budget; request bodies
  are capped. A way to exhaust memory with a hostile response is worth reporting.

## What's not a vulnerability

- **Crawling a site you don't own.** Slither honours `robots.txt` by default and rate-limits itself,
  but it is a crawler, and `--ignore-robots` exists for auditing sites you control. Using it against
  someone else's site is a misuse of the tool, not a flaw in it.
- **Running the server on `0.0.0.0` without a key.** The default bind is loopback and starting
  unauthenticated on a non-loopback host requires an explicit opt-in. Choosing that opt-in is your
  decision to make.
- **`SLITHER_ALLOW_PRIVATE_TARGETS=1` reaching private addresses.** That is what the variable is for.

## Supported versions

Slither is pre-1.0 and moving quickly. Fixes land on `main`; there are no backports to older tags.
