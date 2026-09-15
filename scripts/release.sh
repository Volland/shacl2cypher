#!/usr/bin/env bash
# Release shacl2cypher: bump every version, verify, push, wait for CI, then tag.
#
# Pushing the tag runs .github/workflows/release.yml, which creates the GitHub
# release with CLI binaries and publishes the Python (PyPI) and Node.js (npm)
# packages once every build and smoke test passes.
#
# Usage: scripts/release.sh <version> [options]
#   --dry-run          check prerequisites and print the plan; change nothing
#   --skip-checks      skip local fmt, clippy and tests (CI still has to pass)
#   --publish-crates   after the release workflow succeeds, `cargo publish`
#                      shacl2cypher-core, shacl2cypher-runner and shacl2cypher
#   --yes              do not ask before pushing the tag
#
# Needs git, gh (signed in), cargo, node and python3. One-time setup:
#   - PyPI trusted publisher: project shacl2cypher, repo Volland/shacl2cypher,
#     workflow release.yml, environment release
#   - gh secret set NPM_TOKEN --env release --repo Volland/shacl2cypher
#   - cargo login (only for --publish-crates)

set -euo pipefail

REPO="Volland/shacl2cypher"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

version=""
dry_run=false
skip_checks=false
publish_crates=false
assume_yes=false

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

for arg in "$@"; do
  case "$arg" in
    --dry-run) dry_run=true ;;
    --skip-checks) skip_checks=true ;;
    --publish-crates) publish_crates=true ;;
    --yes) assume_yes=true ;;
    -h | --help) usage 0 ;;
    -*) echo "unknown option: $arg" >&2; usage 2 ;;
    *) [[ -z "$version" ]] || usage 2; version="$arg" ;;
  esac
done
[[ -n "$version" ]] || usage 2

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }
confirm() {
  $assume_yes && return 0
  read -r -p "$1 [y/N] " answer
  [[ "$answer" == [yY] || "$answer" == [yY][eE][sS] ]]
}

workspace_version() {
  python3 - <<'EOF'
import re
text = open("Cargo.toml").read()
section = text.split("[workspace.package]", 1)[1]
print(re.search(r'^version = "([^"]+)"', section, re.M).group(1))
EOF
}

# Waits for the newest run of a workflow on a commit and fails unless it succeeds.
wait_for_run() {
  local workflow="$1" commit="$2" id=""
  for _ in $(seq 1 40); do
    id="$(gh run list --repo "$REPO" --workflow "$workflow" --commit "$commit" --limit 1 \
      --json databaseId --jq '.[0].databaseId // empty')"
    [[ -n "$id" ]] && break
    sleep 15
  done
  [[ -n "$id" ]] || fail "no $workflow run started for ${commit:0:7}"
  echo "watching $workflow run https://github.com/$REPO/actions/runs/$id"
  gh run watch "$id" --repo "$REPO" --exit-status --interval 30 >/dev/null ||
    fail "$workflow failed: https://github.com/$REPO/actions/runs/$id"
  echo "$workflow passed"
}

step "Checking prerequisites"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version must look like 1.2.3, got $version"
tag="v$version"
for tool in git gh cargo node python3; do
  command -v "$tool" >/dev/null || fail "$tool is not installed"
done

# Collect every unmet prerequisite so one run shows them all.
problems=()
problem() { problems+=("$*"); }
gh auth status >/dev/null 2>&1 || problem "gh is not signed in (gh auth login)"
[[ "$(git branch --show-current)" == main ]] || problem "releases are made from main"
[[ -z "$(git status --porcelain --untracked-files=no -- . ':!.omc' ':!**/.omc')" ]] ||
  problem "commit or stash tracked changes first"
if git fetch --quiet origin main --tags; then
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] ||
    problem "main is not in sync with origin/main"
else
  problem "cannot fetch origin, so main cannot be checked against origin/main"
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null ||
  git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
  problem "tag $tag already exists"
fi
gh api "repos/$REPO/environments/release" >/dev/null 2>&1 ||
  problem "GitHub environment 'release' is missing"
gh secret list --repo "$REPO" --env release 2>/dev/null | grep -q '^NPM_TOKEN' ||
  problem "secret NPM_TOKEN is missing (gh secret set NPM_TOKEN --env release --repo $REPO)"
if $publish_crates; then
  [[ -s "${CARGO_HOME:-$HOME/.cargo}/credentials.toml" ]] || problem "run cargo login first"
fi

current="$(workspace_version)"
echo "current version $current, releasing $version"
python3 -c 'import sys; a, b = (tuple(map(int, v.split("."))) for v in sys.argv[1:]); sys.exit(a < b)' \
  "$version" "$current" || problem "version $version is older than the current $current"
echo "reminder: PyPI trusted publishing must be configured for $REPO (release.yml, environment release)"
for message in "${problems[@]+"${problems[@]}"}"; do
  printf 'problem: %s\n' "$message" >&2
done

if $dry_run; then
  step "Dry run: nothing changed"
  cat <<EOF
Would:
  1. $([[ "$current" == "$version" ]] && echo "keep version $version" || echo "bump $current -> $version (Cargo, npm, snapshots, Cargo.lock)")
  2. $($skip_checks && echo "skip local checks" || echo "run cargo fmt --check, clippy and tests")
  3. commit and push to main, then wait for CI
  4. push tag $tag and wait for the Release workflow
  5. $($publish_crates && echo "cargo publish shacl2cypher-core, shacl2cypher-runner, shacl2cypher" || echo "leave crates.io unchanged")
EOF
  if (( ${#problems[@]} > 0 )); then
    echo "${#problems[@]} problem(s) must be fixed before releasing" >&2
    exit 1
  fi
  exit 0
fi
(( ${#problems[@]} == 0 )) || fail "fix the problems above before releasing"

if [[ "$current" != "$version" ]]; then
  step "Bumping $current -> $version"
  python3 - "$current" "$version" <<'EOF'
import re, sys
from pathlib import Path

old, new = sys.argv[1], sys.argv[2]

def edit(path, pattern, replacement, count):
    file = Path(path)
    text, n = re.subn(pattern, replacement, file.read_text(), flags=re.M)
    if n != count:
        sys.exit(f"{path}: expected {count} replacement(s) of {pattern!r}, made {n}")
    file.write_text(text)

edit("Cargo.toml", r'^version = "' + re.escape(old) + '"', f'version = "{new}"', 1)
for crate in ("crates/s2c-cli/Cargo.toml", "crates/s2c-runner/Cargo.toml"):
    count = 2 if "s2c-cli" in crate else 1
    edit(crate, r'(shacl2cypher-(?:core|runner) = \{[^}]*version = ")' + re.escape(old) + '"',
         r"\g<1>" + new + '"', count)
for snapshot in Path("crates/s2c-core/tests/snapshots").glob("*.snap"):
    edit(snapshot, r"Generated by shacl2cypher \S+ for", f"Generated by shacl2cypher {new} for", 1)
EOF
  node crates/s2c-node/scripts/version.js set "$version"
  cargo update --workspace --quiet
fi
node crates/s2c-node/scripts/version.js check "$version"

if ! $skip_checks; then
  step "Running local checks"
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
fi

if [[ -n "$(git status --porcelain --untracked-files=no -- . ':!.omc' ':!**/.omc')" ]]; then
  step "Committing and pushing the version bump"
  git add Cargo.toml Cargo.lock crates/s2c-cli/Cargo.toml crates/s2c-runner/Cargo.toml \
    crates/s2c-core/tests/snapshots crates/s2c-node/package.json crates/s2c-node/npm
  git commit --quiet -m "Release $tag"
  git push --quiet origin main
fi
commit="$(git rev-parse HEAD)"

step "Waiting for CI on ${commit:0:7}"
wait_for_run CI "$commit"

confirm "Push tag $tag and publish to PyPI and npm?" || fail "stopped before tagging; nothing was published"
step "Tagging $tag"
git tag -a "$tag" -m "shacl2cypher $tag" "$commit"
git push --quiet origin "$tag"
wait_for_run Release "$commit"

if $publish_crates; then
  step "Publishing crates"
  for crate in shacl2cypher-core shacl2cypher-runner shacl2cypher; do
    cargo publish --locked -p "$crate"
  done
fi

step "Released $tag"
cat <<EOF
  GitHub: https://github.com/$REPO/releases/tag/$tag
  PyPI:   https://pypi.org/project/shacl2cypher/$version/
  npm:    https://www.npmjs.com/package/shacl2cypher/v/$version
EOF
