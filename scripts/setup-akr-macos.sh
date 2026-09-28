#!/usr/bin/env bash
# One-shot macOS setup for AKR: checks the toolchain, builds and installs the
# `akr` CLI and the `akr-mcp` server into ~/.local/bin, puts that directory on
# the login-shell PATH, and registers the MCP server with Claude Code.
#
# This is a thin macOS front for scripts/setup-akr-mcp.sh, which does the
# building, installing and registration and is safe to re-run. What this
# script adds is the macOS-specific ground: the Xcode command-line tools (git,
# the linker), a rustup toolchain new enough for the workspace, and the PATH
# entry that macOS login shells (zsh) do not carry by default.
#
# Usage: scripts/setup-akr-macos.sh [--debug] [--with-codex] [--with-opencode] [--dry-run]
#
#   --debug          build the debug profile instead of release-final (faster)
#   --with-codex     also update ~/.codex/config.toml (off by default on macOS)
#   --with-opencode  also update ~/.config/opencode/opencode.jsonc (needs jq)
#   --dry-run        print what would change without writing anything
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIN_RUST="1.94"
DRY_RUN=0
WITH_CODEX=0
WITH_OPENCODE=0
DEBUG_BUILD=0

for arg in "$@"; do
  case "$arg" in
    --debug) DEBUG_BUILD=1 ;;
    --with-codex) WITH_CODEX=1 ;;
    --with-opencode) WITH_OPENCODE=1 ;;
    --dry-run) DRY_RUN=1 ;;
    -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
    *) echo "error: unknown arg: $arg" >&2; exit 1 ;;
  esac
done
EXTRA_ARGS=()
[[ "$WITH_CODEX" -eq 1 ]] || EXTRA_ARGS+=(--no-codex)
[[ "$WITH_OPENCODE" -eq 1 ]] || EXTRA_ARGS+=(--no-opencode)
[[ "$DEBUG_BUILD" -eq 1 ]] && EXTRA_ARGS+=(--debug)
[[ "$DRY_RUN" -eq 1 ]] && EXTRA_ARGS+=(--dry-run)

log() { echo "[setup-akr-macos] $*"; }

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: this script is for macOS; on Linux run scripts/setup-akr-mcp.sh directly" >&2
  exit 1
fi

# 1. Xcode command-line tools: git and the system linker come from them.
if ! xcode-select -p >/dev/null 2>&1; then
  log "Xcode command-line tools are missing. Requesting the install dialog..."
  xcode-select --install || true
  echo "Re-run this script once the command-line tools have finished installing." >&2
  exit 1
fi
log "Xcode command-line tools: $(xcode-select -p)"

# 2. Rust via rustup, new enough for the workspace (edition 2024).
if ! command -v rustup >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/rustup" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v rustup >/dev/null 2>&1; then
  log "rustup is missing. Install it with:"
  echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
  echo "then re-run this script." >&2
  exit 1
fi
RUST_VERSION="$(rustc --version | awk '{print $2}')"
version_ge() { [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]; }
if ! version_ge "${RUST_VERSION%%-*}" "$MIN_RUST"; then
  log "rustc $RUST_VERSION is older than $MIN_RUST; updating the stable toolchain"
  [[ "$DRY_RUN" -eq 1 ]] || rustup update stable
fi
log "Rust: $(rustc --version)"

# 3. ~/.local/bin on the login-shell PATH. zsh is the macOS default shell and
#    reads ~/.zprofile for login shells; Claude Code and Terminal both start
#    from one. Idempotent: the line is added once.
BIN_DIR="$HOME/.local/bin"
ZPROFILE="$HOME/.zprofile"
PATH_LINE='export PATH="$HOME/.local/bin:$PATH"'
mkdir -p "$BIN_DIR"
if ! grep -qsF '.local/bin' "$ZPROFILE" 2>/dev/null; then
  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "DRY-RUN: would append to $ZPROFILE: $PATH_LINE"
  else
    printf '\n# AKR CLI and MCP server (scripts/setup-akr-macos.sh)\n%s\n' "$PATH_LINE" >> "$ZPROFILE"
    log "Added $BIN_DIR to PATH in $ZPROFILE"
  fi
else
  log "$ZPROFILE already puts ~/.local/bin on PATH"
fi
export PATH="$BIN_DIR:$PATH"

# 4. Build, install, register. The shared installer is the authority on how.
log "Delegating to scripts/setup-akr-mcp.sh ${EXTRA_ARGS[*]}"
bash "$REPO_DIR/scripts/setup-akr-mcp.sh" --repo-dir "$REPO_DIR" "${EXTRA_ARGS[@]}"

# 5. Prove it from a fresh shell's point of view.
if [[ "$DRY_RUN" -eq 0 ]]; then
  log "akr: $("$BIN_DIR/akr" --version)"
  log "akr-mcp: $("$BIN_DIR/akr-mcp" --version)"
  log "Open a new terminal (or run: source $ZPROFILE) so 'akr' is on PATH."
fi
