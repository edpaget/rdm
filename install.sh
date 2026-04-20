#!/bin/sh
# Install rdm by downloading a prebuilt binary.
#
# Usage:
#   curl -fsSL https://github.com/edpaget/rdm/releases/latest/download/install.sh | sh
#   curl -fsSL https://github.com/edpaget/rdm/releases/latest/download/install.sh | sh -s -- --version v0.6.2
#   curl -fsSL https://github.com/edpaget/rdm/releases/latest/download/install.sh | sh -s -- --dir ~/.local/bin
#
# Options:
#   --version <tag>  Install a specific release (default: latest).
#   --dir <path>     Install into <path> instead of the cargo-dist default
#                    ($CARGO_HOME/bin or ~/.cargo/bin).
#   --help           Print this help and exit.
#
# This is a thin wrapper around the cargo-dist shell installer published with
# each release. OS/arch detection and sha256 verification of the binary tarball
# are handled by that installer. Provenance of *this* script relies on GitHub
# Releases' HTTPS certificate chain — pin to a tagged release URL for
# reproducibility.

set -eu

REPO="edpaget/rdm"
VERSION=""
DIR=""

usage() {
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || { echo "error: --version requires a tag" >&2; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --dir)
            [ $# -ge 2 ] || { echo "error: --dir requires a path" >&2; exit 2; }
            DIR="$2"
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if ! command -v curl >/dev/null 2>&1; then
    echo "error: curl is required but not found on PATH" >&2
    exit 1
fi

if [ -n "$VERSION" ]; then
    URL="https://github.com/${REPO}/releases/download/${VERSION}/rdm-cli-installer.sh"
else
    URL="https://github.com/${REPO}/releases/latest/download/rdm-cli-installer.sh"
fi

echo "Downloading rdm installer from ${URL}"

if [ -n "$DIR" ]; then
    curl --proto '=https' --tlsv1.2 -fsSL "$URL" | sh -s -- --install-dir "$DIR"
else
    curl --proto '=https' --tlsv1.2 -fsSL "$URL" | sh
fi
