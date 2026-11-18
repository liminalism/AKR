# AKR — Agent Knowledge Records

AKR is a typed, versioned knowledge ledger for software projects worked on by
humans and AI agents. It gives project decisions, requirements, observations,
evidence, questions, and work items stable identities, lifecycle states,
scope, typed relations, freshness, and Git provenance.

Version 0.6.5 adds [local-only secrets and opaque citations](docs/18-private-secrets.md):
keep developer paths and private values off Git while public ledger text renders `[*]`.

The Rust workspace currently includes the compiler and validator, the `akr`
CLI, the `knowledge.*` MCP server, full-text search and migration tooling, and
a read-only native desktop review workbench. The project is under active
development; the remaining platform and release acceptance checks are tracked
in the AKR ledger rather than inferred from this README.

## What AKR does

AKR treats project knowledge as a small build system:

1. Parse canonical `.akr` source files.
2. Type-check record kinds, slots, lifecycle states, relations, and scopes.
3. Link references and resolve record heads and supersession chains.
4. Compute freshness, impact, and Git facts.
5. Build a disposable search index, generated Markdown views, and `akr.lock`.

The build is deterministic and does not use a language model. Agents can draft
or rank material, but authority, resolution, validation, freshness, and
acceptance remain compiler decisions.

The repository separates four layers:

| Layer | Contents | Authority | Writer |
| --- | --- | --- | --- |
| Scratch | `.agent/scratch/` working notes | Disposable | Agents |
| Sources | Registered outside advice in `sources/` | Non-authoritative and content-hashed | `akr source` |
| Ledger | Typed records in `.akr/` | Canonical | Validated CLI/MCP operations |
| Views | `docs/generated/` projections | Derived | `akr build` only |

## Current capabilities

### CLI and compiler

The `akr` binary supports workspace initialization, canonical formatting,
validation, builds, generated views, record lookup and search, deterministic
context assembly, impact and freshness analysis, migration/import review,
immutable source registration, evidence, papercuts, and validated record
authoring. It also provides a staged change transaction that connects AKR work
records to a Git commit without making Git history part of the ledger itself.

```text
akr init
akr check
akr build
akr context <planning-key>
akr search "freshness"
akr view active-work
akr review-queue
```

Run `akr --help` for the complete command list. Global options include
`--dir`, `--strict`/`--lenient`, `--format text|json`, `--at <commit>`, and
`--today <date>`.

### MCP server

`akr-mcp` exposes the same implementation over JSON-RPC 2.0 on stdio. The
`knowledge.*` tools cover context, search, get, impact, source inspection,
evidence, validation, and validated writes. The read-only surface can be
selected when a session should not receive authoring tools; optional accounting
records call sizes, budgets, and durations.

```text
akr-mcp --surface read
akr-mcp --surface full --accounting .akr/mcp-accounting.jsonl
```

The CLI and MCP server share `akr-core` and the CLI library, so they use the
same parsing, resolution, validation, freshness, and context behavior.

### Native review workbench

`akr-gui` is a read-only native desktop application for reviewing one or more
local AKR workspaces. It loads an immutable review snapshot and presents:

- planning hierarchy and record navigation;
- record bodies, typed relations, claims, acceptance checks, and provenance;
- freshness, diagnostics, Git metadata, and review counts;
- deterministic filtering and bounded relationship neighborhoods;
- independent tabs for multiple workspace paths.

Launch it with the current workspace, or pass several workspace paths as
arguments:

```text
cargo run -p akr-gui -- .
cargo run -p akr-gui -- path\to\project-a path\to\project-b
```

In the workbench, `/` edits the filter; `P` and `K` switch tree modes; `D`
opens the dashboard; `I` shows record details; `L` shows relations; `G` shows
Git metadata; `R` reloads; `Tab` changes workspace tabs; and the arrow keys
navigate records. The presentation layer is deliberately small and native,
with deterministic software rendering rather than a web UI.

## Quick start for contributors

Requirements are Rust `1.94` or newer and a Git checkout. From the repository
root:

```text
cargo build --workspace
cargo test --workspace --all-targets
cargo fmt --all -- --check
akr build --check
```

The native workbench and its focused tests can be checked independently:

```text
cargo check -p akr-gui --all-targets --locked
cargo test -p akr-gui --all-targets --locked
cargo test -p akr-cli review_snapshot
```

The GitHub Actions workflow in `.github/workflows/akr-gui.yml` runs the GUI
check and test jobs on Ubuntu and Windows. Distribution helpers live in
`scripts/`, including the Windows and Unix MCP setup scripts and the
distribution verification script.

For AKR-governed changes, consult the relevant context first, use a change
transaction for the staged snapshot, and validate before handoff:

```text
akr change begin --kind <kind> --summary "<imperative>" --primary <work-key>
git add <exact-paths>
akr change prepare --staged
akr git commit
```

Never edit `sources/` or `docs/generated/` by hand. Registered sources are
immutable, and generated views are recreated by `akr build`.

## Repository map

| Path | Purpose |
| --- | --- |
| `crates/akr-core` | Data model, parser, formatter, validation, resolver, freshness, index, and views |
| `crates/akr-cli` | `akr` binary, workspace session, operations, and review snapshots |
| `crates/akr-mcp` | `knowledge.*` MCP server over stdio |
| `crates/akr-gui` | Native read-only review workbench |
| `.akr/` | Canonical project ledger, lock, and disposable caches |
| `docs/` | Normative design documents and generated projections |
| `spec/` | Grammar, schemas, vocabulary, and diagnostic registries |
| `examples/` | Worked example and synthetic history |
| `fixtures/` | Parser, formatter, and validator conformance corpus |
| `prompts/` | Prompts for drafting importable material |
| `scripts/` | Setup, test, and distribution helpers |

The design documentation is organized from overview and architecture through
the data model, syntax, validation, compiler pipeline, CLI, MCP, context,
freshness, projections, migration, roadmap, and glossary. Start at
[`docs/00-overview.md`](docs/00-overview.md), then consult the specific
contract that governs a change.

## Non-goals

AKR is not a generic wiki, Markdown-plus-frontmatter convention, RDF authoring
surface, retrieval database, or newest-wins conflict resolver. It does not
silently delete knowledge, use line numbers as record identity, or claim that
every agent output is durable. A model may help draft or search; it does not
decide what the project believes.

## License

Not yet chosen. Workspace packages are currently marked `UNLICENSED` and are
not published.
