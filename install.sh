#!/bin/sh
# fyi installer: installs fyi as the command `lsi`
# Script by S.G.Johansson <s.johansson.it@gmail.com>
# https://voidflow.tech/
# Apache License 2.0
#
# DESCRIPTION:
# Downloads the prebuilt, fully static fyi binary for this machine from the
# GitHub release, verifies its SHA-256 and installs it as `lsi`.
#
#   as a user        ~/.local/bin/lsi     log: ~/.local/state/fyi/install.log
#   as root / -s     /usr/local/bin/lsi   log: /var/log/fyi-install.log
#
# Missing directories are created. An existing lsi is never replaced without
# asking (or -f). Nothing else is moved or touched. -u removes only an lsi this
# script installed and that is unchanged since. Every action is logged.
#
# USAGE:
#   wget2 https://raw.githubusercontent.com/SGJohansson/FYI/main/install.sh
#   chmod +x install.sh
#   ./install.sh            # for this user
#   sudo ./install.sh       # system-wide
#   sudo ./install.sh -u    # uninstall the system-wide copy

set -eu

REPO="SGJohansson/FYI"
CMD="lsi"
PROG="${0##*/}"

usage() {
    cat <<EOF
Usage: $PROG [-s] [-u] [-f] [-y] [-v TAG] [-l DIR] [-h]

Install fyi as '$CMD'.
  (no option)  for this user: ~/.local/bin/$CMD
  -s           system-wide: /usr/local/bin/$CMD (needs root; implied by sudo)
  -u           uninstall what this script installed
  -f           replace an existing $CMD (or remove a changed one with -u) without asking
  -y           answer yes to questions
  -v TAG       install this release (e.g. v0.3.0) instead of the latest
  -l DIR       install from DIR holding fyi-<arch>-linux-musl and its .sha256
               (no download; for offline machines)
  -h           this help

Missing directories are created. Everything is logged:
  user: \${XDG_STATE_HOME:-~/.local/state}/fyi/install.log
  root: /var/log/fyi-install.log
EOF
}

ARGS="$*"
SYSTEM=0 UNINSTALL=0 FORCE=0 YES=0 TAG="" LOCAL=""
while getopts "sufyv:l:h" opt; do
    case $opt in
        s) SYSTEM=1 ;;
        u) UNINSTALL=1 ;;
        f) FORCE=1 ;;
        y) YES=1 ;;
        v) TAG=$OPTARG ;;
        l) LOCAL=$OPTARG ;;
        h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done
shift $((OPTIND - 1))
[ $# -eq 0 ] || { usage >&2; exit 2; }

say() { printf '%s: %s\n' "$PROG" "$*"; }
die() { printf '%s: %s\n' "$PROG" "$*" >&2; log "ERROR $*"; exit 1; }

ROOT=0
[ "$(id -u)" -eq 0 ] && ROOT=1
if [ "$SYSTEM" -eq 1 ] && [ "$ROOT" -eq 0 ]; then
    printf '%s: -s installs to /usr/local/bin and needs root: sudo %s\n' "$PROG" "$0" >&2
    exit 1
fi
[ "$ROOT" -eq 1 ] && SYSTEM=1

if [ "$SYSTEM" -eq 1 ]; then
    BIN_DIR=/usr/local/bin
    LOG=/var/log/fyi-install.log
else
    BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
    LOG="${XDG_STATE_HOME:-$HOME/.local/state}/fyi/install.log"
fi
TARGET="$BIN_DIR/$CMD"
LOG_READY=0

# Traditional log: one line per action.
log() {
    [ "$LOG_READY" -eq 1 ] || return 0
    printf '%s %s %s[%s]: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$(id -un)" "$PROG" "$$" "$*" >>"$LOG"
}

mkdir_logged() {
    [ -d "$1" ] && return 0
    mkdir -p "$1" || die "cannot create $1"
    say "created $1"
    MADE="${MADE:-} $1"
}

MADE=""
mkdir_logged "$(dirname "$LOG")"
touch "$LOG" 2>/dev/null || { printf '%s: cannot write log %s\n' "$PROG" "$LOG" >&2; exit 1; }
LOG_READY=1
log "START $PROG${ARGS:+ $ARGS} mode=$([ "$SYSTEM" -eq 1 ] && echo system || echo user) target=$TARGET"
for d in $MADE; do log "MKDIR $d"; done
MADE=""

sha() { sha256sum "$1" | cut -d' ' -f1; }

ver() { "$1" --version 2>/dev/null | awk '{print $2}' || true; }

# Ask a yes/no question on the terminal; no terminal means no.
ask() {
    [ "$YES" -eq 1 ] && return 0
    if ( : </dev/tty ) 2>/dev/null; then
        printf '%s: %s [y/N] ' "$PROG" "$1" >/dev/tty
        read -r ans </dev/tty || ans=""
        case $ans in [yY] | [yY][eE][sS]) return 0 ;; esac
    fi
    return 1
}

# ---- uninstall ---------------------------------------------------------------

if [ "$UNINSTALL" -eq 1 ]; then
    if [ ! -e "$TARGET" ]; then
        say "$TARGET is not installed"
        log "UNINSTALL nothing at $TARGET"
        exit 0
    fi
    cur=$(sha "$TARGET")
    if grep -q " INSTALL $TARGET .*sha256=$cur\$" "$LOG" 2>/dev/null; then
        rm -f "$TARGET" || die "cannot remove $TARGET"
        say "removed $TARGET"
        log "REMOVE $TARGET sha256=$cur"
    elif [ "$FORCE" -eq 1 ] || ask "$TARGET was not installed by this script or has changed. Remove it anyway?"; then
        rm -f "$TARGET" || die "cannot remove $TARGET"
        say "removed $TARGET (forced)"
        log "REMOVE $TARGET sha256=$cur forced"
    else
        say "left $TARGET alone: not installed by this script, or changed since (use -f to remove)"
        log "KEEP $TARGET sha256=$cur not ours"
        exit 1
    fi
    say "history in ~/.local/state/fyi and this log ($LOG) are kept"
    exit 0
fi

# ---- install -----------------------------------------------------------------

[ "$(uname -s)" = Linux ] || die "fyi runs on Linux and WSL only"
case "$(uname -m)" in
    x86_64 | amd64) ARCH=x86_64 ;;
    aarch64 | arm64) ARCH=aarch64 ;;
    *) ARCH="" ;;
esac
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget2 >/dev/null 2>&1; then
        wget2 -q -O "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        die "curl, wget2 or wget is required"
    fi
}

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM
NEW="$TMP/$CMD"
SRC=""

if [ -n "$ARCH" ]; then
    asset="fyi-$ARCH-linux-musl"
    if [ -n "$LOCAL" ]; then
        [ -f "$LOCAL/$asset" ] && [ -f "$LOCAL/$asset.sha256" ] ||
            die "$LOCAL must hold $asset and $asset.sha256"
        cp "$LOCAL/$asset" "$NEW" && cp "$LOCAL/$asset.sha256" "$TMP/sum"
        SRC="$LOCAL/$asset"
    else
        if [ -n "$TAG" ]; then
            base="https://github.com/$REPO/releases/download/$TAG"
        else
            base="https://github.com/$REPO/releases/latest/download"
        fi
        say "downloading $asset${TAG:+ ($TAG)}"
        if fetch "$base/$asset" "$NEW" && fetch "$base/$asset.sha256" "$TMP/sum"; then
            SRC="$base/$asset"
        else
            rm -f "$NEW"
            say "no prebuilt binary available"
        fi
    fi
    if [ -n "$SRC" ]; then
        want=$(cut -d' ' -f1 "$TMP/sum")
        got=$(sha "$NEW")
        [ "$want" = "$got" ] || die "checksum mismatch for $SRC (expected $want, got $got)"
        log "VERIFIED $SRC sha256=$got"
    fi
fi

if [ -z "$SRC" ]; then
    [ -z "$LOCAL" ] || die "no prebuilt binary for $(uname -m) in $LOCAL"
    command -v cargo >/dev/null 2>&1 ||
        die "no prebuilt binary for $(uname -m) and Rust is not installed (https://rustup.rs)"
    say "building from source with cargo"
    cargo install --locked --git "https://github.com/$REPO" ${TAG:+--tag "$TAG"} --root "$TMP/root" ||
        die "cargo build failed"
    cp "$TMP/root/bin/$CMD" "$NEW"
    SRC="source build${TAG:+ $TAG}"
    log "BUILT from source sha256=$(sha "$NEW")"
fi
chmod 755 "$NEW"
NEWVER=$(ver "$NEW")
[ -n "$NEWVER" ] || die "the new binary does not run on this machine"
NEWSUM=$(sha "$NEW")

if [ -e "$TARGET" ]; then
    if cmp -s "$NEW" "$TARGET"; then
        say "$TARGET is already $NEWVER, nothing to do"
        log "UNCHANGED $TARGET $NEWVER sha256=$NEWSUM"
        exit 0
    fi
    OLDVER=$(ver "$TARGET")
    OLDVER=${OLDVER:-unknown version}
    if [ "$FORCE" -eq 1 ] || ask "replace $TARGET ($OLDVER) with $NEWVER?"; then
        log "REPLACE $TARGET $OLDVER sha256=$(sha "$TARGET")"
    else
        say "kept $TARGET ($OLDVER); run again with -f, or answer y, to replace it"
        log "KEEP $TARGET $OLDVER (not replaced by $NEWVER)"
        exit 1
    fi
fi

mkdir_logged "$BIN_DIR"
for d in $MADE; do log "MKDIR $d"; done
# Copy next to the target, then rename: never a half-written lsi.
cp "$NEW" "$TARGET.new.$$" && chmod 755 "$TARGET.new.$$" && mv -f "$TARGET.new.$$" "$TARGET" ||
    { rm -f "$TARGET.new.$$"; die "cannot write $TARGET"; }
say "installed $CMD $NEWVER to $TARGET"
log "INSTALL $TARGET $NEWVER from $SRC sha256=$NEWSUM"

# ---- hints (nothing below changes anything) ---------------------------------

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        say "note: $BIN_DIR is not in your PATH; add it to your shell profile"
        log "NOTE $BIN_DIR not in PATH"
        ;;
esac

old=""
IFS=:
for d in $PATH; do
    for n in fyi "$CMD"; do
        p="$d/$n"
        [ "$p" = "$TARGET" ] && continue
        [ -e "$p" ] && case " $old " in *" $p "*) ;; *) old="$old $p" ;; esac
    done
done
unset IFS
for p in $old; do
    say "note: older copy left untouched: $p"
    log "NOTE other copy $p"
done

say "optional, for wcd/wcp, add to ~/.bashrc: command -v $CMD >/dev/null && eval \"\$($CMD --init bash)\""
log "DONE"
