# The check catalogue

123 checks across 16 categories. Every one runs on `slither crawl`; a subset that makes sense for a
single page also runs on `slither inspect`.

Thresholds quoted here are the ones in the code. Where a threshold is a judgement call rather than
a rule from Google, the category note says so.

**Severity** is one of Critical, Warning or Info. It is not the same thing as scoring weight — see
[scoring.md](scoring.md), where what actually moves the score is the distinction between a page
being *broken*, *defective*, or merely imperfect.

**Gating.** Most content-quality checks only run on indexable HTML (2xx, HTML, not `noindex`). A
page excluded from the index cannot cause a duplicate-title problem, and reporting one on a 404 is
noise. This is why a crawl of a heavily `noindex`ed staging site reports far fewer findings than
its page count suggests.

---

## Response Codes (7)

The first thing to look at on any audit: pages that do not return content cannot rank.

| Check | Detects |
|---|---|
| 4xx Client Errors | Pages returning 400–499 |
| 5xx Server Errors | Pages returning 500+ |
| 3xx Redirects | Internal URLs that redirect — named by **the URL that redirects**, not its destination |
| Long Redirect Chains | Multi-hop chains that waste crawl budget and dilute signals |
| Redirect Loops | Chains that never resolve |
| No Response | URLs the crawler could not reach at all |
| Access-Restricted or Rate-Limited | 401/403/429 — reported as Warnings, since they usually mean the crawler was blocked rather than the page being broken |

## Security (10)

HTTPS and mixed content are ranking-relevant; the header checks are reported because they are
cheap to collect during a crawl and useful to a technical audience. They deliberately do **not**
affect the health score — a missing CSP header is a security finding, not an SEO one.

| Check | Detects |
|---|---|
| HTTP Pages (Not Secure) | Pages served over HTTP |
| Mixed Content | HTTPS pages loading HTTP subresources |
| Forms on HTTP Pages / Insecure Form Actions | Forms submitting over plaintext |
| Unsafe Cross-Origin Links | `target="_blank"` without `rel="noopener"` |
| Missing HSTS / CSP / X-Frame-Options / X-Content-Type-Options / Referrer-Policy | Absent or unsafe security headers |

## URL (9)

URL hygiene. Individually minor; collectively a signal of an unmanaged site.

Consecutive Slashes · Repetitive Path Segments · URLs Over 115 Characters · URLs with Parameters ·
URLs with Spaces · URLs with Underscores · URLs with Uppercase · Non-ASCII Characters ·
GA Tracking Parameters

Fragments are stripped before a URL becomes a page identity, so `/docs/#toc` and `/docs/` are one
page and no `#`-derived text is measured as part of the URL.

## Page Titles (8)

| Check | Detects |
|---|---|
| Missing Title Tag | No `<title>` in the document at all |
| Title Outside Head | A `<title>` the parser placed in `<body>` — almost always invalid markup in `<head>` (a tracking pixel, a stray `<div>`) closing the head early |
| Multiple Title Tags | More than one document title |
| Duplicate Titles | The same title on multiple indexable pages |
| Over 60 Characters / Below 30 Characters | Character-length bounds |
| Over 561px Wide / Below 200px Wide | Pixel-width estimate, which is what actually decides SERP truncation |

Inline SVG `<title>` elements are accessibility labels, not document titles, and are excluded from
all of the above.

## Meta Description (7)

Missing Meta Description · Meta Description Outside Head · Multiple Meta Descriptions ·
Duplicate Descriptions · Over 155 Characters · Below 70 Characters · Over 985px Wide

Same reasoning as titles: character count is the familiar number, pixel width is the one that
predicts truncation.

## Headings (9)

Missing H1 · Multiple H1 Tags · Duplicate H1 Headings · H1 Over 70 Characters · H1 Same as Title ·
Missing H2 · Duplicate H2 Headings · H2 Over 70 Characters · Non-Sequential Headings

"H1 Same as Title" is informational — sometimes correct, sometimes a sign of a template with
nothing specific to say.

## Content (5)

| Check | Detects |
|---|---|
| Duplicate Content | Pages with identical content hashes (noindex pages excluded) |
| Low Word Count | Thin pages |
| Soft 404 Pages | Pages returning 200 whose content says the page does not exist |
| Difficult / Very Difficult Readability | Flesch reading-ease bands |

Word counting excludes `<script>`, `<style>`, `<noscript>` and `<template>` contents — all of which
are markup rather than prose, and all of which inflated word counts before being excluded.
Readability is a heuristic and should be read as one, particularly on technical or non-English
content.

## Images (3)

Missing Alt Text · Alt Text Over 100 Characters · Missing Image Dimensions

`alt=""` is **not** reported as missing: it is the correct way to mark an image decorative.
Only a genuinely absent `alt` attribute counts. Data-URI images and 1×1 tracking pixels are
excluded as decorative by definition.

## Canonicals (9)

Missing Canonical Tag · Multiple Canonical Tags · Canonical Declared Twice ·
Canonical Outside Head · Relative Canonical URLs · Canonicalised to Different URL ·
Canonical to Non-Indexable · Canonical Chain · Canonical Loop

"Canonicalised to Different URL" is Info, not a warning — it is frequently intentional. The ones
worth acting on are the loops, chains and canonicals pointing at pages that cannot be indexed.

## Directives (2)

Conflicting Index Directives · Conflicting Follow Directives — a page whose `<meta robots>` and
`X-Robots-Tag` disagree.

Directives are user-agent scoped. `googlebot-news` binds Google News only and does not affect
general Search indexability; treating it as site-wide `noindex` silently suppressed around twenty
other checks on those pages.

## Hreflang (9)

Missing Self-Reference · Missing Return Links · Missing x-default · Invalid Language Codes ·
Duplicate Language Entries · Conflicting Hreflang Languages · Hreflang to Non-200 Pages ·
Hreflang to Noindex Pages · Hreflang to Non-Canonical

Hreflang is the category where tools most often produce noise, because the checks are relational —
they depend on both ends being crawled. A "missing return link" on a page whose counterpart was
outside the crawl budget is an artefact, not a finding.

## Links (9)

| Check | Detects |
|---|---|
| Broken Internal Links | Internal links resolving to 4xx/5xx — **including** links that redirect into an error |
| Orphan Pages | Pages with no inbound internal links (see [link-graph.md](link-graph.md)) |
| High Crawl Depth (5+) | Pages buried deep from the seed |
| No Internal Outlinks | Dead-end pages |
| Internal Nofollow Links | `rel="nofollow"` on internal links |
| Empty Anchor Text / Non-Descriptive Anchors | "click here", empty anchors |
| Excessive External Links | Unusually high outbound counts |
| Localhost Links | `localhost`/`127.0.0.1` links left in production |

## Structured Data (3)

No Structured Data · Schema Parse Errors · Missing Required Fields

Validates that JSON-LD parses and carries the fields its type requires. It is not a full
schema.org validator and does not replace Google's Rich Results Test.

## Sitemaps (10)

No Sitemap Found · Declared Sitemap Unreachable · Sitemap XML Parse Error · Empty Sitemap ·
Invalid Sitemap URL · Sitemap Over 50k URLs · Sitemap Over 50 MB · URLs in Multiple Sitemaps ·
Non-Indexable in Sitemap · Pages Not in Sitemap

Several of these are **site-level**: they describe the site, not a page, and legitimately carry no
URL list. They still count toward the score and are rendered as site-wide findings.

## Performance (11)

Core Web Vitals, only populated when the crawl ran with `--pagespeed`:

LCP exceeds 2.5s / 4.0s (Poor) · INP exceeds 200ms / 500ms (Poor) · CLS exceeds 0.1 / 0.25 (Poor) ·
FCP exceeds 3.0s · TTFB exceeds 800ms · Performance score below 90 / below 50 · Slow Server Response

The thresholds are Google's published good/needs-improvement/poor boundaries. Without
`--pagespeed`, only Slow Server Response (measured directly during the crawl) can fire.

## JavaScript (10)

Populated meaningfully only when a rendering backend is used, since the whole point is comparing
raw HTML against the rendered DOM:

Title / Meta description / H1 / Canonical / Structured data injected by JavaScript ·
Critical JavaScript errors · JavaScript console errors · Excessive JavaScript ·
Render-blocking scripts · Heavy third-party scripts

"Injected by JavaScript" findings are the valuable ones: they identify SEO-critical elements absent
from the HTML Google first receives. Only executable scripts count toward "Excessive JavaScript" —
JSON-LD and framework data blobs are not scripts in the relevant sense.

## Robots / AI (2)

AI Crawlers Blocked in robots.txt · AI Opt-Out (noai) Directive

Reported as **Info**, deliberately. Blocking GPTBot or setting `noai` is a legitimate business
decision, not a defect, and Slither reports it without an opinion. These are site-level findings.
