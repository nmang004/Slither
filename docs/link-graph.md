# The internal link graph

```bash
slither link crawl.json
slither link crawl.json --json --top 50
```

Internal linking is the part of technical SEO most often left unexamined, because it is invisible
in any single page's markup — it only exists across a whole crawl. `slither link` builds that graph
and reports what it shows.

It reads a `crawl.json` and performs no network I/O, so it is cheap to re-run with different
`--top` values.

---

## How the graph is built

**Nodes** are the crawled pages, keyed by normalized URL. A page that was never crawled is not a
node — so a link pointing outside the crawl budget, or to an uncrawled URL, contributes no edge.
This matters when reading the output: a small `--max-pages` produces a graph of the pages you
fetched, not of the site.

**Edges** are internal `<a href>` links, with three deliberate rules:

- **Destinations are resolved through redirects.** A page is recorded at the URL that finally
  served it, so an `href` naming a URL that 301s matches no node by string. Those edges are
  resolved to the page that served them rather than dropped. When they were dropped, the
  destination showed in-degree 0 — a false orphan — the source showed out-degree 0, and the
  component count, `avg_out_degree` and every PageRank value were computed over the wrong graph.
- **Repeated links from one page count once.** A header nav linking `/pricing` from every page
  contributes one edge per page, not one per occurrence, so a site-wide nav does not drown out
  editorial links.
- **Self-links are ignored.** A page linking to itself is not internal authority.

---

## What each number means

### PageRank

Slither computes classic PageRank over the internal graph: damping **0.85**, **40** iterations,
rank from dangling nodes (pages with no outbound internal links) redistributed uniformly.

Read it as **relative internal authority** — which pages your own site's link structure pushes
importance toward. It is not Google's PageRank, has no access to external links, and is only
meaningful *within* one crawl. Comparing a PageRank value between two crawls of different sizes is
meaningless; comparing the ranking within one crawl is the point.

The useful question it answers: *are the pages I most want to rank actually the pages my site
links to most?* A commercial site whose top five pages by PageRank are `/privacy`, `/terms`,
`/careers`, `/blog` and `/about` has its internal authority pointed at pages that will never earn
revenue.

### Orphan pages

Pages with **no inbound internal links** from anywhere else in the crawl. The seed is excluded by
identity (resolved through its redirect, so a site whose `/` 301s to `/en/` does not report its own
homepage as an orphan).

An orphan is reachable only if something outside the crawl links to it — a sitemap, an ad, an
external site. Google can still find it, but it inherits no internal authority and is usually
either forgotten or a mistake.

Two caveats before acting:

- A page is only an orphan **relative to the crawl**. If `--max-pages` cut the crawl short, pages
  whose only inbound links live on uncrawled pages will look orphaned. Check the crawl reached the
  whole site before believing an orphan list.
- Links injected by JavaScript are invisible to the default backend. On a JS-heavy site, run
  `--backend playwright` before treating orphans as real.

`orphan_page_count` carries the true total; `orphan_pages` is bounded by `--top`.

### Hubs

Pages with the highest **out-degree** — the ones linking out the most. Usually navigation,
category and index pages. A hub with high out-degree and low PageRank is a page distributing
authority it does not have; a hub with both is doing real structural work.

### Silos

Pages grouped by **first path segment** — `/blog/x` and `/blog/y` are both in the `blog` silo.
This is a crude proxy for site sections and is presented as such: it reflects URL structure, not
semantics or navigation. It is useful for spotting that 80% of your pages sit under one section,
or that a section you consider important has three pages in it.

`silo_count` carries the true number of distinct silos; the distribution list is bounded by
`--top`.

### Components

**Weakly connected components**, computed with edges treated as undirected. A healthy site is
normally one component: everything is reachable from everything else if you ignore link direction.

More than one component means the crawl found groups of pages with no internal links between them
at all. That is usually a genuine structural problem — a section reachable only from a sitemap, a
migrated subtree nobody linked back into — or an artefact of a truncated crawl.

### `total_edges` and `avg_out_degree`

Total internal links (after the dedup and redirect-resolution rules above) and the mean per page.
Their main use is as a sanity check: if `avg_out_degree` is near zero on a site you know has
navigation, the crawl did not see the links — almost always because they are JS-injected.

---

## Reading the output honestly

The graph describes **the crawl**, not the site. Every number above is conditioned on which pages
were fetched, whether links were rendered, and whether redirects resolved. Slither bounds the lists
and reports true totals so a truncated list is visible as truncated, but it cannot tell you that
your crawl budget was too small. Check `pages_crawled` against what you expect the site to have
before drawing structural conclusions.
