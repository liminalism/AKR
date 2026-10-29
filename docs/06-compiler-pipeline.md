# 06 — The Compiler Pipeline

Stage-by-stage contracts for the AKR toolchain. For each of the six stages: what it
consumes, what it produces, what it checks, which diagnostic codes it may raise, how it
fails, and what makes it deterministic. Then hashing, incremental rebuild, ordering
guarantees, scale expectations, and the exact `akr build` sequence.

Normative for stage boundaries, failure semantics, the hashing scheme, and output
ordering. The *rules* checked in stages B, C and D are catalogued in
[`05-validation-rules.md`](05-validation-rules.md); this document says where each rule
runs and what happens when it fires.

---

## 1. Stage summary

| | Stage | Input | Output | Codes | Reads git | Writes |
| --- | --- | --- | --- | --- | --- | --- |
| A | Parse | Bytes | CST + trivia | `AKR-P`, `AKR-F` | no | — |
| B | Type-check | CST | Typed records | `AKR-T` | no | — |
| C | Link | Typed records | Reference graph | `AKR-L` | no | — |
| D | Resolve | Reference graph | Resolved model | `AKR-R`, `AKR-G` | **yes** | — |
| E | Index | Resolved model | `index.sqlite` | `AKR-I` | no | cache |
| F | Emit | Resolved model | Views | `AKR-E` | no | `docs/generated/`, `akr.lock` |

`akr check` runs A–D. `akr build` runs A–F. `akr fmt` runs A and re-emits. Read commands
(`get`, `search`, `context`, `impact`, `review-queue`) run A–E and then query the model.

## 2. Failure semantics

Two rules, and they are not the same rule:

**Within a stage: collect all.** A stage runs to completion over every input it can
still process and reports every diagnostic it finds. A file with four parse errors
reports four parse errors. A record with three missing required slots reports three.
This is the difference between one edit-compile cycle and four.

The exception is *local* rather than global: within a single record, a diagnostic that
makes further analysis of that record meaningless suppresses dependent diagnostics on
that record only. An unparsable record body yields one `AKR-P` diagnostic, not a type
error for every slot the parser never saw. Other records are unaffected.

**Between stages: halt.** If a stage collected any diagnostic of severity `error` — and
under the default `--strict` profile that includes every warning — the pipeline stops at
that stage boundary and produces no output. Later stages are not run.

The reason is that later-stage diagnostics computed over a broken earlier-stage model are
noise. If a reference does not resolve, stage D cannot tell whether a graph has a cycle;
reporting "no cycle found" or "cycle found" would both be lies. Stopping at the boundary
means every diagnostic a user sees was computed over a model that was valid up to that
point.

The rendered form, and `akr explain <code>`, are specified in
[`../spec/diagnostics/README.md`](../spec/diagnostics/README.md) §5.

**Exit status** follows [`07-cli.md`](07-cli.md) §3: 0 clean, 1 diagnostics, 2 usage,
3 environment.

**Staleness is not in this stream.** Stage D derives staleness and at-risk flags and
carries them in the resolved model. They never produce a code, never affect a stage
boundary, and never change an exit status (D-024). `akr check --review-clean` is the one
opt-in exception and it raises `AKR-G041`, which reports an unmet command-line request.

## 3. Stage A — Parse

**Input.** The byte contents of `.akr/project.akr`, every `*.akr` file under
`.akr/records/` and `.akr/archive/`, and `akr.lock` if present. Files are discovered by
a recursive directory walk whose entries are sorted by full path with a plain byte
comparison, so file order is the same on every filesystem.

**Output.** A concrete syntax tree per file. Every node carries a byte-offset span into
its source file; spans are converted to 1-based line and column only at render time.
Comments are captured as trivia and attached per D-006 — leading trivia to the next item
in the brace scope, trailing trivia to the item on the same line, end-of-scope comments
to the enclosing block — because the formatter must round-trip them.

**Checks.** Purely lexical and syntactic, per D-005 through D-008 and D-012:

- The `akr 0.1` header. An unknown minor version is a warning; an unknown major version
  is a hard error (D-021).
- Identifier charsets: key segments `[a-z][a-z0-9]*(-[a-z0-9]+)*`, two to eight segments;
  slot and block names `[a-z][a-z0-9_]*`. Keys never contain `_`; slot names never
  contain `-`.
- String escapes: exactly `\"`, `\\`, `\n`, `\t`, `\r`, `\u{HHHH}`, no raw newlines.
- Prose blocks: opening `"""` followed by a newline, closing `"""` alone on its line,
  trailing whitespace stripped, common indentation prefix removed, no tabs in the prefix,
  no leading or trailing blank lines.
- Scalar literals: bare dates, `Z`-terminated UTC timestamps, `git:` plus exactly 40
  lowercase hex digits, integers without leading zeros, `true`/`false`, bare enums.
- Structure: balanced braces and brackets, four-space indentation, no repeated slot
  within a scope, arrays comma-separated with optional trailing comma.
- File hygiene: UTF-8 without BOM, LF endings, exactly one trailing newline.

**Codes.** `AKR-P***`. `akr fmt --check` additionally re-emits the CST canonically and
compares against the input, raising `AKR-F***` on a difference. Both ranges are
registered in [`../spec/diagnostics/codes-lang.md`](../spec/diagnostics/codes-lang.md).

**Determinism.** The parser is a deterministic recursive-descent parser with no
backtracking that affects diagnostics, no locale-dependent character classification
(the identifier charset is ASCII by construction), and no dependence on file order.
Formatting is a fixed point: `fmt(fmt(x)) == fmt(x)` for every parseable `x`, and this
is property-tested in phase P2.

## 4. Stage B — Type-check

**Input.** The CST of each file.

**Output.** Typed records: for each record, its key, revision number, kind, class, state,
and a slot map whose values carry resolved types (`prose`, `commit`, `glob`,
`scope_term[]`, `ref[]`, …) rather than raw tokens. Claim, check, source and disposition
blocks become typed sub-structures. References are parsed into their four D-009 forms but
are *not* resolved — that is stage C.

**Checks.** Everything decidable from one record plus
[`../spec/tables/vocabulary.json`](../spec/tables/vocabulary.json):

| Check | Rule | Code |
| --- | --- | --- |
| The kind exists in the twelve-kind vocabulary | — | `AKR-T001` family |
| Required slots present; unknown slots rejected | V-008 | `AKR-T001` |
| The state belongs to the kind's class lifecycle | V-007 | `AKR-T011` |
| An observation carries `observed_at` | V-009 | `AKR-T021` |
| Evidence carries `result`, `method`, `observed_at` | V-010 | `AKR-T022` |
| A resolved question carries a `resolution` | V-011 | `AKR-T031` |
| Every value matches its slot's declared type | V-008 | `AKR-T001` family |
| Enum values are drawn from the slot's value set | V-008 | `AKR-T001` family |
| `scope` present on normative kinds | V-008 | `AKR-T001` |
| `topic` only on normative kinds | V-008 | `AKR-T001` |
| Blocks appear where the kind allows, with the right repeatability | V-008 | `AKR-T001` |
| `milestone` carries an `acceptance` block | V-008 | `AKR-T001` |
| `disposition` `into` present/absent per `outcome` | V-008 | `AKR-T001` |

**Codes.** `AKR-T***`, Writer A's registry.

**Determinism.** Stage B examines one record at a time and consults only a frozen table.
Its diagnostics are emitted in source order, which is file-path order then span order.

## 5. Stage C — Link

**Input.** Typed records for the whole ledger, plus `.akr/project.akr` for the namespace
declarations.

**Output.** The reference graph: every reference occurrence bound to a
`(key, revision, anchor?)` triple, plus a resolution log recording, for each *current
head* reference, which revision it resolved to. That log is what stage F writes into
`akr.lock` (D-014). The graph is stored as adjacency lists keyed by relation name so
that stage D's traversals are cheap.

**Checks.**

| Check | Rule | Code |
| --- | --- | --- |
| Every reference resolves to a declared key and existing revision | V-001 | `AKR-L001` |
| A key's namespace is declared in `project.akr` | V-002 | `AKR-L004` |
| All revisions of one key live in one file | V-003 | `AKR-L006` |
| Claim and check anchors exist; retired anchors reported distinctly | V-004 | `AKR-L012` |
| Relation and slot targets are kind-correct against the declared range | V-005 | `AKR-L031` |
| References to invalid terminal records only via historical relations | V-006 | `AKR-L021` |

The anchor check is worth its own sentence. A reference to `@key#anchor` where the head
revision does not define `anchor` but a previous revision did, *and* the head lists the
anchor in `retired_claims`, produces "anchor retired at revision N" and names the
revision that dropped it (D-011). Without the explicit retirement it produces a plain
"anchor not found". The difference is the difference between a five-second and a
five-minute investigation.

**Codes.** `AKR-L***`, Writer A's registry.

Head resolution here is the two-tier function of
[`04-references-and-versioning.md`](04-references-and-versioning.md) §3: the live
revision if there is one, otherwise the end of the supersession chain. A floating `@key`
therefore **always** resolves as long as the key exists — finishing a milestone does not
break `after [ @sys.milestone.m2-deterministic-sim ]`. Whether the resolved revision is
*live* is a separate question, asked by V-019 of the four relations where it matters
(`depends_on`, `implements`, `plan_of_record`, `supported_by`) and of no others. For
`depends_on`, a completed planning head is satisfied rather than invalid.

**Determinism.** Head resolution reads only the record set — never `akr.lock`, never git.
The lock is *written* from this stage's log and *compared* in stage D; it is never an
input to resolution, so a stale lock cannot change what a build resolves to. Reference
occurrences are visited in canonical record order (key, then revision) and within a
record in canonical slot order (D-012), so the resolution log is byte-stable.

## 6. Stage D — Resolve

The largest stage, and the only one that reads git.

**Input.** The reference graph, the git repository, `akr.lock` (for sealed hashes), and
today's date (threaded in explicitly; `--today` overrides it for testing).

**Output.** The **resolved model**, the data structure every later stage and every read
command consumes:

```
ResolvedModel
    commit              newest commit that exactly contains the loaded AKR sources;
                        absent for a dirty source graph or outside Git. A later
                        projection-only commit does not change this provenance.
    tool_version        semver of the binary
    grammar_version     from the file headers
    source_graph_hash   §9
    records[]           key -> { kind, class, revisions[] }
    revisions[]         (key, revision) -> { state, slots, claims, checks,
                                             sources, dispositions,
                                             content_hash, live, sealed }
    heads[]             key -> revision            (the single live revision)
    relations[]         (from_key, from_rev, relation, to_key, to_rev, to_anchor)
    supersession[]      chains, already checked acyclic
    acceptance[]        (owner, check_id) -> { satisfied, satisfying_evidence[] }
    freshness[]         (key, revision) -> { stale, cause, at_risk, path[] }
    contradictions[]    symmetric pairs with disposition status
    diagnostics[]       everything collected in A-D
```

**Checks, in the order they run.** Order matters because later checks assume earlier
ones passed.

1. **Head resolution (V-012, `AKR-R001`).** For each key, partition revisions into live
   and terminal by state against the class lifecycle. Exactly zero or one live revision
   is legal; two is a failure, never a newest-wins tiebreak (D-004a).
2. **Supersession chains (V-014, `AKR-R011`).** Walk `supersedes` edges. Every target
   must be in a terminal state. The graph must be acyclic.
3. **Acyclicity of the dependency graphs (V-015, `AKR-R012`; V-016, `AKR-R013`).**
   `depends_on`, `derived_from`, `part_of`, `implements`, `blocks`, and separately
   `after`. Each is checked by a depth-first traversal in canonical key order, so the
   *reported* cycle is the same one on every run.
4. **Live/terminal coherence (V-019, `AKR-R021`).** A live record may not point at a
   terminal one through `implements`, `plan_of_record` or `supported_by`. The same holds
   for `depends_on` except when the target is a completed planning record: completion
   satisfies the prerequisite and the edge remains as provenance.
   The historical relations — `after`, `part_of`, `blocks`, `verified_by`,
   `derived_from`, `supersedes`, `contradicts`, `resolves` — are exempt, because
   pointing at finished or retired work is exactly what they are for.
5. **Normative exclusivity (V-013, `AKR-R002`).** For each `topic`, collect the live
   normative records carrying it and test every pair for scope overlap under the
   conservative rule of D-010. Records with no `topic` never conflict.
6. **Plan of record (V-018, `AKR-R018`).** At most one live `plan_of_record` edge per
   milestone or track.
7. **Disposition completeness (V-017, `AKR-R014`).** For every superseding planning
   record, compute the unfinished children of its target — records in a live planning
   state related by `part_of` — and require a `disposition` block for each.
8. **Sealing (V-024, `AKR-R051`, `AKR-R052`).** Recompute the content hash of every
   revision in a non-`proposed` state and compare against `akr.lock` (D-015, §9 below).
9. **Acceptance (V-020, `AKR-R022`).** For each `check` of each completed milestone or
   work record, find the referenced evidence with `result pass` and test the
   descendant-commit condition against git.
10. **Contradiction disposition (V-023, `AKR-R041`).** Every `contradicts` edge is
    symmetric regardless of which side declared it; each must be resolved (one side
    terminal) or `acknowledged true`.
11. **Freshness derivation.** Not a rule, not a diagnostic — a build fact.
    [`10-freshness-and-git.md`](10-freshness-and-git.md) §3 specifies the algorithm:
    stale by watched path, stale by review date, then reverse propagation to dependents
    along `supported_by`, `depends_on` and `derived_from` only.

**Git access.** Stage D asks git exactly four kinds of question, all read-only:

| Question | Used by | Failure |
| --- | --- | --- |
| Does commit *c* exist? | `observed_at`, `as_of` | `AKR-G011` |
| Is *a* an ancestor of *b*? | acceptance (D-016), staleness | `AKR-G012`, `AKR-G003` |
| Which paths did the commits in *(a, b]* touch? | staleness by watch | `AKR-G002` |
| What is HEAD, and is the tree clean? | the build commit | `AKR-G001`, `AKR-G004` |

Results are memoised per build. The commit-range path query is issued **once** for the
union of all watch globs rather than once per record, which is what keeps the stage
linear in history size rather than quadratic (§11).

**Codes.** `AKR-R***` (Writer A) and `AKR-G***` (this design set,
[`../spec/diagnostics/codes-runtime.md`](../spec/diagnostics/codes-runtime.md)).

**Determinism.** Every traversal starts from a list sorted by key then revision. Every
set operation produces a sorted result. The only clock read is the `review_after`
comparison, which uses the explicitly threaded date. Git is queried by commit id, never
by branch name or reflog.

## 7. Stage E — Index

**Input.** The resolved model.

**Output.** `.akr/cache/index.sqlite`, whose DDL is
[`../spec/schema/index.sql`](../spec/schema/index.sql).

**What it is, and is not (D-019).** A cache. Gitignored. Never authoritative. Rebuildable
from source at any time. Deleting it is always safe. Nothing outside the tool reads it —
not agents, not scripts, not the MCP server, which goes through the same in-process
model the CLI uses. The boundary is absolute so the schema stays free to change: the
moment something reads the cache directly, the cache acquires compatibility obligations
and the ledger stops being the single source of truth.

**Procedure.**

1. Open or create the database. A file at the path that is a SQLite database without an
   AKR `meta` table is `AKR-I004`; an unreadable file is `AKR-I001`; a non-writable
   directory is `AKR-I003`.
2. Read `meta.schema_version` and `meta.source_graph_hash`. If either is absent or
   differs from the tool's values, drop every table and rebuild from scratch. This is
   **silent** — routine invalidation is not a diagnostic.
3. Open one transaction. Populate `sources`, `records`, `revisions`, `claims`,
   `relations`, `scopes`, `watches`, `checks`, `evidence_links`, `dispositions`,
   `resolutions`, `diagnostics`, and the `records_fts` virtual table. FTS5 unavailable is
   `AKR-I021`.
4. Verify invariants: every head in the model has a `resolutions` row (`AKR-I012`); row
   counts agree with the model (`AKR-I013`).
5. Commit. Run `PRAGMA integrity_check` (`AKR-I011`). Write failure is `AKR-I002`.

A second concurrent build in the same workspace fails with `AKR-I032` rather than
racing. A read command that needs a rebuild while `--no-rebuild` is in force fails with
`AKR-I031`. `akr search` in a binary built without FTS5 fails with `AKR-I022`; a binary
with FTS5 treats a cache that lacks `records_fts` (one written by a binary without it) as
stale and rebuilds it, or under `--no-rebuild` fails with `AKR-I031` naming `akr build`.

**Determinism.** Rows are inserted in canonical order (§8) so that a dump is stable, and
`rowid` is never used as an identifier that escapes the cache. SQLite's on-disk page
layout is *not* promised to be byte-identical between runs; every query result is.

## 8. Stage F — Emit

**Input.** The resolved model.

**Output.** Six Markdown views under the configured `view_output` directory (default
`docs/generated/`), and an updated `akr.lock`.

**Procedure.** For each view in the catalogue of
[`11-projections.md`](11-projections.md) §2, in the catalogue's fixed order: run the
view's source query against the model, render the sections in the view's declared order,
prepend the banner (D-025), and write the file. Then write `akr.lock` from stage C's
resolution log plus stage D's sealed hashes.

**Checks.**

| Check | Rule | Code |
| --- | --- | --- |
| The output directory is writable and inside the repository | — | `AKR-E001`, `AKR-E002` |
| Every record a view selects is present in the model | — | `AKR-E021` |
| No two records in one view render to the same heading anchor | V-115 | `AKR-E022` |
| Named view templates exist and their sections are defined | — | `AKR-E031`, `AKR-E032` |

Under `akr check --views-current`, stage F renders to memory instead of to disk and
compares against the committed files: a difference is `AKR-E011`, a missing file is
`AKR-E012`, a damaged banner is `AKR-E013`, and an unexpected file in the output
directory is `AKR-E014`. That comparison is the CI gate of D-025 and the thing that
gives the `sys.policy.no-hand-edited-views` record actual force.

`akr view <name>` renders one view to stdout without writing; an unknown name is
`AKR-E003`.

**Determinism.** Headings come from the required `title` slot, never derived from prose.
Every list is sorted by the view's declared sort key with key as the final tiebreak.
Relative links between views use fixed file names. The banner's three variable fields are
the source-graph hash, the commit, and the tool version — all inputs to the build, none
of them a clock reading.

## 9. Hashing

Two hashes, both SHA-256, both rendered as `sha256:` plus 64 lowercase hex digits. The
definitions here match D-014, D-015 and
[`../spec/schema/akr-lock.md`](../spec/schema/akr-lock.md).

### Revision content hash

> SHA-256 over the **canonically formatted text of that record**, as UTF-8 bytes.

Precisely: take the record's subtree, format it exactly as `akr fmt` would emit it —
canonical slot order per D-012, four-space indentation, canonical array layout, prose
blocks dedented and re-indented — and hash the resulting bytes, from the first byte of
`record` through the final `}` and its terminating newline inclusive.

Two exclusions, both normative in
[`../spec/schema/akr-lock.md`](../spec/schema/akr-lock.md) §3.3 and both deliberate:

- **Comment trivia is excluded.** Comments are commentary, not content. Adding a
  clarifying comment to a sealed record must not trip `AKR-R051`, or people stop writing
  comments — which is the opposite of what the format wants.
- **Surrounding file content is excluded.** Moving a record between files, or reordering
  it within one, does not change its hash. Identity is the key, never the file (D-018).

And one consequence of hashing the canonical form rather than the raw bytes:

- Reordering slots does not change the hash, because canonical formatting is applied
  first. A sealed record survives `akr fmt`. If a reformat *does* change a seal hash, the
  file was not canonical before, and the mismatch is real information.

This hash is what V-024 compares, and it is what `AKR-R051` reports when a sealed
revision has drifted.

### Source-graph hash

> SHA-256 over the sorted list of `(path, file hash)` pairs.

Precisely: for every source file the build read — `project.akr` and every `*.akr` under
`records/` and `archive/`, but **not** `akr.lock` and **not** anything under `cache/` —
compute the SHA-256 of its raw bytes. Sort the pairs by repository-relative path using a
byte comparison. Serialise each pair as `path` `NUL` `sha256:hex` `LF` (the form given
in `spec/schema/akr-lock.md` §3.2 — NUL because it cannot occur in a path), concatenate,
and hash the result.

Raw bytes, not canonical form, because this hash answers "are the inputs on disk the same
inputs?" — a question about the filesystem, not about meaning. It appears in the `meta`
table of the index, in every view banner, and in `akr.lock`, and a change to it is what
invalidates the cache.

## 10. Incremental rebuild and cache invalidation

The pipeline is fast enough at the target scale (§11) that full rebuild is the default
and the honest choice. Incrementality is an optimisation with a strict correctness rule:

> **An incremental build must produce byte-identical output to a full build, or it must
> fall back to a full build.**

Anything that cannot be shown to satisfy that rule is not attempted.

**What may be reused.** Stages A and B are per-file and per-record: a file whose SHA-256
is unchanged since the last build may reuse its cached typed records. That is the whole
of the safe incrementality, and it is where the time goes in a large ledger.

**What may not.** Stages C, D and F are whole-ledger by nature. Adding one record can
change a head resolution, complete or break a cycle, change a scope-overlap result,
satisfy an acceptance check, or alter staleness propagation anywhere in the graph. They
are re-run in full.

**Invalidation triggers.** A full rebuild — dropping and repopulating every table —
happens when any of:

| Trigger | Why |
| --- | --- |
| `meta.schema_version` differs from the tool's | The DDL changed; old rows are not readable |
| `meta.source_graph_hash` differs | Some source file changed, was added, or was removed |
| `meta.tool_version` differs | Rendering or derivation logic may have changed |
| `meta.commit` differs | Staleness and acceptance are relative to HEAD |
| `meta.today` differs | `review_after` comparisons move |
| The cache file is absent or fails `integrity_check` | Nothing to reuse |

A `schema_version` bump is therefore always a full rebuild, which is why the schema may
change freely between versions and why the file is gitignored.

**Never a trigger:** the branch name, the working-tree state (except that `AKR-G004`
warns about it), the wall clock beyond `today`, or the contents of `akr.lock`. The lock
is compared, never consulted for resolution (§5).

## 11. Ordering guarantees

Every collection that reaches an output — a view, a JSON document, an index table, a
context bundle, the lock — is sorted by an explicit key. The rule is stated once here and
relied on everywhere:

**Primary sort is always the record key, byte-comparison on the UTF-8 (in practice ASCII)
form.** Where a different primary sort is required by meaning — milestones by `after`
order in `ROADMAP.md`, review-queue entries by severity then depth — the *tiebreak chain*
ends in the key, so the order is total.

| Output | Sort |
| --- | --- |
| Diagnostics | file path, then span start, then code |
| `akr.lock` sections | fixed section order; within a section, path or key then revision |
| Index rows | key, then revision, then a per-table discriminator |
| Relations | from-key, from-revision, relation name, to-key, to-revision, anchor |
| View sections | the view's declared sort, ending in key |
| Context bundle sections | fixed section order; within a section, the section's sort key (`09-context-assembly.md` §4) |
| JSON arrays | the same order as the text form of the same command |
| Claims within a record | anchor id |
| Checks within an acceptance block | check id |
| Dispositions | the referenced key, then revision |
| Sets of scope terms | `all`, then `ref` terms by key, then `path` terms by glob |

Topological orders — `after` chains, supersession chains, propagation paths — are
computed with a deterministic tie-break on key, so a graph with several valid topological
orders always yields the same one.

## 12. Scale expectations

The design target is **10,000 records** in a single ledger, which is roughly two orders
of magnitude above where the worked example sits and about one above a large real project
after several years.

| Quantity | Target | Notes |
| --- | --- | --- |
| Records | 10,000 | Revisions perhaps 1.5× that |
| Source files | 200–2,000 | Convention groups records by namespace and kind |
| References | ~50,000 | Roughly five per record |
| Full `akr check` | < 2 s | Cold, no cache |
| Full `akr build` | < 5 s | Including index and six views |
| `akr context` | < 200 ms | Warm cache |
| `akr search` | < 100 ms | FTS5 over 10,000 documents |
| Index size | < 50 MB | Dominated by FTS and prose |

Complexity, with *n* records, *e* references, *w* distinct watch globs and *c* commits
since the oldest `observed_at`:

- A, B: **O(bytes)**.
- C: **O(e log n)**.
- D: **O(n + e)** for the traversals; **O(n²)** worst case for exclusivity, but only
  within one `topic` group, and topic groups are small by construction — the practical
  cost is Σ|group|².
- D, git: **one** ancestry query per distinct `(observed_at, HEAD)` pair and **one**
  path-changed query per distinct commit range, both memoised. Emphatically not one
  `git log` per record; the union of watch globs is queried once and the resulting
  path set intersected per record, giving **O(c + n·w)** rather than **O(n·c)**.
- E: **O(n + e)** inserts in one transaction.
- F: **O(n log n)** dominated by sorting.

The 10,000-record target is what justifies keeping full rebuild as the default. If a
future ledger makes that untenable, the escape hatch is the per-file reuse of §10, not a
weakening of the determinism contract.

## 13. The `akr build` sequence

The exact sequence, in order. Every step is skippable only where noted.

```
akr build [--dir <path>] [--strict|--lenient] [--at <commit>] [--today <date>]
```

1. **Locate the workspace.** Walk up from `--dir` (default: the current directory)
   looking for `.akr/`. Not found is `AKR-C011`, exit 3. `project.akr` missing is
   `AKR-C012`, exit 3.
2. **Read `project.akr`.** Namespaces and the `defaults` block. An unknown defaults key
   is `AKR-C021`; a duplicate namespace is `AKR-C022`; a malformed project name is
   `AKR-C023`.
3. **Open the repository.** Not a git repository is `AKR-G001`, exit 3. Resolve HEAD, or
   the `--at` commit (`AKR-G013` if unknown). A shallow history is `AKR-G003`. A dirty
   working tree is `AKR-G004`, a warning.
4. **Discover and read sources.** Sorted walk of `records/` and `archive/`. Compute each
   file's SHA-256 and the source-graph hash (§9).
5. **Stage A — parse** every file. Collect all. Halt on error.
6. **Stage B — type-check** every record. Collect all. Halt on error.
7. **Stage C — link.** Build the reference graph; record the head-resolution log. Collect
   all. Halt on error.
8. **Stage D — resolve.** Checks 1–10 of §6 in order, then freshness derivation. Collect
   all. Halt on error. Staleness never halts.
9. **Stage E — index.** Compare `meta` against the current values; full rebuild if any
   differs. Populate in one transaction. Verify. Commit.
10. **Stage F — emit views.** Render each view in catalogue order into memory. Compare
    against what is on disk; write only the files whose bytes differ, so that an
    unchanged view keeps its mtime and a no-op build produces no diff. Report the count
    written.
11. **Stage F — update `akr.lock`.** Serialise, in the fixed order of D-014: tool and
    grammar version; the resolved commit; the source-graph hash; one entry per source
    file with its content hash; one entry per current-head resolution performed during
    this build; one entry per sealed revision with its content hash. Format canonically
    in AKR syntax with the `akr-lock 0.1` header. Write only if the bytes differ.
12. **Report.**

```
$ akr build
parsed 42 revisions in 19 files
resolved 40 heads, 2 superseded revisions
2 stale records, 4 at risk (see akr review-queue)
wrote .akr/cache/index.sqlite
wrote docs/generated/ (6 views, 2 changed)
akr.lock unchanged
```

Exit 0. The staleness line is a build fact, printed for the operator's benefit and
carrying no code and no effect on the exit status (D-024).

**Ordering rationale for steps 10 and 11.** Views are written before the lock so that a
build interrupted between them leaves a lock that is *older* than the views rather than
newer. A stale lock is caught by the next `akr check` (`AKR-R052`); a lock that claims to
describe views it does not would be caught by nothing.

**What `akr build` never does.** It never writes a `.akr` source file, never stages,
never commits, never contacts the network, and never invokes a language model (D-003,
D-020). The only paths it writes are `.akr/cache/index.sqlite`, `akr.lock`, and the files
under `view_output`.

---

Next: [`07-cli.md`](07-cli.md) for the commands built on these stages,
[`../spec/schema/index.sql`](../spec/schema/index.sql) for the stage E schema, or
[`11-projections.md`](11-projections.md) for what stage F renders.
