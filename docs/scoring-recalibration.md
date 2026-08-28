# Health-score recalibration — evidence and proposed model

**Status: implemented.** See `slither-core/src/analysis/scoring.rs`; the calibration table is
enforced by `slither-core/tests/scoring_tests.rs`. The Python prototype used to derive the weights
is kept as `docs/scoring-model-prototype.py` for reference.

Owner decisions taken: security headers are reported but **not scored**; a good site with minor
issues lands in **high B** rather than low A; Ahrefs-equivalent leniency on partial 5xx is
accepted.

## Why the current score is wrong

The current model cannot rank sites by quality. Measured:

| Site | Current score |
|---|---|
| example.com (blank placeholder) | 69 (D) |
| web.dev (Google's web-quality reference) | 68 (D) |
| stripe.com | 67 (D) |
| nasa.gov | 65 (D) |
| callwaynes.com | 54 (F) |
| homeshieldpestcontrol.com | 57 (F) |

A parked placeholder outscores Google's own reference site. Four causes:

1. **Info findings are a fixed ~11–12 point tax.** They cost 11.35 points on callwaynes and 12.14
   on homeshield — about a quarter of all deductions. Sitewide info issues hit 100% coverage and
   take their full per-category cap, across 16 categories. Nothing can reach an A.
2. **Non-ranking factors score.** Security headers took the full 2.00-point Security info cap on
   every site tested.
3. **Severity is inverted.** On homeshield a render-blocking script cost 6.75 while a real broken
   link plus a 404 cost 6.00 combined.
4. **Proportional scaling saturates at both ends.** On a 1-page site any issue is "100% of the
   site"; on a large site one template defect is also 100%. Both max out, so a slightly-flawed and
   a genuinely broken site look identical.

## What the research says

Primary sources first (Google Search Central, Mueller), industry tiering where Google is silent.

**Blocks or removes indexing — must dominate the score**
- 4xx: "Google doesn't index URLs that return a 4xx status code, and URLs that are already indexed
  and return a 4xx status code are removed from the index."
- 5xx/429: crawl rate drops proportionally; indexed URLs "eventually dropped."
- noindex / robots.txt blocking / canonical pointing at a non-indexable page.

**Materially affects ranking**
- Missing titles (Screaming Frog: High), exact duplicate content (signal dilution + crawl waste),
  HTTP/mixed content, orphan pages.

**Real but secondary**
- Duplicate/long titles, redirect chains, hreflang errors (consequence is that annotations are
  *ignored*, not penalised), Core Web Vitals (Google: "no single signal", explicitly a tiebreaker).

**Low — should barely register**
- Meta descriptions: snippet input, **not a ranking factor**.
- H1/heading hierarchy: Mueller — "Your site is going to rank perfectly fine with no H1 tags or
  with five H1 tags." Not in Lighthouse's SEO category at all (it's Accessibility).
- Image alt: accessibility and Google Images, not web ranking. Also Lighthouse Accessibility.
- Structured data: rich-result **eligibility**, not ranking.
- URL formatting, image dimensions, readability.
- Missing sitemap: Google says a small, well-linked site may not need one.

**Not a ranking factor — should not score at all**
- HSTS, CSP, X-Content-Type-Options, Referrer-Policy, X-Frame-Options. Mueller, verbatim: "the
  security headers are more about, well, security." (HTTPS itself is separate and does count.)
- AI-crawler / noai directives: a deliberate policy choice, not a defect.

**How the industry scores.** Ahrefs publishes the only concrete formula:
`health = (total URLs − URLs with errors) / total URLs × 100`, and explicitly: "Warning and Notice
issues … do not affect Site Audit health score at all." Semrush weights errors above warnings above
notices, and calibrates >80 = well-optimised, <60 = significant issues. Screaming Frog and Sitebulb
decline to produce a single score at all. The dominant pattern is a **small, strictly-defined error
set drives the score; everything else is advisory.**

## Proposed model

Two parts.

**1. Page-defect backbone (Ahrefs-aligned).** The share of pages that are actually defective.
- *Broken* page (100-point weight at full coverage): 5xx, 4xx, no response, redirect loop,
  canonical to non-indexable, conflicting directives.
- *Materially defective* page (55-point weight): missing title, exact duplicate, soft 404,
  HTTP/mixed content/insecure form, orphan.
- Counted as a **union of pages**, so a page broken two ways is one broken page, and a page already
  counted as broken is not counted again as defective.
- The denominator has a floor of 5 pages: a proportion measured over one or two pages is noise.

**2. Bounded penalties** for problems that are site-level or link-level rather than a defective
page — capped so they can shape the top of the range but never dominate:
- site-high (cap 20): broken internal links, no sitemap, conflicting canonicals
- site-medium (cap 8): title duplicates, missing/relative canonical, redirect chains, hreflang
  errors, CWV poor, thin content, structured-data parse errors, JS-injected SEO content
- low (cap 5): everything else real but minor — meta description, headings, alt text, URL
  formatting, image dimensions, readability

Note `broken_internal` is a bounded penalty, not a page defect: a page linking to one dead URL is
not itself broken, and the dead target is already counted by the 4xx check.

## Calibration

Synthetic scenarios of known severity:

| Scenario | Proposed |
|---|---|
| Flawless site | 100 (A) |
| Cosmetic issues sitewide (meta length, heading order, alt, no H1) | 96 (A) |
| Security headers missing sitewide | 100 (A) |
| One dead link in global nav + the 404 itself | 88 (B) |
| Missing titles on 20% of pages | 89 (B) |
| Exact duplicate content on 30% | 84 (B) |
| 10% of pages 5xx | 90 (A) — matches Ahrefs' formula exactly |
| 25% of pages 404 | 75 (C) |
| 40% of pages 404 | 60 (D) |
| Sitewide indexability collapse | 0 (F) |
| Half 5xx + sitewide noindex + mass duplication | 0 (F) |

Real sites:

| Site | Current | Proposed |
|---|---|---|
| gov.uk | 81 (B) | 95 (A) |
| smashingmagazine.com | 70 (C) | 95 (A) |
| developer.mozilla.org | 84 (B) | 95 (A) |
| stripe.com | 67 (D) | 94 (A) |
| web.dev | 68 (D) | 88 (B) — real duplicate x-default + invalid lang codes |
| nasa.gov | 65 (D) | 82 (B) |
| homeshieldpestcontrol.com | 57 (F) | 81 (B) |
| wikipedia.org | 72 (C) | 79 (C) |
| example.com | 69 (D) | 75 (C) |
| callwaynes.com | 54 (F) | 73 (C) |

## Measured after implementation

Live, through the shipped binary (25-page crawls):

| Site | Before | After |
|---|---|---|
| developer.mozilla.org | 89 (B) | 90 (A) |
| gov.uk | 87 (B) | 89 (B) |
| smashingmagazine.com | 76 (C) | 88 (B) |
| stripe.com | 75 (C) | 86 (B) |
| web.dev | 68 (D) | 79 (C) — genuine duplicate x-default, invalid lang codes |
| nasa.gov | 66 (D) | 73 (C) |
| example.com | 69 (D) | 70 (C) |
| callwaynes.com | 54 (F) | 73 (C) |

A separate false positive surfaced during calibration and was fixed: the title selectors were
unscoped, so the `<title>` inside an accessible inline SVG counted as a second document title.
gov.uk, MDN, stripe.com and Smashing Magazine were all reported as having "Multiple Title Tags" on
25 of 25 pages — the tool was penalising correct accessibility markup.

## Follow-up

Keep the reference set as a **calibration test** so weight changes are validated against
known-good sites. The absence of one is why this drifted unnoticed.


---

## Field validation — deliberately varied site shapes

After implementation, Slither was run against site shapes chosen to stress different assumptions:
a JS-heavy SPA (react.dev), a multilingual site (mozilla.org), a CJK site (zh.wikipedia.org), two
ecommerce sites, and a news site (bbc.com). **Five defects surfaced that neither code-review pass
had found**, which is the argument for doing this routinely:

| Defect | Symptom | Fix |
|---|---|---|
| Empty crawl scored A (100) | A seed blocked by robots.txt produced 0 pages and a perfect score | Reports "No pages crawled" |
| CJK measured with Latin metrics | zh.wikipedia's full-length titles flagged "Below 30 Characters" on 25/25 pages; pixel width ~half actual | Full-width ranges are double-width; character-count rules skipped for full-width text |
| Decorative images counted as missing alt | 13,067 of 17,064 (76%) were `data:` URI spacers and tracking pixels | `ImageData::needs_alt_text` excludes `data:` URIs and 1×1 images |
| Mixed content over-weighted | books.toscrape.com scored 19 (F) for loading jQuery over http | Bounded rather than a page defect — the page still indexes. 19 → 67 |
| Missing sitemap cost 12 points | zh.wikipedia publishes none and indexes fine; Google says a well-linked site may not need one | Demoted to the medium tier. 66 → 74 |

Final calibration:

| Site | Score |
|---|---|
| developer.mozilla.org | 90 (A) |
| gov.uk | 89 (B) |
| smashingmagazine.com | 88 (B) |
| stripe.com | 86 (B) |
| nasa.gov | 81 (B) |
| example.com | 80 (B) |
| web.dev | 79 (C) — genuine duplicate x-default, invalid language codes |
| spa (react.dev) | 76 (C) |
| homeshieldpestcontrol.com | 77 (C) |
| CJK (zh.wikipedia.org) | 74 (C) |
| callwaynes.com | 73 (C) |
| multilingual (mozilla.org) | 72 (C) |
| news (bbc.com) | 71 (C) |
| ecommerce (books.toscrape.com) | 67 (D) — mixed content sitewide |
| crawl blocked by robots.txt | – (no pages) |

Both delivered client audits were re-run and their reports corrected where the tool had been
wrong — most notably the alt-text figure, which was a false positive in its entirety on one site.


### Second field pass — faceted ecommerce, bot walls, and scale

Three more shapes: a faceted-navigation store, sites behind bot protection, and a synthetic
12,000-page site carrying a facet trap (`?color=&size=&sort=`), deep path nesting, duplicate
pages and linked 404s. Two more defects:

| Defect | Symptom | Fix |
|---|---|---|
| Blocked crawls were still graded | g2.com answered the seed with 403; one page crawled, scored **92 (A)** | If every page seen is non-2xx there is nothing to grade — reports that the crawl was blocked |
| Report unbounded at scale | A 12,000-page crawl produced a **170 MB** single-file report no browser opens | Affected-URL lists, page rows, tree entries and the JSON island are each bounded worst-first with a visible note. 170 MB → **14 MB**; peak memory 1.4 GB → 668 MB |

Confirmed sound at scale: 12,000 pages in 0.6s, budget honoured exactly, deep nesting bounded,
and the facet trap enumerated without runaway.

**Running the tool against unfamiliar site shapes has now found 7 defects across 9 shapes, none
of which three code-review passes caught.** It is the highest-yield check available and worth
repeating whenever the analyzers change.
