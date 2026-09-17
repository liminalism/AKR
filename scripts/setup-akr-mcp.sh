#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: setup-akr-mcp.sh [--repo-dir DIR] [--dry-run] [--no-claude] [--no-codex] [--no-opencode] [--no-agents] [--no-build] [--debug]

One-time setup for the AKR MCP server across:
- AkR CLI build/install
- Codex MCP config (~/.codex/config.toml)
- OpenCode MCP config (~/.config/opencode/opencode.jsonc)
- Claude MCP registration
- The AKR section of the global agent instruction files

Options:
  --repo-dir DIR   AKR repo root (default: parent directory of this script)
  --dry-run        Print changes without writing
  --debug          Use target/debug/akr-mcp instead of target/release-final
  --no-claude      Skip Claude registration
  --no-codex       Skip Codex config update
  --no-opencode    Skip OpenCode config update
  --no-agents      Skip the agent instruction files
  --no-build       Install what is already in target/ without building first
  -h, --help       Show this help

Safe to re-run: every step either rewrites in place or is a no-op. Run it after any
change to the AKR source or to scripts/agent-section.md.
USAGE
}

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DRY_RUN=0
DO_CLAUDE=1
DO_CODEX=1
DO_OPENCODE=1
DO_AGENTS=1
DO_BUILD=1
USE_DEBUG=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-dir)
      REPO_DIR="$2"; shift 2 ;;
    --dry-run)
      DRY_RUN=1; shift ;;
    --debug)
      USE_DEBUG=1; shift ;;
    --no-claude)
      DO_CLAUDE=0; shift ;;
    --no-codex)
      DO_CODEX=0; shift ;;
    --no-opencode)
      DO_OPENCODE=0; shift ;;
    --no-agents)
      DO_AGENTS=0; shift ;;
    --no-build)
      DO_BUILD=0; shift ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "error: unknown arg: $1" >&2
      usage
      exit 1 ;;
  esac
done

CARGO_TOML="$REPO_DIR/Cargo.toml"
if [[ ! -f "$CARGO_TOML" ]]; then
  echo "error: no Cargo.toml at repo root: $CARGO_TOML" >&2
  exit 1
fi

if [[ ! -d "$REPO_DIR/crates/akr-mcp" ]]; then
  echo "error: akr-mcp crate missing at $REPO_DIR/crates/akr-mcp" >&2
  exit 1
fi

AKR_BIN_DIR="${HOME}/.local/bin"
AKR_BIN="$AKR_BIN_DIR/akr"
AKR_MCP_BIN="$AKR_BIN_DIR/akr-mcp"

log() { echo "[setup-akr-mcp] $*"; }
run() { if [[ "$DRY_RUN" -eq 1 ]]; then log "DRY-RUN: $*"; else "$@"; fi; }

# Under Git Bash the paths written into the editor configs are MSYS-style (/c/Users/...).
# A tool that is itself an MSYS program resolves those; a native Windows one may not. The
# PowerShell mirror writes native paths, so say so once rather than silently rewriting a
# setup that currently works.
case "$(uname -o 2>/dev/null || echo unknown)" in
  Msys | Cygwin)
    log "NOTE: running under $(uname -o); editor configs will receive MSYS-style paths"
    log "      like $AKR_MCP_BIN. If Codex or OpenCode fails to start the server, run"
    log "      scripts/setup-akr-mcp.ps1 instead — it writes native Windows paths."
    ;;
esac

# Build and install AKR MCP
if [[ "$USE_DEBUG" -eq 1 ]]; then
  BUILD_MODE="debug"
  BUILD_CMD=(cargo build --package akr-cli --package akr-mcp)
  SOURCE_AKR="$REPO_DIR/target/debug/akr"
  SOURCE_BIN="$REPO_DIR/target/debug/akr-mcp"
else
  BUILD_MODE="release-final"
  BUILD_CMD=(cargo build --profile release-final --package akr-cli --package akr-mcp)
  SOURCE_AKR="$REPO_DIR/target/release-final/akr"
  SOURCE_BIN="$REPO_DIR/target/release-final/akr-mcp"
fi

log "Using repo: $REPO_DIR"
log "Build mode: $BUILD_MODE"
# Always build unless told not to. Cargo is the authority on what is stale, and it is a
# fast no-op when nothing changed; deciding for it is how a script installs yesterday's
# binary over today's source.
if [[ "$DO_BUILD" -eq 0 ]]; then
  log "Skipping build on request (--no-build); installing whatever is in target/$BUILD_MODE"
elif [[ "$DRY_RUN" -eq 1 ]]; then
  log "DRY-RUN: (cd $REPO_DIR && ${BUILD_CMD[*]})"
else
  (cd "$REPO_DIR" && "${BUILD_CMD[@]}")
fi

if [[ "$DRY_RUN" -eq 0 && ( ! -x "$SOURCE_AKR" || ! -x "$SOURCE_BIN" ) ]]; then
  echo "error: built binaries not found: $SOURCE_AKR, $SOURCE_BIN" >&2
  echo "Hint: rerun with --debug or build first manually." >&2
  exit 1
fi

# Copy beside the target and rename over it, rather than writing through it.
#
# `cp` onto a binary that a running MCP server has mapped fails with ETXTBSY ("Text file
# busy"), which is the common case: the server whose stale build you are replacing is the
# reason you are running this. `rename(2)` swaps the directory entry instead, so the
# running process keeps the inode it already opened and the next start picks up the new
# one — and it is atomic, so there is never a half-written binary on the path.
if [[ "$DRY_RUN" -eq 1 ]]; then
  log "DRY-RUN: install $SOURCE_AKR -> $AKR_BIN and $SOURCE_BIN -> $AKR_MCP_BIN"
else
  mkdir -p "$AKR_BIN_DIR"
  install_binary() {
    local source="$1" dest="$2"
    if [[ -f "$dest" ]] && cmp -s "$source" "$dest"; then
      log "Unchanged, leaving in place: $dest"
      return 0
    fi
    cp "$source" "$dest.new"
    # Displace any existing image before renaming the new one into place. Unix would
    # allow the replace outright, but under Git Bash on Windows the destination of a
    # rename cannot be a file some process has running, while renaming that file *away*
    # is allowed. Cleaning up the displaced copy is best effort for the same reason.
    if [[ -f "$dest" ]]; then
      rm -f "$dest.old" 2>/dev/null || true
      mv -f "$dest" "$dest.old" 2>/dev/null || true
    fi
    mv -f "$dest.new" "$dest"
    rm -f "$dest.old" 2>/dev/null || true
  }
  install_binary "$SOURCE_AKR" "$AKR_BIN"
  install_binary "$SOURCE_BIN" "$AKR_MCP_BIN"
fi
log "Installed $AKR_BIN"
log "Installed $AKR_MCP_BIN"
if [[ "$DRY_RUN" -eq 0 ]]; then
  INSTALLED_VERSION="$($AKR_MCP_BIN --version)"
  log "Verified installed server: $INSTALLED_VERSION ($AKR_MCP_BIN)"
fi
log "NOTE: a server that is already running keeps the old binary until it restarts."
log "      Reconnect the MCP server (or restart the session) before using knowledge.* tools."

# Install/refresh a section in ~/.codex/config.toml
if [[ "$DO_CODEX" -eq 1 ]]; then
  CODEX_CFG="$HOME/.codex/config.toml"
  if [[ ! -f "$CODEX_CFG" ]]; then
    echo "warning: Codex config not found: $CODEX_CFG (skipping Codex update)"
  else
    if grep -q '^\[mcp_servers\.akr\]' "$CODEX_CFG"; then
      log "Codex already has [mcp_servers.akr]; updating command to $AKR_MCP_BIN"
      # awk rather than python3: this used to hand an MSYS-style path to a native Windows
      # python, which cannot open it, and `set -e` then aborted the whole script before the
      # remaining steps ran. awk also keeps the script's dependencies to what a shell
      # already has. Only the `command` key inside the akr section is touched; other keys,
      # other sections and the file's ordering are preserved.
      if [[ "$DRY_RUN" -eq 0 ]]; then
        cp "$CODEX_CFG" "$CODEX_CFG.bak"
        CODEX_TMP="$(mktemp)"
        CMD="$AKR_MCP_BIN" awk '
          BEGIN { cmd = ENVIRON["CMD"]; inside = 0; n = 0; found = 0 }
          # Emit the buffered akr section, with the command key rewritten in place or
          # added at the top when the section did not carry one.
          function emit(  i) {
            if (found == 0) print "command = \"" cmd "\""
            for (i = 1; i <= n; i++) print buf[i]
            inside = 0; n = 0; found = 0
          }
          /^\[mcp_servers\.akr\][ \t]*$/ { print; inside = 1; n = 0; found = 0; next }
          inside == 1 && /^\[/ { emit(); print; next }
          inside == 1 && /^command[ \t]*=/ {
            buf[++n] = "command = \"" cmd "\""; found = 1; next
          }
          inside == 1 { buf[++n] = $0; next }
          { print }
          END { if (inside == 1) emit() }
        ' "$CODEX_CFG" > "$CODEX_TMP"
        mv "$CODEX_TMP" "$CODEX_CFG"
      fi
    else
      log "Appending [mcp_servers.akr] to Codex config"
      if [[ "$DRY_RUN" -eq 1 ]]; then log "DRY-RUN: append AKR MCP config to $CODEX_CFG"; else printf '\n[mcp_servers.akr]\ncommand = "%s"\n' "$AKR_MCP_BIN" >> "$CODEX_CFG"; fi
    fi
  fi
fi

# Update ~/.config/opencode/opencode.jsonc
if [[ "$DO_OPENCODE" -eq 1 ]]; then
  OPCFG="$HOME/.config/opencode/opencode.jsonc"
  if [[ ! -f "$OPCFG" ]]; then
    echo "warning: OpenCode config not found: $OPCFG (skipping OpenCode update)"
  else
    # A missing jq skips one integration; it does not abort the run. This script is meant
    # to be re-run after every build, and the steps below it — the agent instruction
    # files, the Claude registration — must not be hostage to an optional dependency.
    if ! command -v jq >/dev/null 2>&1; then
      echo "warning: jq not found; skipping OpenCode config (install jq, or pass --no-opencode to silence this)" >&2
    else
      log "Updating OpenCode MCP section"
      TMPFILE="$(mktemp)"
      if [[ "$DRY_RUN" -eq 1 ]]; then log "DRY-RUN: update OpenCode MCP section"; else jq --arg cmd "$AKR_MCP_BIN" '.mcp.akr = { type: "local", command: [ $cmd ], enabled: true }' "$OPCFG" > "$TMPFILE" && mv "$TMPFILE" "$OPCFG"; fi
    fi
  fi
fi

# Install/refresh the AKR section of the global agent instruction files.
#
# The section lives between HTML comment markers. Everything outside them is yours --
# your own notes, sections other tools own like CodeGraph's -- and is copied through
# byte for byte, including its line endings.
#
# Three rules make re-running this safe:
#
#   1. Idempotent. If the file already says what this script would write, it is left
#      alone entirely: not rewritten, not re-timestamped, and no .bak. Refreshing the
#      MCP binaries therefore does not touch your instruction files at all.
#   2. Collapsing. Any number of marked blocks -- however they got there -- become one,
#      at the position of the first. Earlier versions re-expanded every block on every
#      run, so once a file had two it kept two forever.
#   3. Non-destructive. An unbalanced marker pair is refused, not guessed at. There is no
#      way to know where a block with no end marker was meant to stop, and the earlier
#      version answered "the end of the file", silently deleting everything after it.
AGENT_SECTION_FILE="$REPO_DIR/scripts/agent-section.md"
AGENT_BEGIN="<!-- AKR_START -->"
AGENT_END="<!-- AKR_END -->"

# Whole-line markers only. `grep -cF` used to count a *mention* of the token in
# prose — this repo's CLAUDE.md documents the markers in a sentence — as a real
# block, then the rewriter (which requires the marker to *be* the line) found
# none and appended a duplicate. Count the same way the rewriter recognises them.
count_whole_line_markers() {
  local file="$1" marker="$2"
  MARKER="$marker" awk '
    function bare(s) { sub(/\r$/, "", s); gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
    BEGIN { n = 0 }
    { if (bare($0) == ENVIRON["MARKER"]) n++ }
    END { print n }
  ' "$file"
}

install_agent_section() {
  local target="$1"
  local parent
  parent="$(dirname "$target")"
  if [[ ! -d "$parent" ]]; then
    log "No $parent; skipping $target"
    return 0
  fi
  if [[ ! -f "$AGENT_SECTION_FILE" ]]; then
    echo "error: agent section source missing: $AGENT_SECTION_FILE" >&2
    return 1
  fi

  local existed=0 begins=0 ends=0
  if [[ -f "$target" ]]; then
    existed=1
    begins="$(count_whole_line_markers "$target" "$AGENT_BEGIN")"
    ends="$(count_whole_line_markers "$target" "$AGENT_END")"
  fi
  if [[ "$begins" -ne "$ends" ]]; then
    echo "warning: $target has $begins '$AGENT_BEGIN' and $ends '$AGENT_END' markers;" >&2
    echo "         leaving it untouched. Balance the pair (or delete the stray marker)" >&2
    echo "         and re-run; this script will not guess where the block ends." >&2
    return 0
  fi

  # Match the line ending the file already uses, so a CRLF file does not come back as a
  # whole-file diff -- the most annoying way for a setup script to touch a file you
  # maintain by hand.
  #
  # Counted with `tr`, which sees bytes. Do not use grep: under MSYS, `grep -c $'\r$'`
  # reports a match on every line of a file that contains no CR byte at all.
  local cr=""
  if [[ "$existed" -eq 1 ]] && [[ "$(tr -dc '\r' < "$target" | wc -c)" -gt 0 ]]; then
    cr=$'\r'
  fi

  local source=/dev/null
  [[ "$existed" -eq 1 ]] && source="$target"
  local tmp
  tmp="$(mktemp)"
  BEGIN="$AGENT_BEGIN" END="$AGENT_END" SECTION="$AGENT_SECTION_FILE" CR="$cr" awk '
    BEGIN {
      begin = ENVIRON["BEGIN"]; end = ENVIRON["END"]; cr = ENVIRON["CR"]
      inside = 0; emitted = 0
    }
    # A marker is recognised wherever it sits on its own line, whatever the indentation
    # and whatever the line ending. The previous version demanded column 1, so an
    # indented marker was invisible to the rewriter but visible to the grep that chose
    # the log line -- it announced a refresh and appended a duplicate.
    function bare(s) { sub(/\r$/, "", s); gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
    function emit_block(   line) {
      printf "%s%s\n", begin, cr
      while ((getline line < ENVIRON["SECTION"]) > 0) {
        sub(/\r$/, "", line)
        printf "%s%s\n", line, cr
      }
      close(ENVIRON["SECTION"])
      printf "%s%s\n", end, cr
    }
    {
      tag = bare($0)
      if (tag == begin) {
        inside = 1
        if (emitted == 0) { emit_block(); emitted = 1 }
        next
      }
      if (tag == end) { inside = 0; next }
      # Reprint with the chosen ending rather than passing $0 straight through:
      # MSYS awk strips a trailing CR on read, so a plain `print` would quietly
      # convert a CRLF file to LF. Stripping first keeps this right on the awks
      # that do preserve it.
      if (inside == 0) { line = $0; sub(/\r$/, "", line); printf "%s%s\n", line, cr }
      next
    }
    END {
      if (emitted == 0) {
        if (NR > 0) printf "%s\n", cr
        emit_block()
      }
    }
  ' "$source" > "$tmp"

  if [[ "$existed" -eq 1 ]] && cmp -s "$tmp" "$target"; then
    rm -f "$tmp"
    log "Agent section already current, leaving in place: $target"
    return 0
  fi

  local action="Appended"
  [[ "$begins" -gt 0 ]] && action="Refreshed"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    rm -f "$tmp"
    log "DRY-RUN: would change the agent section in $target ($action)"
    return 0
  fi

  if [[ "$existed" -eq 1 ]]; then
    cp "$target" "$target.bak"
    mv "$tmp" "$target"
    log "$action agent section: $target (previous copy at $target.bak)"
  else
    mv "$tmp" "$target"
    log "$action agent section: $target (new file)"
  fi
}

# Refresh a file only when it already opted in with markers. Unlike
# install_agent_section, this will not append a block to a project AGENTS.md
# that does not have one.
refresh_marked_agent_section() {
  local target="$1"
  [[ -f "$target" ]] || return 0
  local begins ends
  begins="$(count_whole_line_markers "$target" "$AGENT_BEGIN")"
  ends="$(count_whole_line_markers "$target" "$AGENT_END")"
  if [[ "$begins" -gt 0 && "$begins" -eq "$ends" ]]; then
    install_agent_section "$target"
  fi
}

if [[ "$DO_AGENTS" -eq 1 ]]; then
  install_agent_section "$HOME/.claude/CLAUDE.md"
  install_agent_section "$HOME/.codex/AGENTS.md"
  install_agent_section "$HOME/.config/opencode/AGENTS.md"
  # Project files that already carry the marked block, so a setup run from
  # the workspace rewrites the same short protocol rather than leaving a
  # stale copy in AGENTS.md.
  refresh_marked_agent_section "$PWD/AGENTS.md"
  refresh_marked_agent_section "$PWD/CLAUDE.md"
fi

# Register Claude MCP server
if [[ "$DO_CLAUDE" -eq 1 ]]; then
  if ! command -v claude >/dev/null 2>&1; then
    echo "warning: claude binary not found; skipping Claude registration"
  else
    if claude mcp get akr >/dev/null 2>&1; then
      log "Claude already has an AKR MCP server configured"
    else
      log "Registering AKR MCP in Claude (user scope)"
      run claude mcp add --scope user akr "$AKR_MCP_BIN"
    fi
  fi
fi

log "Done. Restart Codex, Claude, and OpenCode to load updated MCP config."
