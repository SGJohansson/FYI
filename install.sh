#!/bin/sh
# fyi installer
# Script by S.G.Johansson <s.johansson.it@gmail.com>
# https://voidflow.tech/
# Apache License 2.0
#
# DESCRIPTION:
# Installs the latest prebuilt, fully static fyi binary for this machine into
# ~/.local/bin (override with FYI_INSTALL_DIR). The download is verified
# against its published SHA-256. If no prebuilt binary fits this machine and
# Rust is installed, fyi is built from source instead.
#
# USAGE:
#   curl -fsSL https://raw.githubusercontent.com/SGJohansson/FYI/main/install.sh | sh

set -eu

REPO="SGJohansson/FYI"
DIR="${FYI_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'fyi-install: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || die "fyi runs on Linux and WSL only."

case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) ARCH="" ;;
esac

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        return 1
    fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$DIR"
installed=""

if [ -n "$ARCH" ]; then
    asset="fyi-$ARCH-linux-musl"
    base="https://github.com/$REPO/releases/latest/download"
    say "downloading $asset"
    if fetch "$base/$asset" "$tmp/fyi" && fetch "$base/$asset.sha256" "$tmp/fyi.sha256"; then
        expected=$(cut -d' ' -f1 "$tmp/fyi.sha256")
        actual=$(sha256sum "$tmp/fyi" | cut -d' ' -f1)
        [ "$expected" = "$actual" ] || die "checksum mismatch for $asset, aborting."
        install -m 755 "$tmp/fyi" "$DIR/fyi"
        installed=1
    else
        say "no prebuilt binary available, trying to build from source"
    fi
fi

if [ -z "$installed" ]; then
    command -v cargo >/dev/null 2>&1 ||
        die "no prebuilt binary for $(uname -m) and Rust is not installed (https://rustup.rs)."
    cargo install --locked --git "https://github.com/$REPO" --root "$tmp/root"
    install -m 755 "$tmp/root/bin/fyi" "$DIR/fyi"
fi

say "installed $("$DIR/fyi" --version) to $DIR/fyi"
case ":$PATH:" in
    *":$DIR:"*) ;;
    *) say "note: $DIR is not in your PATH; add it to your shell profile." ;;
esac
