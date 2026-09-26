#!/usr/bin/env bash
# One workspace, one version — everywhere it is written down.
#
# Every crate here inherits `version.workspace = true`, but each *path
# dependency* also repeats the number (`version = "x.y.z"`). That repetition is
# load-bearing: without it a crate cannot be published to crates.io, because a
# bare path dependency means nothing to anyone who is not in this checkout.
#
# `pearl ship` stamps `[workspace.package] version` and nothing else, so the
# moment the number moves the workspace stops resolving:
#
#   error: failed to select a version for the requirement `thread-structured-id = "^0.2.2"`
#   candidate versions found which didn't match: 0.6.0
#
# That is what this script is for. `.pixygon.json` runs it as the ship's
# `postVersion` hook, after the stamp and before the tests it must pass.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(awk '/^\[workspace\.package\]/ { inside = 1; next }
               /^\[/               { inside = 0 }
               inside && /^version[[:space:]]*=/ { gsub(/[",]/, "", $3); print $3; exit }' Cargo.toml)
if [ -z "${version:-}" ]; then
    echo "✗ no [workspace.package] version in Cargo.toml" >&2
    exit 1
fi

changed=0
for f in Cargo.toml crates/*/Cargo.toml; do
    before=$(cat "$f")
    # Only lines that are a path dependency: never touch a version from
    # crates.io (serde = { version = "1", … } must stay where it is).
    sed -i -E '/path[[:space:]]*=[[:space:]]*"(\.\.|crates)\//{ s/version[[:space:]]*=[[:space:]]*"[^"]*"/version = "'"$version"'"/ }' "$f"
    if [ "$before" != "$(cat "$f")" ]; then
        echo "  $f"
        changed=$((changed + 1))
    fi
done

# The lock records every workspace member's version too; refresh it so the
# tree the ship is about to commit is one consistent thing.
cargo metadata --format-version 1 --offline >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null

echo "✓ path dependencies pinned to $version ($changed file(s) changed)"
