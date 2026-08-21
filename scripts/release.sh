#!/usr/bin/env bash
# Release the open half of the Thread — every crate, in dependency order.
#
#   ./scripts/release.sh --dry          # print the plan, publish nothing
#   ./scripts/release.sh 0.3.0          # set the version and publish
#   ./scripts/release.sh --patch        # 0.2.2 → 0.2.3
#   ./scripts/release.sh --resume       # skip what is already on crates.io
#
# Why this exists: 0.2.0 was published by hand, from a working copy, in eleven
# separate commands. Four of the crates went out carrying rustdoc links to
# `../../../docs/spec/*.md` — paths that resolved in the monorepo they were
# written in and nowhere else. The fix had already been made; it was in the
# other copy of the source. Nothing about that was a hard problem, and no
# amount of care would have caught it, because the mistake was *being in the
# wrong directory*.
#
# So: this script publishes from the repository it lives in, refuses to run on
# a dirty tree, checks what it is about to ship before it ships it, and goes in
# dependency order so a crate is never published against a version of its
# dependency that does not exist yet.
set -euo pipefail
cd "$(dirname "$0")/.."

# Dependency order. `thread-cli` last because it depends on nearly all of them;
# `thread-structured-id` first because it depends on none.
CRATES=(
  thread-structured-id
  thread-manifest
  thread-avatar
  weft-lang
  thread-chisel
  thread-relay
  thread-rendezvous
  thread-engine
  thread-conformance
  weft-pack
  thread-cli
)

BUMP=""; EXPLICIT=""; DRY=false; RESUME=false
for arg in "$@"; do
  case "$arg" in
    --patch|--minor|--major) BUMP="${arg#--}" ;;
    --dry)    DRY=true ;;
    --resume) RESUME=true ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    [0-9]*.[0-9]*.[0-9]*) EXPLICIT="$arg" ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

CURRENT="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
NEW="$CURRENT"
if [ -n "$EXPLICIT" ]; then
  NEW="$EXPLICIT"
elif [ -n "$BUMP" ]; then
  IFS=. read -r MA MI PA <<<"$CURRENT"
  case "$BUMP" in
    patch) PA=$((PA + 1)) ;;
    minor) MI=$((MI + 1)); PA=0 ;;
    major) MA=$((MA + 1)); MI=0; PA=0 ;;
  esac
  NEW="$MA.$MI.$PA"
fi

echo "▸ the open half of the Thread"
echo "  version:  $CURRENT${NEW:+ → $NEW}"
echo "  crates:   ${#CRATES[@]}, in dependency order"
echo

# ── The checks that would have caught 0.2.0 ────────────────────────────────
fail=0
note() { echo "  ✗ $1"; fail=1; }

# 1. Publishing from a dirty tree is publishing something nobody can check out.
if [ -n "$(git status --porcelain)" ] && [ "$DRY" = false ]; then
  note "the working tree is dirty — commit first, so the tag matches the bytes"
fi

# 2. A relative link that escapes the crate is dead the moment it is packaged.
#    This is the 0.2.0 bug, as a test.
if grep -rn '\]:.*\.\./\.\./' crates/*/src/*.rs 2>/dev/null | grep -q .; then
  grep -rn '\]:.*\.\./\.\./' crates/*/src/*.rs | sed 's/^/      /'
  note "doc links escape the crate root — they resolve here and nowhere else"
fi

# 3. The tree has to build and pass its own tests before anyone else gets it.
if [ "$DRY" = false ]; then
  echo "  · cargo test --workspace"
  cargo test --workspace --quiet >/dev/null 2>&1 || note "tests fail"
fi

[ "$fail" -eq 1 ] && { echo; echo "refusing to publish"; exit 1; }
echo "  ✓ tree is clean, doc links stay inside their crates, tests pass"
echo

# ── Version ────────────────────────────────────────────────────────────────
if [ "$NEW" != "$CURRENT" ]; then
  echo "▸ setting version $NEW"
  if [ "$DRY" = false ]; then
    sed -i "0,/^version = \"$CURRENT\"/s//version = \"$NEW\"/" Cargo.toml
    sed -i "s/version = \"$CURRENT\" }/version = \"$NEW\" }/g" Cargo.toml
    for f in crates/*/Cargo.toml; do
      sed -i "s/\(path = \"\.\.\/[a-z-]*\", \)version = \"$CURRENT\"/\1version = \"$NEW\"/" "$f"
    done
    cargo build --workspace --quiet
    git add -A && git commit -q -m "$NEW"
  fi
fi

# ── Publish ────────────────────────────────────────────────────────────────
for c in "${CRATES[@]}"; do
  if [ "$RESUME" = true ] && curl -sf -A "pixygon-release" \
      "https://crates.io/api/v1/crates/$c/$NEW" >/dev/null 2>&1; then
    echo "  · $c $NEW already published — skipping"
    continue
  fi
  if [ "$DRY" = true ]; then
    echo "  [--dry] cargo publish -p $c"
    continue
  fi
  echo "▸ publishing $c"
  # crates.io rate-limits *new* crates hard; an existing crate's next version
  # is not throttled, so a retry loop only matters the first time a name ships.
  for try in 1 2 3 4 5 6 7 8 9 10; do
    if out=$(cargo publish -p "$c" 2>&1); then break; fi
    if grep -q "already exists" <<<"$out"; then echo "  · already published"; break; fi
    if grep -q "429" <<<"$out"; then
      echo "  · rate-limited, waiting 2 min (attempt $try)"; sleep 120; continue
    fi
    echo "$out" | tail -5; echo "✗ $c failed"; exit 1
  done
done

echo
echo "▸ verifying what actually shipped, not what is in the tree"
for c in thread-relay thread-conformance thread-cli; do
  tmp="$(mktemp -d)"
  if curl -sfL -o "$tmp/c.crate" "https://static.crates.io/crates/$c/$c-$NEW.crate" \
     && tar xzf "$tmp/c.crate" -C "$tmp" 2>/dev/null; then
    if grep -rq '\]:.*\.\./\.\./' "$tmp/$c-$NEW/src/" 2>/dev/null; then
      echo "  ✗ $c $NEW shipped with escaping doc links"
    else
      echo "  ✓ $c $NEW — doc links resolve"
    fi
  else
    echo "  · $c $NEW not on the CDN yet (it lags a minute)"
  fi
  rm -rf "$tmp"
done

echo
echo "✓ $NEW published. Tag it so the binaries build:"
echo "    git tag v$NEW && git push origin v$NEW"
