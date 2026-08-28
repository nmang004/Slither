# Releasing and distribution

## Cutting a release

Everything is driven by a `v*` tag. There is no manual build step.

```bash
# 1. Make sure the version is right and the tree is green
grep '^version' Cargo.toml
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings

# 2. Tag and push
git tag v0.3.0
git push origin v0.3.0
```

`.github/workflows/release.yml` then builds four binaries, generates `SHA256SUMS`, and publishes a
GitHub release with generated notes.

| Artifact | Target | Notes |
|---|---|---|
| `slither-v0.3.0-linux-x86_64` | `x86_64-unknown-linux-musl` | Statically linked; runs on any distro |
| `slither-v0.3.0-macos-aarch64` | `aarch64-apple-darwin` | Apple Silicon |
| `slither-v0.3.0-macos-x86_64` | `x86_64-apple-darwin` | Intel Macs |
| `slither-v0.3.0-windows-x86_64.exe` | `x86_64-pc-windows-msvc` | |

Builds use `--locked`, so a published binary is built from the committed lockfile rather than a
silently newer dependency graph. Third-party actions are pinned by commit SHA, and write access is
scoped to the release job so the build matrix runs read-only.

### Why Linux is musl

A `x86_64-unknown-linux-gnu` build links against the *runner's* glibc. The GitHub runner is well
ahead of most deployed distributions, so that binary fails to start on anything older with
`GLIBC_2.x not found` — a confusing error that reads like a broken release. The musl target links
statically and runs anywhere.

This is only possible because nothing in the tree needs a system C library: `reqwest` uses
`rustls` rather than `native-tls`, and `rusqlite` is `bundled` (SQLite compiled from vendored
source). If either changes, revisit this.

## The install script

```bash
curl -fsSL https://raw.githubusercontent.com/nmang004/Slither/main/install.sh | sh
```

`install.sh` resolves the **latest** release rather than a pinned version, so a cached or forked
copy of the script cannot silently install an old build forever. Override with:

```bash
SLITHER_VERSION=0.3.0 sh install.sh
```

It downloads to a temp file, verifies against the release `SHA256SUMS`, and only then moves the
binary into `~/.local/bin`. A checksum mismatch aborts; a missing `SHA256SUMS` is a loud warning
rather than a silent skip.

Platform combinations that are not built are refused up front with a message pointing at the
build-from-source instructions, rather than producing a 404 that looks like a broken release.
Linux on ARM is currently in that category.

## Homebrew

A formula lives at [`packaging/homebrew/slither-seo.rb`](../packaging/homebrew/slither-seo.rb). It is
not published yet; the header of that file has the steps.

Two naming facts worth knowing before choosing:

- **`slither` already means something in homebrew-core:** `slither-analyzer`, the Solidity static
  analyzer, installs an executable called `slither`. A tap formula named plain `slither` would
  invite confusion with it, so the formula is named `slither-seo`, while the installed command
  stays `slither`.
- **homebrew-core will not accept a brand-new project.** It has notability requirements
  (stars/forks/watchers) that a fresh repo does not meet. A personal tap
  (`github.com/nmang004/homebrew-tap`) works immediately and needs nobody's approval, so that is
  the realistic path.

Homebrew also solves a problem the raw download has: a binary downloaded through a **browser** gets
macOS's quarantine attribute, and an unsigned binary then fails Gatekeeper with "cannot be opened
because the developer cannot be verified". Homebrew strips that attribute. (Binaries fetched by
`curl`, including through `install.sh`, are never quarantined, so the one-line installer is
unaffected.)

## Other registries

`cargo install` is not currently a supported path, and the obvious names are taken by unrelated
crates:

| Name | crates.io |
|---|---|
| `slither` | Taken — a public-transit routing library |
| `slither-cli` | Taken — a TOTP authenticator tool |
| `slither-seo` | Free |
| `slither-core` | Free |

This matters more than it looks: telling a user to run `cargo install slither-cli` would install
somebody else's unrelated program. If Slither is ever published to crates.io, `slither-seo` is the
name available for the binary crate.

## Release checklist

1. `CHANGELOG.md` — move `Unreleased` entries under the new version
2. Version in `Cargo.toml` matches the tag
3. `cargo test --workspace`, `clippy -D warnings`, `fmt --check` all clean
4. Tag `vX.Y.Z` and push
5. Confirm all four artifacts and `SHA256SUMS` attached to the release
6. Test the installer end to end on at least one machine
7. **After the first release only:** switch the README's install section from build-from-source to
   the `curl | sh` one-liner. It currently documents building from source because that is the only
   path that works until a release exists, and a README promising an installer that 404s is worse
   than one that asks for a compiler.
8. If publishing to the tap: bump `version` and the three sha256 values in the formula
