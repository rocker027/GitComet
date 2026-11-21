#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/cli.sh"

usage() {
  cat <<'EOF'
Usage: scripts/symbolicate-minidump.sh [--symbols-url URL] [--cache DIR] [--json] MINIDUMP [-- EXTRA_ARGS...]

Turns a user-submitted minidump into a readable stack trace by resolving it
against a Breakpad symbol store you host for your builds. Symbols are fetched lazily over HTTP
and cached, so only the modules a crash actually touches are downloaded, and
only once per build.

Only builds whose symbols you published to that store can be resolved.

Options:
  --symbols-url  Symbol store base URL (required unless GITCOMET_SYMBOLS_URL is set)
  --cache        Local symbol cache (default: ${XDG_CACHE_HOME:-$HOME/.cache}/revweave-symbols)
  --json         Emit the JSON report instead of the human-readable one

Environment:
  GITCOMET_SYMBOLS_URL
    Overrides the symbol store base URL, for a staging store or a local mirror.
EOF
}

# RevWeave has no public symbol store, so there is no default URL; see
# docs/crash-symbolication.md.
symbols_url="${GITCOMET_SYMBOLS_URL:-}"
cache_dir="${XDG_CACHE_HOME:-${HOME}/.cache}/revweave-symbols"
output_flag="--human"
minidump=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --symbols-url)
      require_value "$@"
      symbols_url="$2"
      shift 2
      ;;
    --cache)
      require_value "$@"
      cache_dir="$2"
      shift 2
      ;;
    --json)
      output_flag="--json"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$minidump" ]]; then
        echo "Only one minidump may be given (got '$minidump' and '$1')." >&2
        exit 2
      fi
      minidump="$1"
      shift
      ;;
  esac
done

if [[ -z "$minidump" ]]; then
  echo "A minidump path is required." >&2
  usage >&2
  exit 2
fi

if [[ ! -f "$minidump" ]]; then
  echo "Minidump not found: $minidump" >&2
  exit 1
fi

if ! command -v minidump-stackwalk >/dev/null 2>&1; then
  echo "minidump-stackwalk is not on PATH." >&2
  echo "Install it with: cargo install minidump-stackwalk --locked" >&2
  exit 1
fi

if [[ -z "$symbols_url" ]]; then
  echo "No symbol store URL. Pass --symbols-url or set GITCOMET_SYMBOLS_URL." >&2
  echo "See docs/crash-symbolication.md." >&2
  exit 2
fi

mkdir -p "$cache_dir"
# Downloads are atomically renamed into the cache, so the temporary directory
# must live on the same filesystem as the cache.
symbols_tmp="$(mktemp -d "${cache_dir%/}/.symbols-tmp.XXXXXX")"
cleanup_symbols_tmp() {
  rm -rf -- "$symbols_tmp"
}
trap cleanup_symbols_tmp EXIT

# minidump-stackwalk is `[OPTIONS] <MINIDUMP> [SYMBOLS_PATHS]...`, so extra
# arguments have to precede the positional: after it they are parsed as local
# symbol paths and silently ignored.
minidump-stackwalk \
  "$output_flag" \
  --symbols-url "$symbols_url" \
  --symbols-cache "$cache_dir" \
  --symbols-tmp "$symbols_tmp" \
  "$@" \
  "$minidump"
