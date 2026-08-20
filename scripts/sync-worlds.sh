#!/usr/bin/env bash
# Refresh the world corpus from the registry, which is what the internet
# actually serves.
#
# The corpus in worlds/ is not decoration: the conformance suite is developed
# against it, so a fixture that disagrees with the real world quietly redefines
# what "conformant" means. That happened — for weeks the nexus fixture routed
# four veils at hostnames that had never existed, while the live Nexus had been
# re-addressed. Nobody noticed, because nothing compared the two.
#
#   ./scripts/sync-worlds.sh            # rewrite worlds/ from the registry
#   ./scripts/sync-worlds.sh --check    # exit 1 if any fixture has drifted
#
# Worlds that are not registered (local-only test fixtures) are left alone.

set -euo pipefail
cd "$(dirname "$0")/.."

API="${THREAD_REGISTRY:-https://api.pixygon.io/v1/thread}"
CHECK=0
[[ "${1:-}" == "--check" ]] && CHECK=1

command -v jq >/dev/null || { echo "sync-worlds: needs jq" >&2; exit 2; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

echo "→ listing registered worlds from $API"
curl -fsS "$API/worlds" -o "$tmp/worlds.json"
ids=$(jq -r 'if type=="array" then .[] else (.worlds//.items//[])[] end | .worldId // .id' "$tmp/worlds.json")
[[ -n "$ids" ]] || { echo "sync-worlds: registry returned no worlds" >&2; exit 2; }

# Index local fixtures by the world id inside the manifest, not by directory
# name — several directories are named for their host instead.
declare -A local_dir
for f in worlds/*/world.json; do
  id=$(jq -r '.world.id // empty' "$f" 2>/dev/null || true)
  [[ -n "$id" ]] && local_dir["$id"]="$(dirname "$f")"
done

drifted=0 updated=0 skipped=0
for wid in $ids; do
  curl -fsS "$API/manifest/$wid" -o "$tmp/$wid.json" 2>/dev/null || { echo "  · $wid — registry has no manifest, skipping"; continue; }
  id=$(jq -r '.world.id // empty' "$tmp/$wid.json")
  dir="${local_dir[$id]:-}"
  if [[ -z "$dir" ]]; then
    echo "  · $wid ($id) — no local fixture, skipping"
    skipped=$((skipped+1))
    continue
  fi
  # Compare as data, not as text: key order and whitespace are not drift.
  if jq -S . "$tmp/$wid.json" | diff -q - <(jq -S . "$dir/world.json") >/dev/null 2>&1; then
    continue
  fi
  drifted=$((drifted+1))
  if [[ $CHECK -eq 1 ]]; then
    echo "  ✗ $dir has drifted from the registry ($wid)"
    jq -S . "$tmp/$wid.json" | diff -u <(jq -S . "$dir/world.json") - | head -30 || true
  else
    jq . "$tmp/$wid.json" > "$dir/world.json"
    echo "  ✓ $dir updated from $wid"
    updated=$((updated+1))
  fi
done

if [[ $CHECK -eq 1 ]]; then
  if [[ $drifted -gt 0 ]]; then
    echo
    echo "$drifted fixture(s) disagree with the registry. Run ./scripts/sync-worlds.sh and commit."
    exit 1
  fi
  echo "✓ every registered world's fixture matches what the registry serves"
else
  echo
  echo "updated $updated, unregistered/local-only $skipped"
fi
