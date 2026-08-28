# How the health score works

Every crawl produces a score out of 100 and a letter grade. This document explains how it is
computed and, more importantly, what it is and is not claiming.

The research and citations behind the current model are in
[scoring-recalibration.md](scoring-recalibration.md).

---

## The grade bands

| Score | Grade | Verdict |
|---|---|---|
| 90–100 | A | Excellent |
| 80–89 | B | Good |
| 70–79 | C | Needs Work |
| 60–69 | D | Poor |
| below 60 | F | Critical |

The verdict word is composition-aware: a low score with no critical findings is not described as
"Critical", because calling a site critical when nothing critical was found misrepresents it.

**Calibration target:** a competently built site with the ordinary imperfections every real site
has — some long meta descriptions, imperfect heading order, a few redirects — should land in the
**high B** range, not be taxed down into a C or D. A site scoring in the 50s should have something
genuinely wrong with it.

---

## The model: pages, not findings

The score is built on a **page-defect backbone** rather than a count of issues. This is the single
most important thing to understand about it, because it is what makes the number stable.

Counting findings does not work. A 5,000-page site with one templated flaw generates 5,000
findings and scores zero; a 10-page site with three catastrophic problems generates 3 and scores
well. Both answers are wrong. So each check is classified by **what it does to the page**:

| Impact | Meaning | Weight |
|---|---|---|
| `PageBroken` | Google drops or refuses to index the page | 100 |
| `PageDefective` | The page is materially defective for ranking | 55 |
| `SiteHigh` | A real site- or link-level fault, but the page is not broken | 12 (capped at 24) |
| `SiteMedium` | Secondary technical problems | 4 (capped at 14) |
| `Minor` | Real but small | 1 (capped at 12) |
| `Unscored` | Reported, never scored | 0 |

A broken page costs its full share of the site. That matches the one published industry formula —
Ahrefs' health score tracks the share of pages without errors — so a site where 20% of pages 404
loses roughly 20 points from that term alone, which is the intuition most SEOs already have.

### Why the caps exist

Every real site has some long meta descriptions and some imperfect heading order. Without a
collective cap, those near-universal findings accumulate until a well-built site scores like a
broken one. Capping each tier's total contribution is what keeps the minor stuff visible in the
report without letting it dominate the number.

### Proportion, with two guards

Page-level deductions are proportional to the share of the site affected — 15 pages missing a
description means something different on a 15-page site than on a 1,000-page one. Two guards keep
that honest at the extremes:

- A **coverage floor** (0.12) so a single instance still registers on a large site rather than
  rounding to nothing.
- A **minimum denominator** (5 pages) so one bad page on a three-page crawl is not scored as "100%
  of the site is broken".

---

## What is deliberately not scored

**Security headers do not affect the score.** Missing HSTS or CSP is a real finding and Slither
reports it, but it is a security concern, not a ranking factor, and letting it move an SEO score
would misrepresent both.

**AI-crawler blocking does not affect the score.** Blocking GPTBot or setting `noai` is a business
decision. Slither reports it without an opinion.

**Findings on non-indexable pages are mostly gated off.** A page excluded from the index cannot
cause a duplicate-title problem.

---

## When the score is withheld

A crawl that reached no pages, or was blocked, does not receive a grade. Slither refuses to score
rather than invent a number from an empty crawl — a 0-page crawl rendering a confident all-green
chart was a real defect, and the fix was to say "unscorable" instead.

---

## How to read it

The score is a **triage signal for one site over time**, not a benchmark between sites. It answers
"is this site healthier than it was last month, and where should I look first?" It does not answer
"is this site better than a competitor", because it has no access to backlinks, rankings, traffic,
intent or content quality — the things that actually decide competitive outcomes.

Treat the category breakdown as the real output and the number as the headline. Two sites can
score 83 for entirely different reasons, and the reasons are what you act on.
