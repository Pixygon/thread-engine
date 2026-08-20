#!/bin/sh
# Install the Thread toolkit — `thread` and `thread-conformance` — from the
# latest GitHub release. No Rust required.
#
#   curl -fsSL https://raw.githubusercontent.com/Pixygon/thread-engine/main/install.sh | sh
#
# Env:
#   THREAD_INSTALL_DIR   where to put the binaries (default ~/.local/bin)
#   THREAD_VERSION       a release tag, e.g. v0.2.0 (default: latest)

set -eu

REPO=Pixygon/thread-engine
DEST="${THREAD_INSTALL_DIR:-$HOME/.local/bin}"

say()  { printf '%s\n' "$*"; }
die()  { printf 'install: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "this script needs '$1'"; }

need uname
need mktemp
need tar
if command -v curl >/dev/null 2>&1; then fetch() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then fetch() { wget -qO- "$1"; }
else die "this script needs curl or wget"
fi

# --- which build do you need -------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  case "$arch" in
            x86_64|amd64)  target=x86_64-unknown-linux-musl ;;   # static: runs anywhere
            aarch64|arm64) target=aarch64-unknown-linux-gnu ;;
            *) die "no prebuilt binary for Linux/$arch — install with: cargo install thread-cli thread-conformance" ;;
          esac ;;
  Darwin) case "$arch" in
            arm64)  target=aarch64-apple-darwin ;;
            x86_64) target=x86_64-apple-darwin ;;
            *) die "no prebuilt binary for macOS/$arch" ;;
          esac ;;
  *) die "unsupported platform '$os' — on Windows download the .zip from https://github.com/$REPO/releases" ;;
esac

# --- which release -----------------------------------------------------------
if [ -n "${THREAD_VERSION:-}" ]; then
  api="https://api.github.com/repos/$REPO/releases/tags/$THREAD_VERSION"
else
  api="https://api.github.com/repos/$REPO/releases/latest"
fi

say "→ looking up the latest release for $target"
assets="$(fetch "$api" | grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' | cut -d'"' -f4)" \
  || die "could not reach the GitHub API"
[ -n "$assets" ] || die "that release has no downloadable assets yet"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
installed=""

for tool in thread thread-conformance; do
  # `thread-conformance-v…` also matches a bare `thread-v…` prefix search, so
  # anchor on the version marker to keep the two tools apart.
  url="$(printf '%s\n' "$assets" | grep -E "/${tool}-v[0-9][^/]*-${target}\.tar\.gz$" | head -n1 || true)"
  [ -n "$url" ] || { say "  ! no $tool build for $target in this release — skipping"; continue; }

  file="${url##*/}"
  say "→ downloading $file"
  fetch "$url" > "$tmp/$file" || die "download failed: $url"

  # Verify if the release published a checksum and we have a tool to check it.
  sumurl="$(printf '%s\n' "$assets" | grep -F "/$file.sha256" | head -n1 || true)"
  if [ -n "$sumurl" ]; then
    if command -v sha256sum >/dev/null 2>&1; then sum=$(sha256sum "$tmp/$file" | cut -d' ' -f1)
    elif command -v shasum   >/dev/null 2>&1; then sum=$(shasum -a 256 "$tmp/$file" | cut -d' ' -f1)
    else sum=""; fi
    if [ -n "$sum" ]; then
      want="$(fetch "$sumurl" | cut -d' ' -f1)"
      [ "$sum" = "$want" ] || die "checksum mismatch for $file — refusing to install"
      say "  ✓ checksum ok"
    fi
  fi

  tar -xzf "$tmp/$file" -C "$tmp"
  mkdir -p "$DEST"
  cp "$tmp/${file%.tar.gz}/$tool" "$DEST/$tool"
  chmod +x "$DEST/$tool"
  installed="$installed $tool"
done

[ -n "$installed" ] || die "nothing was installed"

say ""
say "installed:$installed → $DEST"
case ":$PATH:" in
  *":$DEST:"*) ;;
  *) say ""
     say "$DEST is not on your PATH. Add it:"
     say "  echo 'export PATH=\"\$PATH:$DEST\"' >> ~/.profile && . ~/.profile" ;;
esac
say ""
say "Try it:"
say "  thread init my-world && thread validate my-world/world.json"
say "  thread-conformance --live pixygon.io"
