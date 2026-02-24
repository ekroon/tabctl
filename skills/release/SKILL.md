---
name: release
description: 'Bump version, test, build, tag, push, and create a GitHub Release. Use when user asks to release, bump version, cut a release, or mentions "/release". Analyzes conventional commits to auto-detect bump type. Always confirms before pushing.'
license: MIT
allowed-tools: Bash, AskUser
---

# Release

Automate the full release cycle: version bump → test → build → commit → tag → push → GitHub Release.

## Prerequisites

- Working tree is clean (`git status` shows no uncommitted changes)
- On the `main` branch (or the branch you release from)
- `gh` CLI is authenticated (`gh auth status`)
- `npm test` and `npm run build` pass

## Workflow

### Step 1: Pre-flight checks

```bash
# Verify clean working tree
git status --porcelain

# Verify branch
git branch --show-current

# Verify gh auth
gh auth status
```

If the working tree is dirty, stop and ask the user to commit or stash first.
If not on `main`, warn the user and ask whether to proceed.

### Step 2: Determine version bump

Find the last tag and analyze commits since then:

```bash
LAST_TAG=$(git tag --sort=-v:refname | head -1)
git log "${LAST_TAG}..HEAD" --oneline --no-decorate
```

**Auto-detect bump type from conventional commits:**

| Signal | Bump |
|--------|------|
| Any commit with `BREAKING CHANGE` in body/footer, or type ending in `!` (e.g. `feat!:`) | **major** |
| Any `feat:` or `feat(scope):` commit | **minor** |
| Only `fix:`, `perf:`, `docs:`, `chore:`, `refactor:`, `test:`, `ci:`, `build:`, `style:` | **patch** |

Compute the new version:

```bash
CURRENT=$(echo "$LAST_TAG" | sed 's/^v//')
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

# For major bump:
NEW="$((MAJOR + 1)).0.0"
# For minor bump:
NEW="$MAJOR.$((MINOR + 1)).0"
# For patch bump:
NEW="$MAJOR.$MINOR.$((PATCH + 1))"
```

### Step 3: Show plan and confirm

Before making any changes, present a summary and use AskUser to confirm:

```
Release plan:
  Current version: v{CURRENT}
  New version:     v{NEW} ({BUMP_TYPE} bump)
  Commits:         {N} commits since {LAST_TAG}

  {commit list}

  Steps:
  1. Update package.json version to {NEW}
  2. Sync packages/win32-x64/package.json version
  3. Run npm test
  4. Run npm run build
  5. Commit: chore(release): v{NEW}
  6. Tag: v{NEW}
  7. Push commit + tag to origin
  8. Create GitHub Release with auto-generated notes
```

Ask the user to confirm the version bump. Offer the recommended bump plus alternatives:
- `v{NEW} ({BUMP_TYPE}, recommended)`
- Other bump options

If the user picks a different bump or types a custom version, use that instead.

### Step 4: Update version in package.json

```bash
npm version {NEW} --no-git-tag-version
```

This updates `package.json` and `package-lock.json` without creating a git tag (we do that ourselves).

### Step 5: Sync platform package version

```bash
node -e "
  const fs = require('fs');
  const p = './packages/win32-x64/package.json';
  const pkg = JSON.parse(fs.readFileSync(p, 'utf8'));
  pkg.version = '${NEW}';
  fs.writeFileSync(p, JSON.stringify(pkg, null, 2) + '\n');
"
```

### Step 6: Run tests

```bash
npm test
```

If tests fail, stop and report the failure. Do NOT proceed with the release.

### Step 7: Build

```bash
npm run build
```

If the build fails, stop and report the failure. Do NOT proceed with the release.

### Step 8: Commit

```bash
git add package.json package-lock.json packages/win32-x64/package.json
git commit -m "chore(release): v${NEW}"
```

### Step 9: Tag

```bash
git tag -a "v${NEW}" -m "v${NEW}"
```

### Step 10: Push

```bash
git push origin main
git push origin "v${NEW}"
```

### Step 11: Create GitHub Release

```bash
if [[ "$NEW" == *-* ]]; then
  GH_PAGER="" gh release create "v${NEW}" --prerelease --generate-notes --title "v${NEW}"
else
  GH_PAGER="" gh release create "v${NEW}" --generate-notes --title "v${NEW}"
fi
```

For prerelease semver (for example `-alpha.N` or `-rc.N`), always pass `--prerelease` so release validation does not treat it as stable.

The `--generate-notes` flag auto-generates release notes from merged PRs and commits.
The existing `release.yml` workflow will trigger on the release and publish to npm.

After creation, display the release URL so the user can review and edit the notes if desired.

## Dry-run mode

If the user asks for a dry run (or passes `--dry-run`), perform Steps 1-3 only: pre-flight checks, version analysis, and show the plan. Do NOT modify any files, commit, tag, push, or create a release.

## Abort and rollback

If any step after Step 4 fails (tests, build, commit, tag, push, or release creation), provide rollback instructions:

```bash
# Undo version bump (if committed)
git reset --soft HEAD~1
git checkout -- package.json package-lock.json packages/win32-x64/package.json

# Delete local tag (if created)
git tag -d "v${NEW}"

# Delete remote tag (if pushed) — only with explicit user confirmation
git push origin --delete "v${NEW}"
```

## Safety rules

- NEVER skip tests or build
- NEVER force push
- NEVER create a release without user confirmation
- NEVER delete existing tags or releases without explicit user request
- If `--dry-run`, do NOT modify anything
