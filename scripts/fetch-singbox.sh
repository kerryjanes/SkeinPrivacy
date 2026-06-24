#!/usr/bin/env bash
# Fetch the sing-box core binary for one or more Rust target triples and place it where Tauri
# expects a sidecar: clients/desktop/src-tauri/binaries/sing-box-<triple>[.exe].
# sing-box is the engine V2Box/Happ/Hiddify wrap; the desktop app bundles + manages it.
#
# Usage:
#   scripts/fetch-singbox.sh                 # host triple (rustc --print host-tuple)
#   scripts/fetch-singbox.sh <triple> [...]  # specific triples
#   SINGBOX_VERSION=1.13.13 scripts/fetch-singbox.sh
set -euo pipefail

VERSION="${SINGBOX_VERSION:-1.13.13}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/clients/desktop/src-tauri/binaries"
mkdir -p "$OUT"

triples=("$@")
if [ ${#triples[@]} -eq 0 ]; then
  triples=("$(rustc --print host-tuple)")
fi

# Map a Rust target triple → the sing-box release asset's os/arch slug.
asset_for() {
  case "$1" in
    aarch64-apple-darwin)        echo "darwin-arm64 tar.gz" ;;
    x86_64-apple-darwin)         echo "darwin-amd64 tar.gz" ;;
    x86_64-unknown-linux-gnu)    echo "linux-amd64 tar.gz" ;;
    aarch64-unknown-linux-gnu)   echo "linux-arm64 tar.gz" ;;
    x86_64-pc-windows-msvc|x86_64-pc-windows-gnu) echo "windows-amd64 zip" ;;
    *) echo "" ;;
  esac
}

for triple in "${triples[@]}"; do
  mapping="$(asset_for "$triple")"
  if [ -z "$mapping" ]; then
    echo "fetch-singbox: unsupported triple '$triple'" >&2
    exit 2
  fi
  slug="${mapping% *}"; ext="${mapping#* }"
  base="sing-box-${VERSION}-${slug}"
  url="https://github.com/SagerNet/sing-box/releases/download/v${VERSION}/${base}.${ext}"
  tmp="$(mktemp -d)"
  echo "fetch-singbox: $triple ← $url"
  curl -fsSL "$url" -o "$tmp/pkg.$ext"

  bin="sing-box"; suffix=""
  case "$triple" in *windows*) bin="sing-box.exe"; suffix=".exe" ;; esac

  if [ "$ext" = "zip" ]; then
    unzip -qo "$tmp/pkg.$ext" -d "$tmp"
  else
    tar -xzf "$tmp/pkg.$ext" -C "$tmp"
  fi
  cp "$tmp/$base/$bin" "$OUT/sing-box-${triple}${suffix}"
  chmod +x "$OUT/sing-box-${triple}${suffix}"
  rm -rf "$tmp"
  echo "fetch-singbox: → $OUT/sing-box-${triple}${suffix}"
done
