# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

AKR (Agent Knowledge Records) is a versioned project-knowledge ledger: a typed record language (`.akr` files), a deterministic compiler (`akr build`: parse → type-check → link → resolve → index → emit), and generated Markdown views. It is framed as **a compiler and build system for project knowledge, not a retrieval store** — the build fails on contradictions, output is byte-reproducible, and no language model participates in any pipeline stage.

Note: `README.md` still says "design only"; the implementation actually exists through roadmap phase P8, plus P10 (the immutable source library and its chunk index) and P11 (the AKR ↔ git change protocol). See `docs/13-implementation-roadmap.md`.

Two adjacent systems share the repo and are deliberately *not* the ledger:

- **`sources/`** — an immutable, content-hashed library of outside advice (D-031, `docs/15-external-sources.md`). Registering a document creates no records; editing one is `AKR-S021`. `.akr/cache/sources.sqlite` chunks and ranks it on its own cache generation. Records cite it by `document` + byte range, never by chunk id.
- **The change transaction** (D-032, `docs/16-change-protocol.md`) — a per-worktree file under the git dir that binds a commit to the work it advances. The staged tree is the synchronisation boundary, and the durable link is commit trailers, not stored commit hashes.

## Commands

```powershell
cargo build                              # build the workspace
cargo test                               # all tests, all crates
cargo test -p akr-core                   # one crate
cargo test -p akr-core --test v_rules    # one integration-test file
cargo test -p akr-core --test v_rules some_test_name   # one test
cargo clippy --all-targets               # lints (workspace: clippy::all = warn, missing_docs = warn, unsafe forbidden)
cargo fmt
python tools/check-design.py             # design-set coherence checker over docs/spec/fixtures
```

The `fts5` feature (default on, forwarded explicitly by all four crates — each takes its AKR dependencies with `default-features = false` and re-exports the feature, so that `--no-default-features` is not silently undone by cargo's feature unification) controls whether stage E builds the full-text index; `--no-default-features` exercises the no-FTS path that `akr search` handles with `AKR-I022`. `akr source search` degrades the same way. Tests that need a ranker carry `#[cfg(feature = "fts5")]`; `tests/search_degraded.rs` holds the claims for its absence.

If you are in a sandbox with no Rust toolchain and `static.rust-lang.org` blocked, `scripts/fetch-rust-sandbox.sh` fetches one from the npm mirror of the official component tarballs, and `scripts/run-tests-sliced.sh` runs the test binaries a few at a time for environments that cap a command's wall clock.

`scripts/setup-akr-mcp.sh` and its PowerShell mirror install the binaries and register the MCP server with Codex, OpenCode and Claude. They also install `scripts/agent-section.md` — the short, workspace-agnostic "how to use AKR" brief — into the global agent instruction files (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`, `~/.config/opencode/AGENTS.md`), and into `$PWD/AGENTS.md` / `$PWD/CLAUDE.md` when those files already contain the markers, between `<!-- AKR_START -->` and `<!-- AKR_END -->`. Edit that file, not the installed copies: re-running either script rewrites the marked block in place and leaves everything around it, including sections other tools own, untouched. The rewrite is idempotent — if the marked block already matches `agent-section.md` the file is not written at all, so refreshing the binaries never touches an instruction file you maintain by hand, and no `.bak` accumulates. Repeated blocks collapse to one, and the file's existing line endings are preserved. An unbalanced marker pair is refused rather than guessed at, because there is no way to know where a block with no end marker was meant to stop. `--no-agents` / `-NoAgents` skips the step entirely. This repo's own `AGENTS.md` is the fuller protocol and is not what gets installed.

## Workspace layout

- **`crates/akr-core`** — the model (`model/`), lexer/parser/formatter (`syntax/`), validation rules V-001..V-025 (`validate/`), resolver, git freshness (`freshness/`, `git/`), context assembly (`context/`), SQLite index + renderers (`render/`), lock file (`lock/`), atomic write ops (`ops/`).
- **`crates/akr-cli`** — the `akr` binary, and a library (`akr_cli`) so the MCP server reuses the exact same command implementations.
- **`crates/akr-mcp`** — MCP server (`knowledge.*` tools over stdio). Contains **no ledger logic**: it is only JSON-RPC framing, tool schemas, argument translation, and error mapping over `akr-cli` functions. `tests/differential.rs` enforces that CLI and MCP produce identical results.

**Dependency policy is deliberate and strict**: the only runtime dependency in the workspace is `rusqlite` (bundled) in `akr-core`, sanctioned solely for stage E. Argument parsing, JSON, and JSON-RPC framing are hand-written. Do not add dependencies. Nothing outside `akr-core`'s store module may open the SQLite cache (D-019) — CLI and MCP reach it only through `akr-core`.

## The spec set is the source of truth

The `docs/` and `spec/` trees are a normative specification the code implements, with an explicit precedence: **`docs/DECISIONS.md` (D-001..D-025) wins over the numbered spec docs, which win over `docs/00-overview.md`**. Key entry points:

- `spec/tables/vocabulary.json` — machine-readable spine (kinds, lifecycles, slots, relations, rules); tests read it to check the code matches.
- `spec/exemplar.akr` — the **only** source of quotable syntax; it must round-trip byte-identically through parse+format.
- `spec/diagnostics/codes-lang.md` / `codes-runtime.md` — the diagnostic registries (`AKR-P/F/T/L/R` and `AKR-I/E/X/G/C/M`); every diagnostic code must be registered there.
- The "frozen spine" (DECISIONS.md, vocabulary.json, exemplar.akr, the diagnostics README, the worked-example MANIFEST) must not be edited without a recorded decision.

## Testing conventions

- **Fixtures are the executable spec.** `fixtures/{parse,format,validate}/` at the repo root drive conformance tests (`fixture_corpus.rs`, `fixtures_parse.rs`, etc.), resolved via `CARGO_MANIFEST_DIR/../../fixtures`. Err-fixtures pair with `.expected` files naming exact diagnostic codes (line numbers currently read-and-ignored). A rule with no fixture is considered not implemented.
- **Determinism is tested, not assumed**: anything that produces output gets a same-input-twice byte comparison and a shuffled-input-order byte comparison (`graph_determinism.rs`, `emit.rs`).
- `examples/save-your-skin/` is a frozen worked example (sources, lock, generated views, transcripts) used as an end-to-end test corpus; its inventory in `MANIFEST.md` is frozen.
- `--today` and `--at` exist so builds are reproducible in tests — the system clock and `HEAD` are the only ambient inputs, and both are overridable.

## Invariants that shape changes

- **Write pipeline atomicity** (`docs/07-cli.md` §4): every write command parses the ledger, applies the change in memory, validates the *resulting* ledger (stages A–D), canonically formats, and only then writes atomically. Failure writes nothing. Never add a code path that writes an unvalidated or unformatted record.
- **Exit codes are semantic**: 0 success, 1 ledger diagnostics, 2 usage, 3 environment. Staleness never changes an exit code (D-024).
- `docs/generated/**` views are build outputs — never hand-edit.
- One implementation for CLI and MCP: any behavior reachable via MCP must be reproducible from the command line.
- **Git is spawned only through `akr_core::git`** (`crates/akr-core/src/git/mod.rs`). Use `command()` rather than `Command::new("git")`: it suppresses the console window a child process would otherwise open on Windows, which is what made the desktop shell flash consoles. Every query goes through the `Repository` memo, so a repeated question costs no process; on Windows each spawn is ~30ms and freshness asks thousands. `Repository::shared` extends that memo across handles for a long-lived host (the MCP server), keyed on `HEAD` plus porcelain status — any change discards the whole memo rather than risk serving a stale fact. New git queries belong in that module, memoised if their answer cannot change while the repository does not.
