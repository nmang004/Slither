#!/bin/sh
# Slither SEO toolkit installer
# Usage: curl -fsSL https://raw.githubusercontent.com/nmang004/Slither/main/install.sh | sh

set -e

REPO="nmang004/Slither"
RELEASE_URL="https://github.com/${REPO}/releases"
BUILD_HELP="Build from source instead: https://github.com/${REPO}#install"

unsupported() {
    echo "Error: $1"
    echo "$BUILD_HELP"
    exit 1
}

# Detect OS
case "$(uname -s)" in
    Linux*)  OS="linux" ;;
    Darwin*) OS="macos" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *) unsupported "unsupported operating system: $(uname -s)" ;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)  ARCH="aarch64" ;;
    *) unsupported "unsupported architecture: $(uname -m)" ;;
esac

# Only these combinations are published by the release workflow. Without this
# check the script builds a download URL for, say, linux-aarch64 and fails on a
# 404 that looks like the release is broken rather than the platform being
# unbuilt.
case "${OS}-${ARCH}" in
    linux-x86_64|macos-aarch64|macos-x86_64|windows-x86_64) ;;
    *) unsupported "no prebuilt binary for ${OS}-${ARCH} yet" ;;
esac

# Resolve the version to install. Pinning a version in this script means a
# cached or forked copy silently installs an old release forever, so the default
# is whatever the latest published release is. Override with SLITHER_VERSION.
resolve_latest_tag() {
    # `/releases/latest` redirects to `/releases/tag/<TAG>`; reading the final
    # URL avoids both jq and the API's unauthenticated rate limit.
    if command -v curl > /dev/null 2>&1; then
        curl -fsSLI -o /dev/null -w '%{url_effective}' "${RELEASE_URL}/latest" 2>/dev/null |
            sed 's#.*/tag/##'
    elif command -v wget > /dev/null 2>&1; then
        wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null |
            sed -n 's/.*"tag_name":[ ]*"\([^"]*\)".*/\1/p' | head -1
    fi
}

if [ -n "${SLITHER_VERSION:-}" ]; then
    # Accept both "0.3.0" and "v0.3.0".
    case "$SLITHER_VERSION" in
        v*) TAG="$SLITHER_VERSION" ;;
        *)  TAG="v${SLITHER_VERSION}" ;;
    esac
else
    TAG="$(resolve_latest_tag)"
    [ -n "$TAG" ] || unsupported "could not determine the latest release"
fi
VERSION="${TAG#v}"

# Release artifacts are named with the v-prefixed git tag, e.g.
# slither-v0.3.0-macos-aarch64, matching the release workflow.
if [ "$OS" = "windows" ]; then
    FILENAME="slither-${TAG}-${OS}-${ARCH}.exe"
else
    FILENAME="slither-${TAG}-${OS}-${ARCH}"
fi
URL="${RELEASE_URL}/download/${TAG}/${FILENAME}"
SUMS_URL="${RELEASE_URL}/download/${TAG}/SHA256SUMS"

# Determine install directory
if [ "$OS" = "windows" ]; then
    INSTALL_DIR="${LOCALAPPDATA:-$HOME/AppData/Local}/slither/bin"
else
    INSTALL_DIR="${HOME}/.local/bin"
fi

echo "Slither installer"
echo ""
echo "  Platform:  ${OS}-${ARCH}"
echo "  Version:   ${VERSION}"
echo "  Install:   ${INSTALL_DIR}/slither"
echo ""

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download to a temp file so a failed/partial download never lands on PATH.
TMP="$(mktemp)"
trap 'rm -f "$TMP" "$TMP.sums"' EXIT

fetch() {
    if command -v curl > /dev/null 2>&1; then
        curl -fSL --proto '=https' --tlsv1.2 "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget -q "$1" -O "$2"
    else
        echo "Error: curl or wget is required"
        exit 1
    fi
}

echo "Downloading..."
fetch "$URL" "$TMP"

# Verify the checksum against the release SHA256SUMS. The download is aborted
# if the sum is present but does not match; a missing SHA256SUMS is a hard
# warning rather than a silent skip.
if fetch "$SUMS_URL" "$TMP.sums" 2>/dev/null; then
    EXPECTED="$(grep " ${FILENAME}\$" "$TMP.sums" 2>/dev/null | awk '{print $1}')"
    if [ -n "$EXPECTED" ]; then
        if command -v sha256sum > /dev/null 2>&1; then
            ACTUAL="$(sha256sum "$TMP" | awk '{print $1}')"
        else
            ACTUAL="$(shasum -a 256 "$TMP" | awk '{print $1}')"
        fi
        if [ "$EXPECTED" != "$ACTUAL" ]; then
            echo "Error: checksum mismatch for ${FILENAME}"
            echo "  expected: ${EXPECTED}"
            echo "  actual:   ${ACTUAL}"
            exit 1
        fi
        echo "Checksum verified."
    else
        echo "Warning: ${FILENAME} not listed in SHA256SUMS; skipping verification."
    fi
else
    echo "Warning: SHA256SUMS not available; skipping checksum verification."
fi

# Install atomically now that the file is verified.
mv "$TMP" "${INSTALL_DIR}/slither"
trap - EXIT
rm -f "$TMP.sums"
chmod +x "${INSTALL_DIR}/slither"

echo ""
echo "Installed slither to ${INSTALL_DIR}/slither"
echo ""

# Check PATH
case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        echo "Run 'slither' to get started."
        echo "Run 'slither setup' to install Python analysis tools."
        ;;
    *)
        echo "Add to your PATH:"
        echo ""
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "Then run 'slither setup' to install Python analysis tools."
        ;;
esac
