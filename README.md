# Slither 🐍

*Made for Scorpion.*

**Find out what's wrong with a website — by asking.**

Slither visits every page of a website, checks it against 123 technical SEO rules, and tells you
what's broken and what to fix first. It runs on your own machine. Nothing is uploaded, there's no
account, and there's no subscription.

The part that makes it different: you can connect Slither to Claude and just *talk* to it.

> **You:** Crawl example.com and tell me what I should fix first.
>
> **Claude:** Crawled 42 pages. Health is B (83/100). The biggest problem is 6 pages returning
> 404s that are still linked from your main navigation. After that, 11 pages share the same
> title tag, which stops Google telling them apart…

No dashboards to learn, no CSV exports to pivot. You ask follow-up questions and get answers
grounded in a real crawl of the real site.

---

## Is this for me?

**If you look after a website** — yours or a client's — Slither tells you what search engines see
when they visit, and what's getting in their way. You don't need to know what a canonical tag is;
the report explains each finding in plain language and tells you why it matters.

**If you do SEO professionally**, it's a fast local crawler with a proper check catalogue, an
internal link graph with PageRank, sitemap generation, and exports that go straight into a
spreadsheet — without a per-seat SaaS bill or your client's data leaving your laptop.

**If you're a developer**, it's a single Rust binary with a REST API and an MCP server, so you can
put site-health checks in CI and fail a build when something regresses.

---

## What is a "crawl", and why would I want one?

A search engine finds your pages by following links, the same way a visitor would — that's
*crawling*. If a link is broken, a page is accidentally hidden from search, or ten pages claim the
same title, the search engine sees that too, and your site does worse than it should.

Slither does the same walk and writes down everything that would cause a problem. A typical run
finds things like:

- **Broken links** — pages linked from your site that no longer exist
- **Pages hidden from Google by accident** — a single misplaced tag can remove a page from search
- **Duplicate titles and descriptions** — the text that appears in search results, repeated across pages
- **Orphan pages** — real pages nothing links to, so nobody finds them
- **Slow pages** — measured with Google's own performance data
- **Missing image descriptions** — bad for accessibility and for image search

Every finding comes with what it is, which pages it affects, and what to do about it.

---

## Getting started

### 1. Install

Slither isn't on Homebrew or crates.io yet, so for now you build it from source. You'll need
[Rust](https://rustup.rs) — the installer is one command and takes a couple of minutes.

```bash
git clone https://github.com/nmang004/Slither.git
cd Slither
cargo build --release
```

That produces the program at `target/release/slither`. To be able to type `slither` from anywhere:

```bash
cp target/release/slither ~/.local/bin/
```

### 2. Run your first audit

```bash
slither crawl https://example.com/
```

You'll see progress as it goes, then a summary. It writes two files next to you:

- **`crawl-example.com-<date>.html`** — open it in your browser. This is the report to read, or to
  send to a client. It's one self-contained file that works offline.
- **`crawl-example.com-<date>.json`** — the same data for tools and scripts.

Add `--csv` if you'd rather have a spreadsheet.

**Be polite.** By default Slither waits between requests and obeys `robots.txt`, the file websites
use to tell crawlers what they may visit. Please only crawl sites you own or have permission to
audit.

### 3. Connect it to Claude (the good part)

If you use [Claude Code](https://claude.com/claude-code):

```bash
claude mcp add slither -- /full/path/to/target/release/slither serve --mcp
```

Now you can ask in plain English:

> Crawl mysite.com, then tell me which pages Google can't index and why.

> Compare that against last month's crawl — did anything get worse?

> Which of my blog posts have no internal links pointing at them?

MCP ([Model Context Protocol](https://modelcontextprotocol.io)) is just the standard that lets
Claude call tools on your machine. Slither speaks it, so Claude can run crawls and read results
directly.

---

## Reading your report

The report opens with a **health score out of 100** and a letter grade:

| Grade | Score | Roughly means |
|---|---|---|
| **A** | 90–100 | In good shape |
| **B** | 80–89 | Solid, with the usual small stuff |
| **C** | 70–79 | Needs work |
| **D** | 60–69 | Real problems |
| **F** | below 60 | Something is seriously wrong |

A well-built site with ordinary imperfections should land in the **high B** range. If you're
seeing a C or worse, something genuine is wrong rather than the tool being harsh.

Two honest caveats, because a score that oversells itself isn't useful:

- **The score is for tracking one site over time**, not for comparing two different sites. It
  answers "is this better than last month, and where do I look first?"
- **It only measures technical health.** It knows nothing about your content quality, your
  backlinks or your rankings — the things that ultimately decide how you perform.

The category breakdown is the real output. The number is just the headline.
[How scoring works →](docs/scoring.md)

---

## What else it does

```bash
slither inspect https://example.com/pricing    # audit a single page, no crawl needed
slither link crawl.json                        # which pages have internal authority, which are orphaned
slither sitemap crawl.json -o sitemap.xml      # build a sitemap from the pages that should be indexed
slither serve                                  # REST API, for CI and automation
```

`slither link` is worth knowing about. It builds a map of how your pages link to each other and
runs PageRank over it — the same idea Google was built on — to show which pages your own site
treats as most important. It's the quickest way to notice that your internal linking is pushing
all its weight at your privacy policy instead of the pages that make money.
[More on the link graph →](docs/link-graph.md)

---

## Documentation

The README is the tour. [`docs/`](docs/README.md) is the reference.

| Document | What it covers |
|---|---|
| [Commands](docs/commands.md) | Every command — what it does, how it works, and the flags that matter |
| [Checks](docs/checks.md) | All 123 checks by category, what each detects, and the thresholds used |
| [Link graph](docs/link-graph.md) | PageRank, orphans, hubs and silos — how they're computed and how to read them |
| [Scoring](docs/scoring.md) | How the health score works, and what it doesn't claim |
| [MCP and REST](docs/mcp.md) | Tools, arguments, job lifecycle, security |
| [Cloudflare](docs/cloudflare.md) | The optional JavaScript-rendering integration |

---

## Your data stays yours

Slither is local-first, and that isn't marketing:

- **No accounts, no subscription, no telemetry.** Nothing is sent anywhere.
- **Reports work offline.** The HTML report loads no fonts, scripts or trackers from the internet.
- **Credentials never end up in your reports**, and are stored outside your working directory so
  they can't be committed by accident.
- **The server refuses to crawl internal addresses** by default, so pointing it at something can't
  be turned into a way of reaching your private network.

The two optional integrations that *do* reach a third party — Cloudflare rendering and Google
PageSpeed — are opt-in, and the Cloudflare code isn't even compiled into the default build.

---

## Optional extras

Some features are off by default because they need an external service or a browser:

```bash
cargo build --release --features cloudflare    # screenshots, AI extraction, JS-rendered auditing
cargo build --release --features playwright    # render pages in local Chrome
```

If you run a command whose feature isn't compiled in, Slither tells you what to rebuild with rather
than failing cryptically. Add `--pagespeed` to a crawl for Core Web Vitals from Google's API.

---

## Requirements

- **Rust** (stable, edition 2021; rustc 1.80 or newer)
- Optional: a Cloudflare account for rendering features, or local Chrome for the Playwright backend

---

## Contributing

Bug reports are genuinely welcome, especially **false positives** — a check that flags something
that isn't actually a problem is the worst failure mode for an audit tool, and the ones found so
far have all come from running it against real sites.

See [CONTRIBUTING.md](CONTRIBUTING.md) to get set up.

## License

MIT. See [LICENSE](LICENSE).
