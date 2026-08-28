# Contributing to Slither

Thanks for looking. Slither is an SEO audit tool, which means its whole value is that its output is
**true**. A tool that confidently reports a problem that isn't there wastes someone's afternoon and
loses their trust permanently. Most of the guidance below follows from that.

## Getting set up

```bash
git clone https://github.com/nmang004/Slither.git
cd Slither
cargo build --release
cargo test --workspace
```

You need a stable Rust toolchain (edition 2021, rustc ≥ 1.80). No other services are required —
the default build reaches nothing external, and the test suite runs offline.

Before opening a pull request:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

If you touched anything feature-gated, check it still builds both ways:

```bash
cargo check --workspace --all-targets --no-default-features
cargo check --workspace --all-targets --features cloudflare
cargo check --workspace --all-targets --features playwright
```

## The most valuable bug report

**False positives.** A check that fires on a page that is actually fine is worse than a check that
doesn't exist. Every one found so far came from someone running Slither against a real site and not
believing the output — for example, three pages reported as having no `<title>` that all plainly
had one, because invalid markup in `<head>` had moved the title into `<body>`.

If you hit one, the most useful report includes the URL (or a snippet of markup that reproduces it),
what Slither said, and what's actually true. A failing test case is even better.

## Writing checks and fixes

**Verify against a real fixture, not by reading the code.** Stand up a local site
(`python3 -m http.server` or the fixture helpers in `slither-core/tests/`) and run the actual binary
against it. `SLITHER_ALLOW_PRIVATE_TARGETS=1` is required to crawl loopback addresses.

**Prove your test catches the bug.** Revert your fix, confirm the new test fails, then restore it.
This sounds pedantic and it is not: a "regression test" that passes against the unfixed code is
worse than none, and we have shipped one. If a test asserts on a rendered report, make sure the
assertion targets the specific thing you fixed — a string can appear in five places and only one of
them exercises your code path.

**Never weaken a test to make it pass.** If a test fails after your change, either the change is
wrong or the test encoded the old behaviour. Both happen. Fix the right one, and say in the PR which
it was.

**Find every consumer of what you changed.** The recurring failure mode in this codebase has been
fixing a computation and leaving its readers alone — a guard added to a library while the only
caller kept using the unguarded helper, a value's meaning changed while four joins still compared
the old thing. `grep -rn` for every reader before you call a fix done.

**Say what a check cannot know.** Several checks are conditional on crawl coverage — "orphan page"
means "nothing in *this crawl* links here". Where that matters, the docs say so, and new checks
should do the same rather than implying more certainty than they have.

## Code style

Match the surrounding code. A few conventions worth knowing:

- **Comments explain *why*, not *what*.** The code says what it does. The comment should say why it
  is that way — usually the bug that made it necessary, concretely. `// strip the fragment` is
  noise; explaining that fragments were becoming page identity and putting `#toc` into sitemaps is
  useful to the next person.
- Prefer a named helper over a clever one-liner when a reader would otherwise have to reconstruct
  the reasoning.
- Keep MCP and report output **bounded**. Any list that grows with crawl size needs a cap and a
  reported true total. A tool that returns 300k tokens is unusable to the audience Slither is for.

## Commits and pull requests

Conventional commit prefixes (`fix:`, `feat:`, `docs:`, `refactor:`, `test:`), with a scope where
it helps (`fix(crawler):`). Write the body for someone who will read it in a year while bisecting:
what was wrong, what the user-visible consequence was, and what changed.

Keep pull requests focused. A PR that fixes one thing and is easy to verify will be merged faster
than one that fixes four.

If your change alters behaviour a user could notice, add a line to `CHANGELOG.md` under
`Unreleased`, describing the effect rather than the implementation.

## Reporting security issues

Please don't open a public issue. See [SECURITY.md](SECURITY.md).
