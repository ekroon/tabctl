#!/usr/bin/env bash
# Wait for CI checks on a PR to pass, then merge it.
# Usage: scripts/ci-wait-merge.sh <pr-number> [--tag <tag>]
#
# Options:
#   --tag <tag>    After merge, create and push an annotated tag
#
# Example:
#   scripts/ci-wait-merge.sh 43 --tag v0.6.0-alpha.9
set -euo pipefail

PR="${1:?Usage: $0 <pr-number> [--tag <tag>]}"
TAG=""
shift
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

echo "⏳ Waiting for CI checks on PR #${PR}…"

# gh pr checks --watch exits 0 when all checks pass, non-zero on failure
if ! gh pr checks "$PR" --watch --fail-fast; then
  echo "✗ CI checks failed on PR #${PR}"
  exit 1
fi

echo "✓ CI checks passed on PR #${PR}"
echo "⏳ Merging PR #${PR}…"

if ! gh pr merge "$PR" --merge --delete-branch; then
  echo "✗ Merge failed"
  exit 1
fi

echo "✓ PR #${PR} merged"

# Switch to main and pull
git checkout main
git pull --ff-only origin main

if [[ -n "$TAG" ]]; then
  echo "⏳ Tagging ${TAG}…"
  git tag -a "$TAG" -m "$TAG"
  git push origin "$TAG"
  echo "✓ Tag ${TAG} pushed"

  # Create GitHub release (prerelease if version contains -)
  if [[ "$TAG" == *-* ]]; then
    GH_PAGER="" gh release create "$TAG" --prerelease --generate-notes --title "$TAG"
  else
    GH_PAGER="" gh release create "$TAG" --generate-notes --title "$TAG"
  fi
  echo "✓ GitHub release created for ${TAG}"
fi
