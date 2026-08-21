#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: verify-mcp-conformance.sh [--repo-dir DIR] [--dry-run] [--no-build] [--keep-workspace]

Drives the akr-mcp binary with real MCP client libraries — several major versions of the
`rmcp` crate at once — and fails if any of them rejects a response this server calls
successful.

This is the half of conformance that `cargo test -p akr-mcp` cannot do. That suite checks
the wire envelope against tables we wrote ourselves, which catches a shape we know is wrong
but never a client that has become stricter. Every MCP failure this project has shipped was
of the second kind: `resultType: "tool"` sat in the server for a fortnight while Claude Code
and Codex 0.147 called it happily, because neither read the field, and then Codex 0.149
typed results by it and every tool call stopped working with the server unchanged.

Options:
  --repo-dir DIR    AKR repo root (default: parent directory of this script)
  --dry-run         Print what would run, change nothing
  --no-build        Use whatever is already in target/ instead of building first
  --keep-workspace  Leave the throwaway AKR workspace behind for inspection
  -h, --help        Show this help

Exit status is 0 when every client generation accepted every response, and non-zero
otherwise — including when the build or the workspace setup fails.
USAGE
}

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN=0
DO_BUILD=1
KEEP_WORKSPACE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-dir)
      REPO_DIR="$2"; shift 2 ;;
    --dry-run)
      DRY_RUN=1; shift ;;
    --no-build)
      DO_BUILD=0; shift ;;
    --keep-workspace)
      KEEP_WORKSPACE=1; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "error: unknown argument $1" >&2; usage >&2; exit 2 ;;
  esac
done

log() { echo "[verify-mcp-conformance] $*"; }
run() {
  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "DRY-RUN: $*"
  else
    "$@"
  fi
}

TOOL_DIR="$REPO_DIR/tools/mcp-conformance"
if [[ ! -f "$TOOL_DIR/Cargo.toml" ]]; then
  echo "error: conformance tool missing at $TOOL_DIR" >&2
  exit 1
fi

# `debug-release` rather than `release-final`: this is a wire-shape check run on every
# change, and the ninety seconds LTO costs buys nothing a protocol conformance run can use.
BUILD_PROFILE="debug-release"
AKR_MCP_BIN="$REPO_DIR/target/$BUILD_PROFILE/akr-mcp"
AKR_BIN="$REPO_DIR/target/$BUILD_PROFILE/akr"

log "Repo: $REPO_DIR"
if [[ "$DO_BUILD" -eq 1 ]]; then
  log "Building akr-mcp and akr ($BUILD_PROFILE)..."
  run cargo build --manifest-path "$REPO_DIR/Cargo.toml" \
    --profile "$BUILD_PROFILE" --package akr-mcp --package akr-cli
else
  log "Skipping build on request (--no-build); using target/$BUILD_PROFILE"
fi

if [[ "$DRY_RUN" -eq 0 ]]; then
  for binary in "$AKR_MCP_BIN" "$AKR_BIN"; do
    if [[ ! -x "$binary" ]]; then
      echo "error: not built: $binary" >&2
      echo "Hint: rerun without --no-build." >&2
      exit 1
    fi
  done
fi

# A throwaway workspace, built the way a first-time user's is. The worked example under
# examples/ is deliberately not reused: its value is frozen content and rewritten commit
# hashes, and none of that changes the shape of a tools/call envelope. `akr init` is the
# smaller, more honest fixture — and a server that only speaks correct MCP against a rich
# ledger is still broken.
WORKSPACE="$(mktemp -d -t akr-conformance-XXXXXX)"
cleanup() {
  if [[ "$KEEP_WORKSPACE" -eq 1 ]]; then
    log "Workspace kept at $WORKSPACE"
  else
    rm -rf "$WORKSPACE"
  fi
}
trap cleanup EXIT

log "Materialising a throwaway workspace at $WORKSPACE..."
if [[ "$DRY_RUN" -eq 0 ]]; then
  git -C "$WORKSPACE" init --quiet --initial-branch=main
  git -C "$WORKSPACE" config user.name "AKR Conformance"
  git -C "$WORKSPACE" config user.email "conformance@example.invalid"
  git -C "$WORKSPACE" config commit.gpgsign false
  # `akr init` resolves the workspace from the current directory, not --dir, and walking
  # up from anywhere inside this repository finds this repository's own ledger.
  ( cd "$WORKSPACE" && "$AKR_BIN" init --project conformance >/dev/null )
  git -C "$WORKSPACE" add -A
  # Several read paths key off HEAD, so an uncommitted workspace is not the shape a real
  # session ever has.
  git -C "$WORKSPACE" commit --quiet -m "conformance workspace"
else
  log "DRY-RUN: git init + akr init in $WORKSPACE"
fi

log "Running the client matrix..."
if [[ "$DRY_RUN" -eq 1 ]]; then
  log "DRY-RUN: (cd $TOOL_DIR && AKR_MCP_BIN=$AKR_MCP_BIN AKR_MCP_WORKSPACE=$WORKSPACE cargo run --locked)"
  exit 0
fi

# The tool is outside the workspace, so it cannot reach the server through
# CARGO_BIN_EXE_akr-mcp the way the in-workspace tests do. Two variables instead.
AKR_MCP_BIN="$AKR_MCP_BIN" \
AKR_MCP_WORKSPACE="$WORKSPACE" \
  cargo run --quiet --manifest-path "$TOOL_DIR/Cargo.toml" --locked
