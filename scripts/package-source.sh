#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/package-source.sh --commit COMMIT --version VERSION --out DIR

Create a source archive from a fixed Git commit, including the pinned syntax
fixture and Cargo.lock dependencies. Cargo may download dependencies; the Git
submodule is cloned into a temporary directory. The checkout is not modified.
The requested version must equal [workspace.package].version in the commit.
Existing output files are never overwritten.
EOF
}

fail() { echo "error: $*" >&2; exit 1; }

commit="" version="" out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --commit|--version|--out)
      [[ $# -ge 2 && -n "$2" ]] || fail "missing value for $1"
      case "$1" in
        --commit) commit="$2" ;;
        --version) version="$2" ;;
        --out) out="$2" ;;
      esac
      shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown argument: $1" ;;
  esac
done
[[ -n "$commit" && -n "$version" && -n "$out" ]] || { usage >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || fail "invalid version: $version"
[[ -d "$out" ]] || fail "output directory does not exist: $out"
for tool in git cargo tar gzip mktemp python3; do
  command -v "$tool" >/dev/null 2>&1 || fail "required tool not found: $tool"
done

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$(cd "$out" && pwd)"
sha="$(git -C "$repo" rev-parse --verify "${commit}^{commit}")" || fail "invalid commit: $commit"
archive="$out/revweave-v${version}-source.tar.gz"
[[ ! -e "$archive" && ! -L "$archive" ]] || fail "output already exists: $archive"

work="$(mktemp -d "${TMPDIR:-/tmp}/revweave-source.XXXXXX")"
partial=""
cleanup() { rm -rf "$work"; [[ -z "$partial" ]] || rm -f "$partial"; }
trap cleanup EXIT
root="revweave-v${version}-source"
mkdir -p "$work/$root"
git -C "$repo" archive "$sha" | tar -xf - -C "$work/$root"

actual_version="$(python3 - "$work/$root/Cargo.toml" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
section = re.search(r'^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)', text, re.M | re.S)
match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', section.group(1), re.M) if section else None
if not match:
    sys.exit("error: workspace package version not found")
print(match.group(1))
PY
)"
[[ "$actual_version" == "$version" ]] || fail "version mismatch: commit has $actual_version, requested $version"

# Do not silently omit gitlinks when archiving a release commit.
gitlinks="$(git -C "$repo" ls-tree -r "$sha" | awk '$1 == "160000" {print $3 " " $4}')"
[[ "$gitlinks" =~ ^[0-9a-f]+\ fixtures/syntax_test$ ]] || fail "unexpected submodule layout: $gitlinks"
fixture_sha="${gitlinks%% *}"
fixture_url="$(git config -f "$work/$root/.gitmodules" --get submodule.fixtures/syntax_test.url)"
[[ -n "$fixture_url" ]] || fail "missing fixture submodule URL"
git clone --quiet --no-checkout "$fixture_url" "$work/fixture"
git -C "$work/fixture" cat-file -e "${fixture_sha}^{commit}" ||
  git -C "$work/fixture" fetch --quiet origin "$fixture_sha"
[[ -z "$(git -C "$work/fixture" ls-tree -r "$fixture_sha" | awk '$1 == "160000" {print}')" ]] ||
  fail "nested fixture submodules are not supported"
mkdir -p "$work/$root/fixtures/syntax_test"
git -C "$work/fixture" archive "$fixture_sha" | tar -xf - -C "$work/$root/fixtures/syntax_test"

[[ ! -e "$work/$root/cargo-vendor" && ! -e "$work/$root/SOURCE-REVISION" ]] ||
  fail "reserved source archive path already exists in commit"
# Cargo resolves the lockfile inside the committed snapshot, not the live tree.
(
  cd "$work/$root"
  cargo vendor --locked --versioned-dirs cargo-vendor > "$work/cargo-sources.toml"
  mkdir -p .cargo
  if [[ -f .cargo/config.toml ]] && grep -Eq '^\[source\.' .cargo/config.toml; then
    fail "existing Cargo source replacement requires manual review"
  fi
  printf '\n# Sources vendored for this archive.\n' >> .cargo/config.toml
  cat "$work/cargo-sources.toml" >> .cargo/config.toml
)
printf 'RevWeave commit: %s\nfixtures/syntax_test commit: %s\n' "$sha" "$fixture_sha" > "$work/$root/SOURCE-REVISION"

# Link the finished archive atomically so an existing file is never replaced.
partial="$(mktemp "$out/.revweave-source.XXXXXX")"
tar -C "$work" -czf "$partial" "$root"
chmod 644 "$partial"
ln "$partial" "$archive" || fail "could not create output (it may already exist): $archive"
echo "Wrote $archive (commit $sha)"
