# Diagnostic Registry — Runtime Stages (`AKR-I/E/X/G/C/M/S`)

This registry defines every diagnostic raised by the runtime half of the toolchain:
index construction, view emission, context assembly, git and freshness, the command
line and workspace configuration, and migration. The language half —
`AKR-P/F/T/L/R` — lives in [`codes-lang.md`](codes-lang.md).

Scheme, severity meaning, rendered form, and numbering conventions are fixed by
[`README.md`](README.md) and D-013. This file adds nothing to the scheme; it enumerates
codes.

Two standing rules from `README.md` §6 govern the tables below:

- **Numbers are grouped in tens by topic, with gaps reserved.** The reserved ranges are
  named under each stage heading. A new code takes the next free number in its topic
  group, never a renumbering of an existing one.
- **Every registered code is cited by at least one specification document, and no
  specification document cites an unregistered code.** `tools/check-design.py` enforces
  both directions.

Staleness and `at_risk` are **not** diagnostics (D-024). No code below reports that a
record is stale. `AKR-G041` and `AKR-G042` are the opt-in exceptions: they report that the
operator asked for a clean review queue with `akr check --review-clean`, or for clean
scratch with `akr check --scratch-clean`, and did not get one. Both are facts about the
invocation, not about the ledger.

## Rule identifiers used by this registry

The `V-101`–`V-149` range belongs to the freshness, emission and context documents
(`spec/diagnostics/README.md` §7). The allocation in force is:

| Rules | Catalogue | Codes raised |
| --- | --- | --- |
| `V-101`–`V-104` | [`../../docs/10-freshness-and-git.md`](../../docs/10-freshness-and-git.md) §9 | `AKR-G011`, `G012`, `G021`, `G022`, `G023`, `G026`, `G031`, `G041` |
| `V-111`–`V-115` | [`../../docs/11-projections.md`](../../docs/11-projections.md) §11 | `AKR-E011`, `E012`, `E013`, `E014`, `E015`, `E021`, `E022` |
| `V-121`–`V-123` | [`../../docs/09-context-assembly.md`](../../docs/09-context-assembly.md) §9 | `AKR-X021`, `X022`, `X051`, `X052` |

`V-105`–`V-110`, `V-116`–`V-120` and `V-124`–`V-149` are unallocated. Codes with `—` in
the Rule column implement no rule: they report a fault in the invocation or the
environment rather than a violated invariant.

---

## `AKR-I` — Index

Stage E of the pipeline: building `.akr/cache/index.sqlite` from the resolved model.
Normative document: [`../../docs/06-compiler-pipeline.md`](../../docs/06-compiler-pipeline.md) §6.

Reserved groups: `I001`–`I009` cache I/O; `I011`–`I019` integrity of the written
index; `I021`–`I029` full-text index; `I031`–`I039` concurrency and rebuild policy.

Routine cache invalidation — a `schema_version` bump, a changed `source_graph_hash`, a
missing cache file — is **silent**. It causes a full rebuild and emits nothing. A
diagnostic in this range always means the cache could not be built, not that it had to
be rebuilt.

| Code | Title | Severity | Message template | Cause |
| --- | --- | --- | --- | --- |
| `AKR-I001` | index cache unreadable | error | `cannot read index cache at {path}: {io_error}` | The cache file exists but cannot be opened; the tool will not silently ignore an unreadable file it is about to overwrite. |
| `AKR-I002` | index cache write failed | error | `cannot write index cache at {path}: {io_error}` | The transaction that populates the cache could not commit. |
| `AKR-I003` | index cache directory not writable | error | `index cache directory {path} is not writable` | `.akr/cache/` is missing and cannot be created, or is read-only. |
| `AKR-I004` | index cache is not an AKR index | error | `{path} is a SQLite database but has no AKR meta table` | Some other database was placed at the cache path; deleting the wrong file is the operator's decision, not the tool's. |
| `AKR-I011` | index integrity check failed | error | `index integrity check failed after write: {detail}` | SQLite's own `PRAGMA integrity_check` did not return `ok` after the populate transaction. |
| `AKR-I012` | resolved head absent from index | error | `resolved head {key}/{revision} was not written to the index` | Internal invariant: every head in the resolved model has a row in `resolutions`. Indicates a defect in stage E, not in the ledger. |
| `AKR-I013` | index row count disagrees with the resolved model | error | `index holds {found} {table} rows; the resolved model holds {expected}` | Internal invariant check comparing the model to what landed in the cache. |
| `AKR-I021` | full-text index build failed | error | `cannot build records_fts: {sqlite_error}` | The SQLite build in use lacks FTS5, or the virtual table could not be populated. |
| `AKR-I022` | full-text query without a full-text index | error | `search requires a full-text index; this cache was built without FTS5` | `akr search` was run against a cache built by a binary without FTS5 support. |
| `AKR-I031` | index rebuild required but disabled | error | `index is stale and rebuilding is disabled` | A read command needed a rebuild while running under `--no-rebuild` (used in read-only checkouts). |
| `AKR-I032` | concurrent index write | error | `another akr process holds the index cache lock` | Two builds in the same workspace. The loser fails rather than corrupting the cache. |

---

## `AKR-E` — Emit

Stage F of the pipeline: rendering generated views and enforcing D-025. Normative
documents: [`../../docs/11-projections.md`](../../docs/11-projections.md),
[`../../docs/06-compiler-pipeline.md`](../../docs/06-compiler-pipeline.md) §7.

Reserved groups: `E001`–`E009` output location; `E011`–`E019` view currency (the CI
gate); `E021`–`E029` rendering; `E031`–`E039` view templates.

| Code | Title | Severity | Rule | Message template | Cause |
| --- | --- | --- | --- | --- | --- |
| `AKR-E001` | view output directory not writable | error | — | `view output directory {path} is not writable` | `view_output` names a path the build cannot create or write. |
| `AKR-E002` | view output path escapes the repository | error | — | `view_output {path} resolves outside the repository root` | A `..` or absolute `view_output` in `project.akr`. Generated output stays inside the repository so that D-025's commit rule is meaningful. |
| `AKR-E003` | unknown view | error | — | `unknown view {name}; known views are {list}` | `akr view <name>` naming something outside the catalogue of `docs/11-projections.md` §2. |
| `AKR-E011` | generated view is out of date | error | V-112 | `{path} differs from the view this build would emit ({n} differing lines)` | The committed view was hand-edited, or the ledger changed without a rebuild. This is the CI gate of D-025, raised by every `akr check`; `--views-current` selects the per-view report, not whether the comparison runs. |
| `AKR-E012` | generated view missing | error | V-112 | `{path} is missing; run akr build` | A catalogued view has no committed file. |
| `AKR-E013` | generated view banner malformed | error | V-113 | `{path} does not begin with a well-formed AKR banner` | The banner was edited, truncated, or moved off line 1. A view without a readable banner cannot be told from a hand-written document. |
| `AKR-E015` | generated view carries an older `tool:` stamp | warning | V-112 | `{path} was rendered by a different akr version; its content matches the ledger but its banner does not` | The only differing line is the banner's `tool:` field, so the view's content is current and only the stamp is stale — a migration artifact that disappears the first time anyone rebuilds. Exempt from `--strict` promotion for the same reason as `AKR-G004` and `AKR-G023`: no agent can clear it mid-session, and it says nothing about the knowledge. Any other difference is `AKR-E011` and stays fatal. |
| `AKR-E014` | unexpected file in the view output directory | error | V-114 | `{path} is in the view output directory but is not a generated view` | Something was added by hand to `docs/generated/`. The directory is owned by the build. |
| `AKR-E016` | generated view is behind uncommitted ledger changes | warning | V-112 | `{path} is behind uncommitted ledger changes in the working tree` | `AKR-E011`/`E012` demoted to a warning while the ledger itself is uncommitted: an agent mid-session has stale views by construction, and failing here would force a premature rebuild or a drop to `--lenient`. Once the ledger is committed the same difference is `AKR-E011`/`E012` again and stays fatal — this code exists only to distinguish "behind my own unsaved edits" from a shipped ledger whose committed views do not match it. |
| `AKR-E021` | record required by a view is absent | error | — | `view {name} requires {key}/{revision}, which is not in the resolved model` | Internal invariant: a view's source query selected a record the model does not hold. |
| `AKR-E022` | duplicate heading anchor in a view | error | V-115 | `view {name} renders two records to the heading anchor {anchor}: {key_a} and {key_b}` | Two selected records carry the same `title`. Headings come from `title` and never from prose, so titles must be distinguishable within a view. |
| `AKR-E031` | unknown view template | error | — | `view template {name} is not registered` | A project-local template named in `project.akr` does not exist. |
| `AKR-E032` | view template requests an undefined section | error | — | `template {name} requests section {section}, which view {view} does not define` | A template extension referencing a section outside the view's declared section set (`docs/11-projections.md` §10). |

---

## `AKR-X` — Context

Context assembly and search. Normative document:
[`../../docs/09-context-assembly.md`](../../docs/09-context-assembly.md).

Reserved groups: `X001`–`X009` the bundle anchor; `X011`–`X019` path filters;
`X021`–`X029` budgeting; `X031`–`X039` search; `X041`–`X049` output form;
`X051`–`X059` bundle invariants; `X099` a contained internal failure.

| Code | Title | Severity | Rule | Message template | Cause |
| --- | --- | --- | --- | --- | --- |
| `AKR-X001` | goal does not resolve | error | — | `--goal {ref} does not resolve to a record` | A misspelled key, or a key whose namespace is not declared. When the ledger has no records, the message instead says so and directs the caller to create its first planning record. |
| `AKR-X002` | goal is terminal | error | — | `--goal {key}/{revision} is in terminal state {state}` | Assembling a bundle around finished work is almost always a mistake; `akr get` still retrieves the record. |
| `AKR-X003` | goal kind cannot anchor a bundle | error | — | `--goal {key} is a {kind}; a bundle anchors on a milestone, work or track record` | The assembly algorithm's step 1 requires a planning record. |
| `AKR-X004` | pinned goal is not the head | error | — | `--goal {requested} is not the current head, which is {head}` | Context describes current work. A pin to the current head is normalized; an older revision remains available through `akr get --history`. |
| `AKR-X005` | goal selects an anchor | error | — | `--goal {ref} selects an anchor; a bundle anchors on a planning record` | Claim and check anchors are retrieval targets, not bundle roots. |
| `AKR-X011` | path filter is malformed | error | — | `--paths {glob}: {reason}` | A glob outside the D-008 subset — brace expansion, `!` negation, an unterminated `[` class, a partial `**`. Backslashes and an absolute in-repo path no longer reach this: they are normalized before validation. |
| `AKR-X012` | path filter matches nothing | warning | — | `--paths {glob} matches no path at {commit}` | Usually a typo. The bundle is still assembled without a path-derived section. |
| `AKR-X013` | path filter is outside the repository | error | — | `--paths {path}: outside the repository (root {root})` | An absolute path given to `--paths`/`paths` that, once native separators are normalized, does not lie inside the repository root. A path inside the root is converted to a repo-root-relative glob automatically; one outside it is rejected rather than silently mangled. |
| `AKR-X021` | context budget too small | error | V-123 | `budget of {budget} tokens cannot hold the mandatory sections ({required} tokens)` | Relations, contradictions and staleness warnings never truncate, so a budget below their size is unsatisfiable. |
| `AKR-X022` | context budget exhausted | warning | V-123 | `prose truncated in {n} records to fit a budget of {budget} tokens` | Normal operation on a large bundle; reported so that a reader knows what was dropped. |
| `AKR-X031` | search query is malformed | error | — | `search query: {reason}` | An unbalanced quote or an unsupported operator in the FTS query. |
| `AKR-X032` | search backend unavailable | error | — | `search backend {name} is unavailable: {reason}` | A configured ranker could not be reached. |
| `AKR-X033` | ranking fell back to lexical | warning | — | `ranking model {name} unavailable; ranked lexically` | Ranking is advisory (D-020), so a fallback changes the order of results and nothing else. Reported so the order is explicable. |
| `AKR-X041` | unknown bundle format | error | — | `unknown bundle format {name}; known formats are text, json` | `akr context --format` given something outside the two documented forms. |
| `AKR-X042` | server and workspace disagree on vocabulary | warning | — | `this akr-mcp was built against vocabulary {server} and the workspace was last built with vocabulary {workspace}` | Attached by `akr-mcp` to a failing call when its own vocabulary version differs from the one in `.akr/akr.lock`. An installed server older than the workspace rejects records it has never heard the slots of, which reads as a ledger fault; this names it and says to reinstall *and reconnect*, since a running process keeps the binary it started with (D-034). |
| `AKR-X051` | contradiction not surfaced | error | V-121 | `bundle omits contradiction between {key_a} and {key_b}` | Internal invariant: a declared contradiction touching a bundled record must always appear in the contradictions section, including when one side is superseded (D-023). |
| `AKR-X052` | excluded record present in a bundle | error | V-122 | `bundle includes {key}/{revision}, which is {reason}` | Internal invariant: superseded, terminal and archived records never enter an ordinary bundle (`docs/09-context-assembly.md` §5). |
| `AKR-X099` | contained internal failure | error | — | `the AKR tool implementation failed unexpectedly; the server contained the failure` | A panic inside one `akr-mcp` tool call, caught at the request boundary (`docs/08-mcp.md` §5). The write pipeline is atomic, so a contained panic leaves nothing half-written; the call is retryable once and the stdio server stays up for the next request. It is always a bug in AKR, never in the call.

---

## `AKR-G` — Git and freshness

Repository access, commit ancestry, watched paths, review dates, impact. Normative
document: [`../../docs/10-freshness-and-git.md`](../../docs/10-freshness-and-git.md).

Reserved groups: `G001`–`G009` repository access; `G011`–`G019` commit references;
`G021`–`G029` watches; `G031`–`G039` review dates; `G041`–`G049` review gates.

| Code | Title | Severity | Rule | Message template | Cause |
| --- | --- | --- | --- | --- | --- |
| `AKR-G001` | not a git repository | error | — | `{path} is not inside a git repository` | AKR derives currency from history; without history there is no freshness model. Exit status 3. |
| `AKR-G002` | git invocation failed | error | — | `git {subcommand} failed: {stderr}` | The repository is present but unreadable — a corrupt object, a permission failure, a missing `git` binary. |
| `AKR-G003` | history is shallow | error | — | `repository history is shallow; cannot decide ancestry of {commit}` | A `--depth`-limited clone cannot answer the descendant question D-016 and D-024 depend on. Fetch the full history, or run with `--no-freshness`. |
| `AKR-G004` | working tree is dirty | warning | — | `{n} watched paths have uncommitted changes` | Freshness is computed against committed history, so uncommitted edits are invisible to it. Reported so the reader is not misled. |
| `AKR-G011` | `observed_at` commit not in the repository | error | V-101 | `{key}/{revision}: observed_at {commit} is not present in this repository` | A rebased or force-pushed branch, or a commit from another repository. |
| `AKR-G012` | `observed_at` is not an ancestor of HEAD | warning | V-101 | `{key}/{revision}: observed_at {commit} is not an ancestor of {head}` | The observation was made on a branch that HEAD does not contain. Staleness is not computable for it, so it is neither fresh nor stale — it is reported. |
| `AKR-G013` | unknown revision argument | error | — | `{argument}: {revision} is not a commit in this repository` | `--at`, or either end of `--git-diff A..B`. |
| `AKR-G014` | malformed explicit commit-rewrite map | error | — | `.akr-commit-map line {line}: {reason}` | A repository's reviewed old-to-new commit mapping cannot be parsed. AKR refuses to guess or silently ignore it. |
| `AKR-G021` | malformed watch glob | error | V-102 | `{key}/{revision}: watches {glob}: {reason}` | Same glob subset as `AKR-X011`, checked at the record rather than the flag. |
| `AKR-G022` | watch glob matches nothing | warning | V-102 | `{key}/{revision}: watches {glob} matches no path at {head}` | The watched code moved or was deleted; the record can no longer become stale by that glob, which is silent rot. |
| `AKR-G023` | scope glob matches nothing | warning | V-102 | `{key}/{revision}: scope path {glob} matches no tracked path at {head}` | Usually a copied rendered `path ` prefix, typo, or moved path. Intentionally gitignored targets are exempt. It is visible but does not make an in-progress strict check fail. |
| `AKR-G026` | `project.akr` declares a tracked tree untracked | warning | V-102 | `project.akr declares {glob} untracked, but {root} is tracked at {head}` | The `untracked` block exists so a scope may name a tree the project deliberately keeps out of git (large binary art, a vendored corpus) without reporting as a dead glob. It silences every scope beneath it, so it is the one declaration whose being wrong is invisible by construction — this checks it against git rather than taking the author's word. Drop the declaration, or narrow it to the part git really does not hold. |
| `AKR-G024` | a recorded command names no test that exists | warning | V-105 | `{key}/{revision}: check {id} runs {command}, and {token} names nothing in tracked source` | An acceptance check or evidence `command` whose test-name filter matches nothing in tracked source. `cargo test <filter>` with a filter that matches nothing prints "0 passed" and EXITS 0, so the gate is satisfied having executed nothing. A warning, not an error: every phantom found in a 16-workspace census had prior code history, so this is a test that was removed rather than a name that was never real, and on an `active` record it may be a specification. `::` tokens resolve segment by segment because cargo filters are substring matches against the full test path. `.akr/`, `docs/generated/` and `.agent/` are excluded from the search — the first two are the ledger quoting itself and the third holds captured run logs from when the test still existed. The case for this check is that AKR already accepts the same discipline one level down: `every_freshness_code_is_registered_in_the_runtime_registry` compares what the code raises against what this file documents, and refuses a half-landed change. A recorded command naming a test that no longer exists is that defect one level up. It is also structural rather than careless, and the sharpest evidence is from building the check itself: the first draft of these tests passed four of five cases while the detector never ran at all, because the fixture had no resolved commit and the whole check was skipped — four green assertions proving nothing, in the test suite for the defect that is exactly four green assertions proving nothing. A test for any check that consults git must therefore build its own source and **commit** it; the shipped fixtures hold placeholder content and have no commit, so borrowing a token from them asserts nothing. |
| `AKR-G025` | a recorded command's exit status cannot be trusted | warning | V-105 | `{key}/{revision}: check {id} runs {command}, and {why}` | The command is recorded as the gate, and a check is satisfied by its exit status — but a shell pipeline reports only its LAST stage's status, `|| true` forces success, and an HTML-escaped `&amp;&amp;` is not `&&`. Any of these makes the gate unfalsifiable regardless of whether the test names are real. Demonstrated three times in one afternoon by three separate agents in the 2026-09 sweep, once within an assertion of sealing a fabricated pass into an evidence record. |
| `AKR-G031` | `review_after` precedes `created_at` | warning | V-103 | `{key}/{revision}: review_after {date} precedes created_at {date}` | Almost always a typo; the record is stale from the moment it is written. |
| `AKR-G041` | review queue is not empty | error | V-104 | `review queue holds {stale} stale and {at_risk} at-risk records` | Raised **only** under `akr check --review-clean`. Staleness itself is a build fact and never a diagnostic (D-024); this code reports an unmet request made on the command line. |
| `AKR-G042` | scratch is not clean | error | — | `scratch holds {bytes} in {n} prunable entries` | Raised **only** under `akr check --scratch-clean`. What is in `.agent/scratch/` is a fact about the working tree and never a ledger diagnostic, exactly as staleness is not (D-024, D-036); this code reports an unmet request made on the command line. An entry named in `.agent/scratch/KEEP` is never counted, whatever its age. |

---

## `AKR-C` — CLI and configuration

Invocation, workspace discovery, `project.akr`, and the write pipeline's abort path.
Normative document: [`../../docs/07-cli.md`](../../docs/07-cli.md).

Reserved groups: `C001`–`C009` invocation (exit status 2); `C011`–`C019` workspace
discovery (exit status 3); `C021`–`C029` `project.akr` content; `C031`–`C039` write
operations; `C041`–`C049` output selection.

Lock-file integrity is deliberately **not** in this range: sealing is checked while
resolving heads, so `AKR-R051` and `AKR-R052` belong to Writer A's resolve range
(`spec/diagnostics/README.md` §2).

| Code | Title | Severity | Message template | Cause |
| --- | --- | --- | --- | --- |
| `AKR-C001` | unknown command | error | `unknown command {name}` | Exit status 2. The message lists the nearest known command by edit distance. |
| `AKR-C002` | unknown flag | error | `unknown flag {flag} for command {command}` | Exit status 2. |
| `AKR-C003` | missing required argument | error | `{command} requires {argument}` | Exit status 2. |
| `AKR-C004` | invalid flag value | error | `{flag}: {value} is not {expectation}` | Exit status 2. Covers non-enum values for `--format`, non-integer budgets, and malformed commit arguments given to flags other than the git ones. |
| `AKR-C005` | mutually exclusive flags | error | `{flag_a} cannot be combined with {flag_b}` | Exit status 2. `--strict` with `--lenient`, `--at` with `--git-diff`. |
| `AKR-C011` | no AKR workspace found | error | `no .akr directory found in {path} or any parent` | Exit status 3. |
| `AKR-C012` | `project.akr` missing | error | `{path}/.akr/project.akr is missing` | Exit status 3. A workspace without a project file has no declared namespaces, so nothing can validate. |
| `AKR-C013` | workspace already initialised | error | `{path}/.akr already exists` | `akr init` never overwrites. |
| `AKR-C021` | unknown key in `defaults` | error | `unknown defaults key {name}` | Configuration typos are silent misconfiguration; the set of keys is closed. |
| `AKR-C022` | duplicate namespace declaration | error | `namespace {name} is declared twice in project.akr` | |
| `AKR-C023` | project name missing or malformed | error | `project name {value} is not in key-segment form` | The `project` header line of `project.akr` (D-005). |
| `AKR-C031` | write aborted; the write introduces a diagnostic | error | `write aborted: this write would introduce {n} diagnostic(s); nothing was written` | Every write goes parse → validate → canonical-format → write. A failure at any point leaves the working tree untouched. Diagnostics the ledger already had are reported, not charged to this write (D-039). |
| `AKR-C032` | write would modify a sealed revision | error | `{key}/{revision} is sealed ({state}); create a new revision with akr revise` | The command-line refusal that precedes the build-time check `AKR-R051` (D-015). |
| `AKR-C033` | write target is not the head revision | error | `{key}/{revision} is not the head of {key}` | `akr revise` and friends operate on heads; editing history is a source-file operation done deliberately. |
| `AKR-C034` | a source changed on disk while the write was being prepared | error | `write aborted: {path} changed on disk while this write was being prepared; nothing was written` | The pipeline reads every source, edits in memory, and writes each touched file whole, so a source that changes in between would be overwritten from a stale snapshot. Writers take an advisory lock on `.akr/cache/write.lock` and therefore queue rather than collide; this fires for what a lock cannot cover — an editor, a `git checkout`, or a filesystem with no working lock. Nothing is written; re-read and retry. |
| `AKR-C041` | command has no JSON form | error | `{command} does not support --format json` | Exit status 2. Applies to `fmt` and `init`, whose output is a file-system effect rather than data. |
| `AKR-C042` | workspace path not readable or not writable | error | `cannot {read\|write\|create} {path}: {reason}` | Exit status 3. The filesystem refused, so this is a checkout problem rather than a ledger problem — a read-only mount, a permission, a full disk. |
| `AKR-C043` | handoff object not found | error | `no handoff object {id}` | Exit status 3. `akr handoff open`, `expand`, `reveal`, `verify` or `result` against a packet id this workspace does not hold. Capsules, packets and results are disposable and gitignored (D-040, D-041), so an id from another checkout, another worktree, or one already discarded reaches this rather than a stale read. `akr handoff list` names what is there. |
| `AKR-C044` | no handoff session is open | error | `no handoff session is open in this workspace` | Exit status 3. `akr handoff session show`, `end`, `results` (with no packet named) or `coverage` in a workspace where nothing opened one. A session is what packets inherit and what coverage aggregates over, so these questions have no subject without it; `akr handoff session begin --request "..."` opens one. Cutting a packet without a session is *not* an error — it produces a thin packet, and the command says so. |

---

## `AKR-M` — Migration

`akr import` and the legacy disposition workflow of D-022. Normative document:
[`../../docs/12-migration.md`](../../docs/12-migration.md).

Reserved groups: `M001`–`M009` the source document; `M011`–`M019` extraction;
`M021`–`M029` provenance; `M031`–`M039` the tracking work record; `M041`–`M049` the
import profile.

| Code | Title | Severity | Message template | Cause |
| --- | --- | --- | --- | --- |
| `AKR-M001` | import source not found | error | `import source {path} does not exist` | |
| `AKR-M002` | unsupported import source | error | `{path}: {extension} is not an importable format` | 0.1 imports Markdown and plain text only. |
| `AKR-M011` | no durable claim extracted | warning | `{path}: no durable claim extracted` | The document is entirely status chatter, or the extractor failed. Either way a human decides; the tool does not archive a document it read nothing from. |
| `AKR-M012` | imported key collides | error | `{key} already exists; imported records may not overwrite ledger records` | Import only ever adds `proposed` revisions of new keys, or new revisions of existing keys through `akr revise`. |
| `AKR-M013` | imported key namespace not declared | error | `{key}: namespace {namespace} is not declared in project.akr` | The import's key-suggestion step drew a namespace the project does not have. |
| `AKR-M021` | imported record lacks legacy provenance | error | `{key}/{revision} was produced by import but has no source block with kind legacy` | Provenance is the only thing that makes an import auditable. |
| `AKR-M022` | legacy source path does not exist | warning | `{key}/{revision}: source path {path} does not exist at {head}` | The legacy document was moved or deleted after import; the excerpt is now unverifiable. |
| `AKR-M031` | no tracking record for an imported document | error | `{path} has imported records but no tracking work record` | D-022 requires one `work` record per migrated document, with one acceptance check per durable claim. |
| `AKR-M032` | legacy document archived while tracking incomplete | error | `{path} is archived but {key}/{revision} is in state {state}` | The archive step waits for `completed`, which by V-020 waits for every check to be satisfied. |
| `AKR-M041` | import produced warnings under the strict profile | error | `import produced {n} warnings; rerun with --lenient after reviewing them` | `--lenient` is the only place warnings are downgraded (D-013), and it is opt-in per invocation. |
| `AKR-M042` | imported record is not proposed | error | `{key}/{revision} was produced by import in state {state}; imports land as proposed` | Everything an importer writes is a proposal for human review. Questions land `open`, the inquiry class's only initial state (`docs/12` §3). |

---

## `AKR-S` — Source verification

Source manifest and immutable source catalog checks.
Normative document: [`../../docs/06-compiler-pipeline.md`](../../docs/06-compiler-pipeline.md).

Reserved groups: `S021`–`S029` catalog and immutable-source checks.

| Code | Title | Severity | Rule | Message template | Cause |
| --- | --- | --- | --- | --- | --- |
| `AKR-S021` | immutable source manifest invalid | error | — | `{path}: source manifest error: {detail}` | Source verification failed while loading `source` stage metadata; this blocks any further build work until corrected. |
| `AKR-S022` | record source citation does not resolve | error | — | `{key}/{rev}: {detail}` | A record's `source` block names a registered document that is absent, a byte range outside it, a range that is not on a character boundary, an `excerpt_hash` that disagrees with those bytes, or a line range describing a different passage than the byte range. Raised by `akr check`, not by a V-rule, because it is the one provenance question that needs the registered bytes as well as the ledger (D-031). |

---

## Retired codes

None. Codes are never renumbered and never reused; retired entries would be listed here
with a pointer to their replacement.
