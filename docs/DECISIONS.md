# AKR Design Decisions

This file records every question the planning notes left open or answered
inconsistently, together with the resolution the specification set is built on. Each
entry is normative. Specification documents implement these decisions; they do not
re-open them.

**Frozen.** Nothing in this file changes as a side effect of writing a specification
document. If a specification document cannot be written consistently with a decision
here, the correct move is to report the conflict and amend this file deliberately, in
its own commit, updating every document listed under **Honored by**.

Companion frozen artifacts: `spec/tables/vocabulary.json` (machine-readable form of
D-001, D-002, D-004, D-010, D-012, D-016), `spec/exemplar.akr` (the only quotable
source of syntax forms), `examples/save-your-skin/MANIFEST.md` (the worked example
inventory), `spec/diagnostics/README.md` (D-013).

---

## D-001 — The record kind vocabulary is exactly twelve kinds

**Question.** The first planning draft listed six kinds (requirement, decision,
observation, evidence, plan, question); the second listed twelve, dropping `plan` and
adding `term`, `policy`, `constraint`, `assessment`, `milestone`, `work`, `track`.
Which list is canonical?

**Resolution.** Twelve kinds, and no more before the first dogfood completes:

`term`, `requirement`, `policy`, `constraint`, `decision`, `observation`, `evidence`,
`assessment`, `milestone`, `work`, `track`, `question`.

`plan` is **not** a kind. A plan is a `work` record designated as the plan of record
for a milestone or track through the `plan_of_record` relation. The relation carries
the meaning that a separate kind would have carried, and it is checkable (D-004,
V-018).

**Rationale.** A plan differs from other work only in its authority over a milestone,
which is a relational fact. Encoding it as a kind would have produced two ways to say
the same thing and a second head-resolution rule. Keeping the vocabulary small is a
stated goal; the burden of proof is on additions.

**Honored by.** `docs/02-data-model.md`, `spec/tables/vocabulary.json`,
`docs/09-context-assembly.md`, `docs/11-projections.md`, all fixtures and examples.

---

## D-002 — Kinds are grouped into four classes, and the classes carry the rules

**Question.** Lifecycles, validation rules, and context ordering were specified
kind-by-kind, producing twelve near-duplicate rule sets.

**Resolution.** Every kind belongs to exactly one class:

| Class | Kinds | What the class means |
| --- | --- | --- |
| **normative** | `term`, `requirement`, `policy`, `constraint`, `decision` | States what *ought* to be true; binds future work |
| **empirical** | `observation`, `evidence`, `assessment` | States what *was found* to be true, at a stated point in history |
| **planning** | `milestone`, `work`, `track` | States what is *intended*, in what order, and when it is done |
| **inquiry** | `question` | States what is *not yet known* |

Lifecycle state sets, state-transition graphs, relation domains, staleness behaviour,
and context-assembly ordering are defined per class. Kind-specific rules exist only
where a kind genuinely differs (for example, `observation` requires `observed_at`).

**Rationale.** Four state machines instead of twelve. New kinds, if any are ever
added, join a class and inherit its rules rather than inventing a fifth.

**Honored by.** `docs/02-data-model.md`, `docs/05-validation-rules.md`,
`docs/09-context-assembly.md`, `spec/tables/vocabulary.json`.

---

## D-003 — `needs-review` is derived, never authored

**Question.** The planning notes listed `needs-review` as a lifecycle state for
observations and evidence, and separately said git integration "marks records
needs-review when watched paths change". Those two statements together require the
build to write source files.

**Resolution.** `needs-review` is **not** a lifecycle state. Authored empirical
states are `verified`, `disproven`, `superseded`, `withdrawn`. Staleness is a
*derived* property of the pair (record, current commit), computed in resolve
(stage D), materialised in the index, and surfaced by `akr review-queue` and
`REVIEW-REQUIRED.md`. Nothing in `akr build` ever writes a `.akr` source file.

A human or agent who acts on the review queue does so with an explicit write command
(`akr revise`, `akr evidence add`, `akr supersede`), which is a separate operation
from the build.

**Rationale.** The build must be a pure function of (sources, commit, tool version);
that is what makes it reproducible, cacheable, and safe to run in CI. A build that
mutates its own inputs has neither property. It also preserves the stated invariant
that the system never auto-declares a record false — staleness is a question raised,
not an answer given.

**Honored by.** `docs/02-data-model.md`, `docs/06-compiler-pipeline.md`,
`docs/10-freshness-and-git.md`, `docs/11-projections.md`, `docs/07-cli.md`.

---

## D-004 — One live head per key; normative exclusivity is a separate, topic-based rule

**Question.** "For a normative record type, only ONE active head per key + overlapping
scope" conflates two different checks.

**Resolution.** Two rules.

*(a) Head rule, all kinds (V-012, `AKR-R001`).* For a given logical key, at most one
revision may be in a **live** state. Every other revision must be in a terminal state
(`superseded`, `rejected`, `withdrawn`, `abandoned`, `completed`,
`closed-without-resolution`, `disproven`, as applicable to its class). Two live
revisions of one key is a build failure, never a newest-wins tiebreak.

*(b) Exclusivity rule, normative kinds only (V-013, `AKR-R002`).* Normative records
may carry an optional `topic` identifier. Two live normative records that share a
`topic` and whose `scope` sets overlap (D-010) is a build failure. Records with no
`topic` are never in conflict by this rule.

**Rationale.** (a) is about identity and is universal. (b) is about governance and
needs an explicit, authored declaration of "these two speak to the same thing" —
inferring it from prose would require judgement the compiler does not have. `topic`
is opt-in, cheap to write, and mechanically decidable.

**Honored by.** `docs/02-data-model.md`, `docs/04-references-and-versioning.md`,
`docs/05-validation-rules.md`.

---

## D-005 — Identifier and namespace lexicon

**Question.** Character sets for keys, slot names, and namespaces were unspecified.

**Resolution.**

- **Key segment**: `[a-z][a-z0-9]*(-[a-z0-9]+)*` — lowercase ASCII, digits, internal
  hyphens.
- **Key**: two to eight segments joined by `.`, for example
  `lege.viewer.renderer-boundary`. The first segment is the **namespace**.
- **Slot and block names**: `[a-z][a-z0-9_]*` — lowercase ASCII snake_case.
- **Enum values and anchors**: key-segment form (hyphens, no underscores).
- Keys and slot names are therefore lexically distinguishable: keys never contain
  `_`, slot names never contain `-`.
- Namespaces must be declared in `.akr/project.akr`. A key whose first segment is not
  a declared namespace is an error (V-002, `AKR-L004`).
- No Unicode in identifiers. Prose may be any valid UTF-8.

**Rationale.** One shape per concept, no case rules to remember, no homoglyph or
normalisation questions in identity. Declared namespaces are the cheapest available
defence against silent typo-drift (`lege.` versus `ledge.`) creating a second
knowledge graph nobody notices.

**Honored by.** `docs/03-syntax.md`, `spec/grammar/akr.ebnf`,
`docs/04-references-and-versioning.md`, `fixtures/parse/`.

---

## D-006 — Comments: `#` to end of line, with defined attachment

**Question.** Comment syntax, and whether the canonical formatter preserves comments.

**Resolution.** `#` to end of line. No block comments, no doc-comment convention, no
nesting. Comments are preserved by the formatter with these attachment rules:

- A comment on its own line attaches as **leading trivia** to the next item (record,
  slot, block, or array element) in the same brace scope. Leading trivia is re-emitted
  above that item, at that item's indentation, in original order.
- A comment following a value on the same line attaches as **trailing trivia** to that
  item, and is re-emitted after exactly two spaces.
- Comments at the end of a brace scope with no following item attach as trailing
  trivia to the enclosing block.
- Blank lines inside a record body are not preserved; the formatter emits exactly one
  blank line between records and none within them, except that a leading-comment group
  is preceded by a blank line if it was in the input.

**Rationale.** Comments are how a record explains an edit to the next reader, so they
must survive `akr fmt`. Attachment must be total and deterministic or round-tripping
is not well defined. `#` needs no escape handling inside the rest of the grammar.

**Honored by.** `docs/03-syntax.md`, `spec/grammar/akr.ebnf`, `spec/exemplar.akr`,
`fixtures/format/`.

---

## D-007 — Strings and prose blocks

**Question.** Escaping, multi-line prose, and how indentation inside prose is
normalised.

**Resolution.** Two string forms.

*Quoted string* `"..."` — single line. Legal escapes are exactly `\"`, `\\`, `\n`,
`\t`, `\r`, and `\u{HHHH}` (1–6 hex digits, a Unicode scalar value). Any other
backslash sequence is an error (`AKR-P012`). Raw newlines are not permitted.

*Prose block* `"""..."""` — multi-line, **raw**: no escape sequences at all, so a
backslash is a backslash. Rules:

1. The opening `"""` must be followed by a newline; content starts on the next line.
2. The closing `"""` must be the only non-whitespace content on its line.
3. Trailing whitespace is stripped from every line.
4. The common leading-whitespace prefix of all non-blank lines is removed. Blank lines
   are treated as empty regardless of their whitespace.
5. Tabs inside the indentation prefix are an error (`AKR-P015`); prose is indented with
   spaces.
6. The result has no leading or trailing blank lines.

The formatter re-emits prose blocks indented one level deeper than the owning slot,
with the closing `"""` at that same indentation.

**Rationale.** Prose is the payload of most records and gets pasted, quoted, and
diffed constantly; escape processing in it would be a permanent source of surprise. A
single, fully specified dedent rule means the formatter is a fixed point and diffs
reflect meaning changes only.

**Honored by.** `docs/03-syntax.md`, `spec/grammar/akr.ebnf`, `fixtures/parse/`,
`fixtures/format/`.

---

## D-008 — Scalar literal forms

**Question.** How dates, timestamps, commits, globs, and numbers are written.

**Resolution.**

| Type | Form | Notes |
| --- | --- | --- |
| date | `2026-08-03` | Proleptic Gregorian, bare, unquoted |
| timestamp | `2026-08-03T14:00:00Z` | UTC only; the `Z` is mandatory; no offsets, no local time |
| commit | `git:` + exactly 40 lowercase hex digits | Abbreviations are rejected (`AKR-P021`) |
| glob | quoted string | Repo-root-relative, `/` separators, subset `*`, `**`, `?`, `[a-z0-9]`; no brace expansion, no `!` negation |
| integer | `0`, `42`, `-3` | No floats, no exponents, no underscores, no leading zeros |
| boolean | `true` / `false` | |
| enum | bare identifier in key-segment form | |

**Rationale.** Abbreviated hashes are not stable identity — they collide as history
grows, and resolving them would make parsing depend on repository state. Local time in
a ledger read by agents in unknown timezones is a bug generator. Floats have no use in
this vocabulary and invite formatting non-determinism.

**Honored by.** `docs/03-syntax.md`, `spec/grammar/akr.ebnf`,
`docs/10-freshness-and-git.md`, `spec/schema/akr-lock.md`.

---

## D-009 — Exactly four reference forms

**Question.** Reference syntax and which resolution modes exist.

**Resolution.** Four forms, no others:

| Form | Meaning |
| --- | --- |
| `@key` | Current head — resolves to whichever revision is live at build time |
| `@key/2` | Pinned — resolves to revision 2, always |
| `@key#anchor` | Current head, claim or check anchor |
| `@key/2#anchor` | Pinned revision, claim or check anchor |

No `@key/latest`, no revision ranges, no wildcards, no cross-project references in
0.1. Every current-head resolution performed by a build is written to `akr.lock`
(D-014), so a build is reproducible from (sources, lock) alone.

Guidance, not enforced: pin when citing evidence or narrating history; float when
referring to governing policy you intend to keep following.

**Rationale.** Two resolution modes are the minimum that supports both "follow the
current rule" and "this is what I relied on". Anything more expressive turns reference
resolution into a query language and the lock file into a query cache.

**Honored by.** `docs/03-syntax.md`, `docs/04-references-and-versioning.md`,
`spec/schema/akr-lock.md`, `docs/09-context-assembly.md`.

---

## D-010 — Scope is a set of terms with a conservative overlap test

**Question.** The notes wrote `scope @sys.goal.playable-day`, referring to a `goal`
kind that does not exist, and never defined what scope overlap means.

**Resolution.** `scope` is an array of **scope terms**. A term is one of:

- `all` — project-wide.
- `ref @key` — organisational scope. The target must be a `milestone`, `track`, or
  `constraint` (V-005).
- `path "glob"` — code scope, repo-root-relative.

Two scopes overlap if any term of one overlaps any term of the other:

- `all` overlaps everything.
- Two `ref` terms overlap if they are equal, or if one is reachable from the other
  through `part_of` edges.
- Two `path` terms overlap if their literal prefixes (the portion before the first
  wildcard) are prefix-comparable, treating `**` as matching any sequence of segments
  and `*`/`?` as matching within one segment.
- A `ref` term and a `path` term never overlap by themselves. A record that must be
  compared against both should declare both.

The test is deliberately **conservative**: it may report an overlap where none exists
in practice, and it must never miss one. False positives are resolved by narrowing
scope or removing a `topic`; false negatives would silently permit contradictory
governance.

**Rationale.** Overlap decides D-004(b), so it must be decidable, cheap, and stable
across implementations. Full glob-intersection is neither. There is no `goal` kind;
the example in the planning notes becomes `scope [ ref @sys.milestone.playable-day ]`.

**Honored by.** `docs/02-data-model.md`, `docs/05-validation-rules.md`,
`spec/tables/vocabulary.json`, `examples/save-your-skin/`.

---

## D-011 — Claims are versioned with their record; retirement is explicit

**Question.** Whether claim anchors are independently versioned, and what happens to
a reference to a claim that a later revision drops.

**Resolution.** A claim is a `claim <anchor> { ... }` block inside a record. Claims
are **not** independently versioned; a claim belongs to the revision that contains it,
and `@key/2#anchor` is a reference into revision 2.

Anchor ids are stable across revisions when the claim's meaning is unchanged; a
changed meaning requires a new anchor id. A revision that drops an anchor present in
the previous revision must list it in `retired_claims [ ... ]`. A current-head
reference to a retired anchor then produces the specific diagnostic "anchor retired at
revision N" (V-004, `AKR-L012`) rather than a generic "not found", and points the
reader at the revision that dropped it.

**Rationale.** Independently versioned claims would give a record two version numbers
and a partial-order problem. Explicit retirement costs one line at the moment of
authorship and buys a precise error at the moment of confusion, which is the trade the
whole design keeps making.

**Honored by.** `docs/02-data-model.md`, `docs/04-references-and-versioning.md`,
`docs/05-validation-rules.md`.

---

## D-012 — Slots, blocks, arrays, and enforced canonical ordering

**Question.** Whether slots may repeat, how arrays are written, and whether the
formatter reorders content.

**Resolution.**

- **Slots are unique** within a record or block. A repeated slot is an error
  (`AKR-P031`).
- **Blocks may repeat**: `claim`, `check`, `source`, and `disposition`. `acceptance`
  appears at most once.
- Multi-valued content uses arrays with **plural slot names** (`aliases`,
  `exceptions`, `watches`). Relation slots keep the relation name verbatim
  (`supported_by`, `depends_on`) and are always arrays, even with one element.
- Arrays are comma-separated inside `[ ]` with one space of padding. A trailing comma
  is accepted on input and removed by the formatter. An array is emitted on one line if
  it fits within 96 columns, otherwise one element per line.
- **The formatter enforces canonical ordering**, and this is not optional:
  1. `title`
  2. `state`
  3. `scope`
  4. `topic`
  5. kind-specific content slots, in the order given in `spec/tables/vocabulary.json`
  6. `claim` blocks, sorted by anchor id
  7. `retired_claims`
  8. `acceptance` block, with `check` blocks sorted by check id
  9. `disposition` blocks, sorted by their reference
  10. relation slots, alphabetical by relation name; refs within an array sorted by
      key, then by revision, then by anchor
  11. `author`, `created_at`
  12. `source` blocks, sorted by source kind then by path or url
- Indentation is four spaces per level. Files are UTF-8 without BOM, LF line endings,
  and end with exactly one newline.

**Rationale.** Canonical ordering is what makes a diff mean something: a reordered
record produces no diff, and a real change produces a small one. It also removes an
entire category of review comment. The cost — the tool moves your lines — is paid once.

**Honored by.** `docs/03-syntax.md`, `spec/exemplar.akr`, `fixtures/format/`,
`spec/tables/vocabulary.json`.

---

## D-013 — Diagnostic and rule identifier scheme

**Question.** How errors are numbered, who owns which numbers, and what severity
means.

**Resolution.** Diagnostics are `AKR-<stage><nnn>`, where stage is one letter:

| Letter | Stage | Registry |
| --- | --- | --- |
| `P` | parse | `spec/diagnostics/codes-lang.md` |
| `F` | format / canonicalisation | `spec/diagnostics/codes-lang.md` |
| `T` | type-check | `spec/diagnostics/codes-lang.md` |
| `L` | link | `spec/diagnostics/codes-lang.md` |
| `R` | resolve | `spec/diagnostics/codes-lang.md` |
| `I` | index | `spec/diagnostics/codes-runtime.md` |
| `E` | emit / projection | `spec/diagnostics/codes-runtime.md` |
| `X` | context assembly | `spec/diagnostics/codes-runtime.md` |
| `G` | git / freshness | `spec/diagnostics/codes-runtime.md` |
| `C` | cli / config | `spec/diagnostics/codes-runtime.md` |
| `M` | migration / import | `spec/diagnostics/codes-runtime.md` |

Every code is defined in exactly one registry, is cited by at least one specification
document, and carries: title, severity, message template, a minimal reproducing
source, and fix guidance. Codes are never renumbered or reused.

**Validation rules** are numbered `V-nnn` — a separate namespace from the stage letter
`R`, to avoid two meanings for one prefix. `V-001`–`V-099` are the language and graph
rules catalogued in `docs/05-validation-rules.md`. `V-101`–`V-149` are reserved for
freshness, emission, and context rules catalogued in `docs/10-freshness-and-git.md`,
`docs/11-projections.md`, and `docs/09-context-assembly.md`. A rule cites the code it
raises; a code names the rule that raises it.

**Severity.** `error` and `warning` only. The default profile is `--strict`, in which
warnings are errors and the build fails. `--lenient` downgrades warnings and exists
for exactly one purpose: `akr import` on legacy material (D-022).

**Rationale.** Stage-letter codes tell a reader where in the pipeline a failure
happened before they read the message. Strict-by-default is the only setting under
which a warning ever gets fixed.

**Honored by.** `spec/diagnostics/README.md`, both registries, every specification
document that cites a code.

---

## D-014 — `akr.lock` is written in AKR syntax and is committed

**Question.** Lock file format — TOML, JSON, or something else — and what it contains.

**Resolution.** `akr.lock` is written in the AKR grammar, with the header
`akr-lock 0.1` in place of `akr 0.1`. It is committed to the repository. It contains,
in a fixed order and sorted deterministically:

1. Tool version and grammar version.
2. The git commit the build resolved against.
3. The source-graph hash.
4. One entry per source file: path and content hash.
5. One entry per current-head reference resolved during the build: referring revision,
   referenced key, resolved revision, and that revision's content hash.
6. One entry per sealed revision: key, revision, content hash (D-015).

Hash definitions live in `spec/schema/akr-lock.md`: a revision content hash is
SHA-256 over the canonically formatted text of that record; the source-graph hash is
SHA-256 over the sorted list of (path, file hash) pairs.

**Rationale.** Reusing the grammar means one parser, one formatter, one determinism
story, and one set of diff-readability properties. A generated file that a reviewer
must read during supersession review should not be in a second syntax. TOML or JSON
would each add a dependency and a canonicalisation question already answered here.

**Honored by.** `docs/04-references-and-versioning.md`, `spec/schema/akr-lock.md`,
`docs/06-compiler-pipeline.md`, `docs/07-cli.md`.

---

## D-015 — Sealing gives revision immutability teeth

**Question.** "Accepted bodies are immutable; changes require a new revision" — how is
that enforced without a server?

**Resolution.** A revision in any non-`proposed` state is **sealed**. Its content hash
is recorded in `akr.lock`. `akr check` recomputes the hash of every sealed revision
and fails with `AKR-R051` ("sealed revision modified; create a new revision instead")
on mismatch, naming the key, the revision, and the expected hash. A revision whose
resolution is absent from an otherwise-current lock raises `AKR-R052`.

`proposed` revisions are not sealed and may be edited freely, which is what makes
`proposed` useful.

**Rationale.** The rule is enforced by the same commit-and-review machinery the
project already has: changing a sealed record shows up as a lock diff, which is
exactly the thing a reviewer should be looking at. No daemon, no signatures, no
central authority.

**Honored by.** `docs/04-references-and-versioning.md`, `docs/05-validation-rules.md`,
`spec/schema/akr-lock.md`.

---

## D-016 — Acceptance is a block of checks; evidence never points at what it verifies

**Question.** How acceptance criteria are expressed, and in which direction the
evidence link runs.

**Resolution.** `milestone` records require, and `work` records may carry, an
`acceptance` block containing one or more `check` blocks:

- A `check <id>` has a `statement` (prose), a `method`
  (`manual` | `command` | `observation`), an optional `command`, and a `verified_by`
  array of references to `evidence` records.
- A check is **satisfied** when at least one referenced evidence record has
  `result pass` and an `observed_at` commit that is a descendant of the last commit
  that changed the content of the work item's current revision.
- Completing a `milestone` or `work` record with an unsatisfied check is an error
  (V-020, `AKR-R022`).

The `verified_by` relation runs in exactly one direction: from the thing being
verified to the evidence. An `evidence` record never declares what it verifies. Its
own slots describe only the observation: `result`, `method`, `observed_at`, optional
`command` and `artifact`.

**Rationale.** A two-directional link is two sources of truth and a reconciliation
rule nobody wants to write. Putting `verified_by` on the check keeps acceptance
readable in one place — the milestone tells you what "done" means and what proved it —
and makes evidence records reusable across several checks. The descendant-commit
condition is what stops a passing test from 200 commits ago closing a milestone whose
definition changed yesterday.

**Honored by.** `docs/02-data-model.md`, `docs/05-validation-rules.md`,
`docs/10-freshness-and-git.md`, `docs/11-projections.md`.

---

## D-017 — Supersession must dispose of unfinished children

**Question.** Where disposition is recorded and what the outcomes are.

**Resolution.** The **superseding** record carries one `disposition` block per
unfinished child of the record it supersedes:

```
disposition @sys.work.m3-lighting-pass {
    outcome carried_forward
    into @sys.track.lighting
}
```

`outcome` is one of `carried_forward`, `completed_elsewhere`, `intentionally_dropped`,
`still_required_separately`. `into` is required for `carried_forward` and
`completed_elsewhere`, optional for `still_required_separately`, and forbidden for
`intentionally_dropped`. An optional `note` may explain the choice.

"Unfinished child" means any record in a live planning state related to the superseded
record by `part_of`. A superseding planning record that omits a disposition for any
such child fails with V-017, `AKR-R014`.

**Rationale.** This is the single most valuable check in the system. Dropped work
silently disappearing across a replan is the failure mode that makes long-running
agent projects untrustworthy. The cost is one block per unfinished item at exactly the
moment the author knows the answer.

**Honored by.** `docs/02-data-model.md`, `docs/04-references-and-versioning.md`,
`docs/05-validation-rules.md`, `examples/save-your-skin/`.

---

## D-018 — Files are containers; identity never comes from paths

**Question.** How records map onto files, and what `archive/` means.

**Resolution.** A `.akr` file may contain any number of records. Identity comes from
the key alone; nothing in the compiler derives meaning from a file's name or location
below `.akr/records/`.

Two conventions, one of them enforced:

- *Convention:* one file per namespace subtree and kind group, for example
  `.akr/records/sys/policies.akr`.
- *Enforced (V-003, `AKR-L006`):* every revision of one key lives in one file, so a
  key's whole history is reviewable in one place and one diff.

`.akr/archive/` holds files in which every record is in a terminal state. Archived
records still resolve, so historical references never break, but they are excluded
from ordinary context assembly and from every generated view except
`DECISION-HISTORY.md`. Moving a file to `archive/` is a filesystem operation with no
semantic effect beyond that exclusion; state is what makes a record terminal.

**Rationale.** Filename-as-identity is the failure the whole project exists to avoid.
The one-file-per-key rule is a review ergonomics rule, not a semantic one, and is
cheap to satisfy.

**Honored by.** `docs/04-references-and-versioning.md`,
`docs/05-validation-rules.md`, `docs/09-context-assembly.md`,
`examples/save-your-skin/MANIFEST.md`.

---

## D-019 — The SQLite index is a cache, and agents never read it

**Question.** Status of the generated index, and who may touch it.

**Resolution.** `.akr/cache/index.sqlite` is a rebuildable cache. It is gitignored. It
carries `schema_version` and `source_graph_hash` in a `meta` table; a mismatch in
either triggers a full rebuild. Deleting it is always safe.

Agents access knowledge only through the CLI or MCP surface. `AGENTS.md` says so
explicitly. All authoritative writes go through validated source-record operations that
produce canonically formatted `.akr` text. `akr build` materialises the index eagerly;
a search whose cache is missing or stale refreshes that disposable index before querying.
`--no-rebuild` turns that refresh into `AKR-I031` for read-only checkouts.

**Rationale.** The moment anything reads the cache directly, the cache becomes a
schema with compatibility obligations, and the ledger stops being the single source of
truth. Keeping the boundary absolute keeps the index free to change.

**Honored by.** `docs/06-compiler-pipeline.md`, `spec/schema/index.sql`,
`docs/08-mcp.md`, `docs/09-context-assembly.md`.

---

## D-020 — No language model participates in a build

**Question.** Where the LLM boundary sits.

**Resolution.** Stages A through F contain no model inference of any kind. A build is
a pure function of (source files, git commit, tool version) and is byte-identical
across machines.

Models are welcome on the other side of the boundary: drafting record bodies for human
or agent review, proposing imports from legacy documents, summarising a context bundle,
and ranking search results. They may never determine authority, head resolution,
scope overlap, cycle detection, staleness, acceptance, or supersession. Every one of
those has a defined algorithm in this specification set, and the algorithm is the
answer.

**Rationale.** The value proposition is that the ledger's mechanical claims are
trustworthy. A probabilistic step anywhere in the build destroys that for every claim
downstream of it, and the failures would be quiet.

**Honored by.** `docs/01-architecture.md`, `docs/06-compiler-pipeline.md`,
`docs/08-mcp.md`, `docs/12-migration.md`.

---

## D-021 — Language versioning before 1.0

**Question.** What `akr 0.1` versions, and what compatibility is promised.

**Resolution.** The file header versions the **grammar**, not the tool and not the
vocabulary. `spec/tables/vocabulary.json` carries its own `vocabulary_version`, which
moves independently.

Before 1.0: an unknown minor version is a warning (and therefore an error under the
default strict profile, fixable with `--lenient`); an unknown major version is a hard
error. No forward compatibility, no deprecation windows, and no migration tooling for
grammar changes are promised until AKR has been dogfooded on two or three real
projects. Breaking changes before 1.0 are expected and are handled by `akr fmt`
upgrades shipped with the tool.

**Rationale.** Promising stability before the design has met a second and third real
project is how a format acquires permanent mistakes.

**Honored by.** `docs/03-syntax.md`, `docs/13-implementation-roadmap.md`,
`spec/grammar/akr.ebnf`.

---

## D-022 — Migration adds no kinds

**Question.** The notes referred to "legacy-source records" without saying whether
that is a kind.

**Resolution.** It is not. Legacy provenance is a repeatable `source` block:

```
source {
    kind legacy
    path "docs/legacy/ROADMAP.md"
    excerpt """
        M3 — playable day. Ship the day loop.
        """
}
```

`kind` is `legacy`, `external`, or `internal`. Each legacy document being migrated
gets one tracking `work` record whose acceptance checks enumerate the disposition of
its durable claims. The legacy document is archived only when that work record reaches
`completed`, which by V-020 requires every check satisfied. `akr import --lenient` is
the only place warnings are downgraded, and everything it produces lands in `proposed`
state for review.

**Rationale.** Migration is a workflow, not a category of knowledge. A thirteenth kind
would outlive the migration it was created for. Reusing `work` plus acceptance means
migration progress shows up in `ACTIVE-WORK.md` like any other work.

**Honored by.** `docs/12-migration.md`, `docs/02-data-model.md`,
`docs/07-cli.md`, `examples/save-your-skin/`.

---

## D-023 — Contradictions are declared, with one inferred check

**Question.** Whether the compiler detects contradictions.

**Resolution.** Primarily, no — contradiction is **declared** with the `contradicts`
relation, which is treated as symmetric regardless of which side declares it. A
declared contradiction must be dispositioned: either resolved (one side reaches a
terminal state) or explicitly `acknowledged true` on the declaring record. An
undispositioned contradiction fails with V-023, `AKR-R041`.

The single inferred check is D-004(b): two live normative records sharing a `topic`
with overlapping scope.

Contradictions are **always** surfaced in `akr context`, including when one side has
been superseded, and they are never suppressed by relevance ranking.

**Rationale.** Detecting semantic contradiction in prose requires judgement, which
belongs to the human or the agent, not the compiler. What the compiler can do is
guarantee that a contradiction someone noticed is never quietly lost — which is the
part that actually goes wrong.

**Honored by.** `docs/02-data-model.md`, `docs/05-validation-rules.md`,
`docs/09-context-assembly.md`.

---

## D-024 — Staleness propagates along three relations and flags, never overwrites

**Question.** How far "this record is at risk" travels.

**Resolution.** A record is **stale** if it is empirical and either (a) a commit
reachable from HEAD but not from its `observed_at` commit touched a path matching one
of its `watches` globs, or (b) its `review_after` date has passed.

Staleness propagates from a stale record to its dependents along exactly three
relations, in the dependent direction: `supported_by`, `depends_on`, `derived_from`.
Propagation is transitive, cycle-safe, and unbounded in depth. Dependents are flagged
`at_risk`, with the propagation path recorded so a reader can see why.

Neither flag ever changes a record's state, its content, or the truth value of any
claim (D-003). The flags appear in `akr review-queue`, in `REVIEW-REQUIRED.md`, and
as warnings in a context bundle.

Staleness is a **build fact, not a diagnostic**: it never enters the `AKR-*` diagnostic
stream and never affects the exit status of `akr check` or `akr build`. A project with
stale knowledge still builds — that is the point, since building is how you find out.
Projects wanting a hard gate opt in with `akr check --review-clean`.

**Rationale.** Three relations are the ones that mean "my correctness rests on yours".
Propagating along `part_of` or `after` would flag half the project every time a file
changes, and a warning that always fires is not a warning.

**Honored by.** `docs/10-freshness-and-git.md`, `docs/09-context-assembly.md`,
`docs/11-projections.md`, `spec/schema/index.sql`.

---

## D-025 — Generated views are committed build outputs, and CI enforces it

**Question.** Whether generated Markdown lives in the repository, and how the
never-hand-edit rule is enforced.

**Resolution.** `akr build` writes views to `docs/generated/` and they are committed,
so that people and tools reading the repository on the web see current knowledge
without running anything. Every generated file opens with:

```
<!-- GENERATED BY AKR — DO NOT EDIT
     source-graph: sha256:<hash>
     tool: akr <version>
-->
```

The banner does not embed a commit hash: these generated bytes participate in that
commit, so the hash is unknowable without a non-converging amendment loop. The stable
source graph maps back to commits through their `AKR-Graph` trailers.

`akr check --views-current` rebuilds views in memory and compares; any difference,
whether a hand edit or a stale build, fails with an emission diagnostic. That check is
the CI gate, and it is what gives the `sys.policy.no-hand-edited-views` record actual
force rather than good intentions.

**Rationale.** Committing generated output is a real cost — merge conflicts on
regenerated files — paid for by the ledger being legible to every reader and tool that
will never install AKR. The banner plus the CI gate makes the cost bounded and the
rule self-enforcing.

**Honored by.** `docs/11-projections.md`, `docs/06-compiler-pipeline.md`,
`docs/07-cli.md`, `examples/save-your-skin/docs/generated/`.

---

## D-026 — Planning kinds carry an optional `note` slot

*Amendment, 2026-08-04. Lead decision, taken on the P6a report; see the rationale below
for what prompted it. Unlike D-001..D-025 this entry postdates the spine's freezing, and
it landed with its implementation rather than in a commit of its own.*

**Question.** `docs/07` §6 said `akr abandon --reason` "lands in a `note`". No kind had a
`note` slot — only `disposition` blocks did — so the reason had nowhere to go. The P6
implementation wrote it as a leading comment, which works and is unsatisfying.

**Resolution.** `work`, `milestone` and `track` gain an optional `note` prose slot:
free-form operator commentary, informational only, with **no validation consequence**. No
rule reads it, nothing is required to set it, and nothing fails if it is absent or
nonsense. Views render it for records in terminal states, so an abandonment reason
appears in `DECISION-HISTORY.md` and the work projections rather than sitting in a
comment nobody renders.

`akr abandon --reason` writes it. Other operations may set it through an ordinary edit.

In canonical order it is the **last content slot of its kind** — `intent`, `target`,
`note` for milestones and work; `intent`, `cadence`, `note` for tracks — which puts it at
the end of the content group, immediately before claims and acceptance. That is as close
to the metadata group as a kind-specific slot can sit without inventing a new ordering
rank in D-012, and it reads correctly: the commentary comes after the thing commented on.

**Rationale.** A comment was the wrong home for two reasons. It is excluded from the seal
hash by D-015 — which is right for commentary and wrong for a reason somebody will later
need — and it is invisible to every generated view, so the operator who abandons a plan
on Tuesday leaves nothing the Thursday reader of `ACTIVE-WORK.md` can see. An
abandonment reason is durable knowledge and deserves a rendered slot.

Scoping it to the planning kinds is deliberate. Normative and empirical records already
have a place for every kind of prose they should carry — `rationale`, `context`,
`consequences`, `summary` — and a general-purpose commentary slot on them would become
the metadata bag `docs/02` §12 refuses to have. Planning records are the ones that get
abandoned, carried forward and re-scheduled by operators mid-flight, and that is the
commentary this slot is for.

**Honored by.** `spec/tables/vocabulary.json`, `docs/02-data-model.md` §4.9–§4.11,
`crates/akr-core/src/model/kind.rs`, `crates/akr-core/src/ops`, `docs/07-cli.md` §6
(Writer B, P6c), `docs/11-projections.md`.

---

## D-027 — A `papercut` kind, logged in the moment, with its own generated view

*Amendment, 2026-08-05. Like D-026 this entry postdates the spine's freezing and lands
with its implementation. It consciously extends D-001's closed set of twelve kinds to
thirteen; D-022 ("migration adds no kinds") is untouched — this kind comes from a
recorded decision, not from an import.*

**Question.** Agents hit small frictions while working — a tool call that missed and had
to be retried, a confusing setup step, a flaky command, a stale cache, a misleading
error, a non-obvious gotcha. None of them blocks; none of them is worth a work item; all
of them are worth knowing in aggregate, because logged together they show where the
project needs sanding down. Where do they go? Scratch is discarded, an `observation`
carries watch/staleness ceremony the moment does not want, and a Markdown file at the
repository root would be exactly the untyped pile AKR exists to replace.

**Resolution.** A thirteenth kind, `papercut`, in the **empirical** class: it records
what was found to be true at a stated point in history, which is precisely what a
friction report is. Two content slots, both filled automatically by the tooling:
`statement` (required prose — what you were doing, what got in the way, and a guess at
the cause or fix as a bonus) and `observed_at` (required commit, defaulted to HEAD). The
agent that hit it goes in the common `author` slot; the date in `created_at`. No
`watches`, so a papercut never goes stale and never enters the review queue; no
relations are required, so logging one is a single call.

The write surface is `akr papercut -m <agent> "message"` and the `knowledge.papercut`
MCP tool. Both allocate the key (`<namespace>.papercut.<slug-of-message>`), fill every
slot, and run the ordinary write pipeline — a papercut is a first-class record that
happens to cost one line to create.

The aggregate lives in a seventh generated view, `PAPERCUTS.md`, newest first, emitted
only once the ledger contains at least one papercut — a project that never logs one
never grows the file.

**Rationale.** The alternative of a free-form `PAPERCUTS.md` at the repository root was
rejected because it recreates the prose pile: no author an agent can trust, no commit,
no dedup handle, invisible to `akr search` and to the index. Making the record typed
costs nothing at the call site — the tooling fills every slot — and buys search,
provenance, and the one thing a papercut log is for: a reviewable aggregate.

Logging is proactive and in the moment. Mining a whole session for papercuts afterwards
is a language-model act, so it lives outside the tool (a harness command that reads the
transcript and calls `akr papercut` per finding), user-triggered, never in stages A–F
(D-020).

**Honored by.** `spec/tables/vocabulary.json`, `crates/akr-core/src/model/kind.rs`,
`crates/akr-core/src/papercut`, `crates/akr-core/src/render`, `docs/07-cli.md` §6,
`docs/08-mcp.md`, `docs/11-projections.md`.

---

## D-028 — Legacy-sourced completion is exempt from the descendant-commit gate

*Amendment, 2026-08-05.*

**Question.** D-016 / V-020 requires a `completed` record's acceptance evidence to have
an `observed_at` commit that descends from the last commit that changed the record's
content — the condition that stops a test from 200 commits ago closing a milestone
redefined yesterday. A historical port authors the record today, citing genuinely old
evidence commits from before the port existed: the record's own introduction to this
repository is necessarily the *newest* commit touching it, so its evidence can never
descend from it. That is not a data error to be fixed by re-running the check; it is
structurally impossible for a transcription of history to satisfy. Live case: `bpg-rs`'s
ledger carries 19 `AKR-R022` at HEAD for exactly this reason (`bpg.papercut.v-020-s-
descendant-commit-freshness-gate-akr/1`).

**Resolution.** When a `completed` record carries at least one `source { kind legacy
... }` block, the descendant-commit comparison of D-016 / V-020 is waived for its
acceptance evidence. Nothing else is: the cited reference must still resolve, the
evidence must still record `result pass`, and — whenever git facts are available at all
— its `observed_at` commit must still be one the repository actually has. Only the
comparison between that commit and the record's last content change is skipped. A record
with no `legacy` source keeps the full gate, unchanged.

The same exemption applies to `docs/11-projections.md`'s acceptance-verdict computation
(`akr-core::resolve::citation_facts`), which mirrors V-020's selection so that a rendered
view and the diagnostic it corresponds to never disagree about why a check is or is not
satisfied.

**Rationale.** A legacy-sourced record is a transcription of history: its git
introduction date says when it was *ported*, not when the work it describes happened.
Gating on descendancy from that introduction date would make every legacy port permanently
`AKR-R022`, forever, regardless of how solid its cited evidence is — a false alarm with no
action that clears it. The evidence commits are still the real, checkable claim about
when the work happened, so they remain required, must resolve, must pass, and must be
commits the repository can find: only the comparison that is structurally impossible for
a port to satisfy is waived.

**Honored by.** `crates/akr-core/src/validate/rules.rs` (`v020_acceptance_satisfied`,
`descends`), `crates/akr-core/src/resolve/mod.rs` (`citation_facts`),
`docs/05-validation-rules.md` (V-020), `docs/10-freshness-and-git.md` (the descendant
rule), `crates/akr-core/tests/v_rules.rs`.

## D-029 — The descendant gate measures the last *definitional* change, not the last transition

*Amendment, 2026-08-05.*

**Question.** D-016 / V-020 gates a `completed` record's acceptance evidence on descending
from "the last commit that changed the record's content." D-028 waived that comparison for
legacy ports, but the same wording bites ordinary, non-legacy work once the ledger is
committed. `akr complete` writes the record: it sets `state` to `completed` and adds a
`verified_by` to each satisfied check. Committing that completion is therefore, by the
literal reading, the record's *newest* content change — and the evidence, created before
the completion, can never descend from it. Every committed non-legacy milestone completion
would fail with `AKR-R022`, proven end to end: define a milestone, add passing evidence,
`akr complete` and commit, and `akr check` reports "evidence predates the last content
change." That defeats the verb the gate exists to serve.

**Resolution.** "Content change" in D-016 means a change to what the record *requires* —
its definition — not to its lifecycle bookkeeping. The commit a record's evidence must
descend from is the last commit that changed the record's **definitional** text: the
canonical record with the `state` slot, every acceptance-check `verified_by`, and the
D-026 `note` removed. `crates/akr-core/src/git/last_change_of` hashes that projection
(`resolve::definitional_record_text`) instead of the full canonical text, so a completion,
an abandonment, or a later note does not move `last_change`, while any change to `intent`,
a check's `statement`/`method`/`command`, `target`, or any other definitional slot still
does. The D-015 seal is untouched: it keeps hashing the whole record, because a seal
attests the literal bytes, not the definition.

D-028 stands and is still needed: a legacy port's *definition* is authored at the port
commit, so its older evidence still cannot descend and still relies on the legacy waiver.
D-029 narrows what counts as a definitional change; D-028 waives the comparison for
transcriptions of history. They are complementary.

**Rationale.** The gate's stated purpose is to stop a test from 200 commits ago closing a
milestone *redefined* yesterday. A state transition or an evidence citation is not a
redefinition, so counting it made the rule stricter than its purpose to the point of
forbidding the normal completion path. Hashing the definitional projection restores the
intended meaning without weakening it: real redefinitions still move the gate.

**Honored by.** `crates/akr-core/src/resolve/source.rs` (`definitional_record_text`),
`crates/akr-core/src/git/mod.rs` (`last_change_of`, `hash_at`),
`docs/05-validation-rules.md` (V-020), `docs/10-freshness-and-git.md` (the descendant
rule), `crates/akr-core/tests/git_queries.rs`.

---

## D-030 — Sister projects' papercuts collate into one master record in the owning ledger

*Amendment, 2026-08-07.*

**Question.** Papercuts are logged per project, each in its own ledger (D-027). An AKR
installation that also dogfoods on sibling repositories therefore scatters frictions
across as many ledgers as there are projects. The aggregate view `PAPERCUTS.md` shows
only the owning ledger's, so the sanding-down signal the kind exists to provide is
invisible across the set. How are the sisters' papercuts gathered into the AKR ledger —
read, summarised, deduplicated — without violating the rule that a write touches exactly
one ledger?

**Resolution.** `akr papercut collate` reads the live papercut heads of every workspace
under a scan directory — the direct subdirectories of `--projects <dir>`, defaulting to
the siblings of the workspace root — and proposes one master `papercut` record in the
owning ledger for every key not already absorbed. The absorbed keys land in a new,
optional `collated` slot (`string[]`) on the master record; that slot *is* the dedup
set, so the next run skips any key it names and, when nothing is new, exits 0 having
written nothing. A sister workspace that fails to load is recorded as skipped, not
fatal. The sisters are read, never written: no lock is staled in another project, no
record is added there, and the V-001 cross-ledger-reference problem never arises.

The master record is a normal papercut — `statement` and `observed_at` as D-027 fixes,
`state verified` — with `collated` as the one additional slot, so it renders in
`PAPERCUTS.md` like any other and is searchable through the same index.

**Rationale.** The alternatives were: writing to the sisters (rejected — a write is
one-ledger by design, and a staled lock in a project nobody is reading is invisible
trouble), and a derived file that lists keys without a record to hold them (rejected —
it recreates the untyped pile D-027 exists to replace and gives the dedup set no
durable, validated home). Making the absorbed keys structured `collated` content keeps
the dedup check a plain ledger read and the master record a first-class citizen.

**Honored by.** `spec/tables/vocabulary.json` (the `collated` slot, vocabulary 0.2),
`crates/akr-core/src/papercut/collate.rs`, `crates/akr-cli/src/write.rs`,
`crates/akr-cli/src/args.rs`, `docs/07-cli.md` §6.

## D-031 — External sources are an immutable library, indexed separately, cited by byte range

*Amendment, 2026-08-07.*

**Question.** An outside advisor report arrives as Markdown. The first attempt fragmented it
into one proposed record per heading, kept only the first paragraph of each, and let the
original be deleted — after which the detail was gone, the fragments had never been
reviewed, the graph did not connect them, and `ACTIVE-WORK.md` was full of unadopted
headings. Markdown is plainly the better format for the *first* reading of a rich technical
audit. What should AKR hold instead, and how should a record point at the part of a report
it was written about?

**Resolution.** Three layers, with one responsibility each.

1. **The immutable source library**, `sources/`, holds exact outside bytes. Registration
   (`akr source add`) content-hashes a document, copies it under `sources/external/` and
   adds a catalog entry. It creates **no records**. Editing a registered file is `AKR-S021`
   from `akr source verify` and from `akr check`; the only correction is registering a
   superseding version, and the older one stays retrievable.
2. **The derived source index**, `.akr/cache/sources.sqlite`, holds semantic chunks:
   heading paths, byte and line ranges, normalised search text and expanded technical
   symbols, ranked by BM25 over `source_chunks_fts`. It is rebuildable and
   non-authoritative. Chunk boundaries are a pure function of (bytes, parser version), and
   a chunk id is derived from both — so a scanner improvement changes chunk ids and
   nothing else.
3. **The ledger** holds the project's interpretation: what was adopted, rejected, deferred,
   verified. Records reach the library through a `source` block naming `document` plus
   `start_byte`, `end_byte`, `start_line` and `end_line` — all-or-nothing — with an
   optional `excerpt_hash`. `akr check` resolves those citations against the registered
   bytes and reports `AKR-S022` when one misses.

A citation names a **document and a byte range, never a chunk id**. Chunk boundaries belong
to a rebuildable index and are allowed to move; provenance is not.

The index lives in its own SQLite file rather than in `index.sqlite`. The record cache is
dropped and rebuilt wholesale whenever the ledger's source-graph hash moves — that is
every write — so sharing the file would rechunk the corpus on every record write and
re-resolve the ledger on every registration. Two files give the two generations the design
wants (`source_corpus_hash` here, `source_graph_hash` there) without threading a partial
rebuild through the record cache's drop-everything invalidation. It remains one storage
engine, one query language, one ranker: what was refused is a second *kind* of index, not
a second file.

Search over the library escapes punctuation by default. `akr search` takes raw FTS5, which
is a trap for an agent — `DecodeRequest::default()` is a parse error, not a query — so
`akr source search` quotes each term, `--literal` verifies an exact substring against the
stored bytes, and `--fts` is there for anyone who wants the operators.

**Rationale.** The alternative that was tried and superseded is line-by-line or
paragraph-by-paragraph record ingest: it turned every sentence of an unreviewed report into
project state, which is precisely what the acceptance and completion rules exist to keep
scarce. The original segmentation idea survives, in the one place where imperfect
segmentation is harmless — the derived index, where a badly placed boundary costs recall
and can never change what the project believes.

Sparse adoption is therefore the default. A record exists because the project *did*
something with the material: adopted it, rejected it, deferred it, or decided to track it.
Everything else stays readable in the report.

**Honored by.** `spec/schema/sources.sql`, `crates/akr-core/src/source/chunk.rs`,
`crates/akr-core/src/store/sources.rs`, `crates/akr-cli/src/source.rs`,
`crates/akr-mcp/src/schema.rs` (`knowledge.source_search`, `knowledge.source_get`),
`spec/tables/vocabulary.json` (the `source` block's citation slots),
`spec/diagnostics/codes-runtime.md` (`AKR-S022`),
`crates/akr-cli/tests/source_library.rs`.

## D-032 — AKR leads intent, git seals the snapshot, and the bridge is a change transaction

*Amendment, 2026-08-07.*

**Question.** An agent finished and verified a substantial change while every corresponding
work record still said `proposed`. A later check caught the divergence — real value, and
exactly what a Markdown roadmap would have missed — but only because someone ran the check.
Tooling should make the synchronised path easier than the unsynchronised one. Does that
need a `commit` record kind?

**Resolution.** No new record kind. A new **change transaction**, which is not part of the
knowledge graph at all.

A durable `commit` kind would be a second, worse copy of git's history: one work record
takes many commits, one commit advances several work records, rebasing and cherry-picking
change every object id, a commit hash cannot be written into a file contained in that same
commit without an amendment loop, and hundreds of commit records would drown out the
decisions and evidence the ledger exists to hold.

So the bridge is:

```text
AKR work record → local change transaction → staged git tree → generated commit → trailers
```

* The transaction lives at `$(git rev-parse --git-path akr/current-change.akr)`: local to
  this worktree, safe to discard, absent from search and context, never committed.
* **The staged tree is the synchronisation boundary**, not the working tree. The earlier
  "if the code is dirty the ledger must be dirty in the same direction" rule was both too
  strict — active work spans several commits without a new revision — and too loose, since
  it said nothing about *which* dirty files belong together. The git index already answers
  that.
* `akr diff --staged` computes a **semantic** delta by parsing the `HEAD` ledger and the
  index ledger and comparing them. It never reads `git diff` text: a reformat, a
  reordering or a moved record is not a semantic change and a textual diff cannot tell.
* `akr change prepare --staged` refuses a material code change that names neither a work
  record nor an explicit `--untracked-reason`, refuses when several work records moved and
  none was named primary, and records the staged tree id — so a tree that moves afterwards
  invalidates the preparation rather than producing a message about a different commit.
* `akr git commit` generates the message and hands the index to git. Git makes the commit;
  AKR does not implement an object store.
* The durable link is **commit trailers** — `AKR-Change`, `AKR-Work`, `AKR-Evidence`,
  `AKR-Decision`, `AKR-Graph`, `AKR-Tree`. They point from the commit to the records, which
  is the direction that has no hash cycle; they survive rebases and cherry-picks; and every
  AKR-to-git link can be rebuilt by walking history.

Evidence names the code it verified through an **implementation digest**: a hash of the
sorted `(path, mode, blob)` triples of the staged tree, excluding `.akr/**` and
`docs/generated/**`. Excluding AKR's own files is what breaks the cycle — writing the digest
into the ledger cannot change the digest — and it also makes the digest mean the right
thing: the implementation that was tested, not the tree including the note about testing it.

A commit still never completes work. Git can show that code exists, that tests ran and that
a commit landed; it cannot decide whether acceptance criteria were met. The transaction
*consumes* AKR state transitions and never invents them.

**Rationale.** Hooks were considered as the primary mechanism and rejected as the primary
mechanism: they are bypassable, and a hook carrying the checks would be a second
implementation nobody keeps in step. `akr git install-hooks` therefore writes two-line
wrappers around `akr git-hook`, which runs the same verification an author can run by hand,
and CI remains the final authority.

**Honored by.** `crates/akr-core/src/change/`, `crates/akr-core/src/git/mod.rs`
(`staged_entries`, `write_tree`, `git_path`, `log_grep`), `crates/akr-cli/src/change.rs`,
`docs/16-change-protocol.md`, `crates/akr-cli/tests/change_protocol.rs`.

## D-033 — A papercut says what it was about, and a collation carries the statements

*Amendment, 2026-08-08.*

**Question.** D-027 gave frictions a home and D-030 gathered the sisters' into one master
record. Two things then went wrong in use. First, `docs/findings/papercut-siloing-2026-08-07.md`
found the structural gap: a papercut is the one record kind whose subject is sometimes the
*tool* rather than the project being worked on, and nothing distinguished the two — a
friction with AKR, hit while working in `jpegxl-rs`, landed in `jpegxl-rs`'s ledger and was
invisible to anyone maintaining AKR. Second, the first real collation absorbed eighteen
papercuts and stored their keys and truncated titles, ending "see the owning project's
ledger for the full statement": eight checkouts to read eighteen findings, and none of them
were acted on for a week.

**Resolution.** Two changes, one to the record and one to the collation.

A papercut gains an optional `about` slot naming what the friction was *with*, when that is
not this project — `akr papercut -m claude --about akr "…"`. Absent means this project's own
code or setup, which is the common case and stays zero-ceremony. `akr papercut collate
--about <subject>` gathers only the sisters' papercuts aimed at one subject, and whatever
the filter leaves behind is *counted in the output* rather than silently dropped: a
collation that looked complete while ignoring two thirds of what it read would be worse
than none. `PAPERCUTS.md` renders subject-bearing papercuts under their own heading, so a
friction with somebody else's tool is not read as this project's backlog.

A collation now carries each source papercut's **full statement**, not a truncated title.
The master record is the only copy of those findings in the owning repository; a summary
that ends "go and look somewhere else" is a list of homework. The view keeps the record
compact — a collation renders as one line naming the counts — because the record being long
is right and the bullet list being long is not.

**Rationale.** The findings document listed four options: convention, a recognised tag,
routine discovery, and citing precedent. This is the second, made cheap by the third
already existing. Convention alone was rejected for the reason the document gives — it
relies on an agent knowing where a sibling checkout lives, which it usually does not — and
a cross-repository ledger was rejected as out of scope for 0.1 (`docs/01-architecture.md`),
which it remains. `about` is deliberately free text rather than an enum: the set of things
that can get in an agent's way is not closed, and a validated vocabulary of tool names
would need a decision every time somebody hit a new one.

**Honored by.** `spec/tables/vocabulary.json` (the papercut `about` slot),
`crates/akr-core/src/model/kind.rs`, `crates/akr-core/src/papercut/`,
`crates/akr-core/src/render/papercuts.rs`, `crates/akr-cli/src/write.rs`,
`crates/akr-mcp/src/schema.rs`, `crates/akr-cli/tests/writes.rs`.

## D-034 — The MCP surface is budgeted, instrumented, and says when it is stale

*Amendment, 2026-08-08.*

**Question.** `sources/context-reduction.md` reports AKR associated with 45% of a usage
window, and identifies the mechanism rather than the cause: an MCP tool result stays in the
conversation, so a four-thousand-token result read twenty times later costs eighty thousand
token-turns. Separately, AKR's own ledger carries a friction logged twice — once on
2026-08-05 and again on 2026-08-08 — in which an installed `akr-mcp` older than the
workspace rejects every write with a type error about a slot the agent never wrote, and
both times it was first diagnosed as a ledger bug.

**Resolution.** Four changes, all in `akr-mcp`, none in the ledger.

1. **Detail levels.** `knowledge.get` takes `summary | body | canonical`, defaulting to
   `body`. Canonical AKR syntax is the largest part of the payload and the least often
   wanted, so it is an explicit request: a tool whose cheapest call returns the most bytes
   will be called that way every time.
2. **Per-tool output budgets.** Every result is measured — *both* halves, because a client
   that renders `content` and one that parses `structuredContent` each pay for their own —
   and one over its hard limit is **withheld**, not shortened. The reply names the size, the
   limit, the top-level counts, and a ready-made narrower call. Truncation that does not
   say so is the failure the context budget already made once, and an agent that cannot
   trust a truncation marker asks for everything.
3. **A read-only surface.** `akr-mcp --surface read` serves the tools that answer questions
   and omits the ones that change the ledger. Tool schemas are a fixed cost in every session
   that loads them.
4. **Accounting and skew.** `--accounting <path>` appends one JSON line per call — sizes,
   estimated tokens, duration, whether the budget withheld it — so the next architectural
   judgement is measured rather than inferred. And the server publishes its vocabulary
   version in `serverInfo` and, on any failing call, compares it against the workspace's
   lock: a disagreement is reported as a named skew with the remedy, including the half
   everybody forgets, which is that reinstalling does not restart a running process.

**Rationale.** The alternative for (2) was to trim results to fit. Rejected: a partial list
that does not announce itself is worse than a refusal, because the caller acts on it. The
alternative for (4) was to make the install script louder. Also rejected, and it is the
same mistake in a different place — the friction recurred *after* the script already said
what to do, because the failure surfaces hours later inside a running session, which is
where the explanation has to be.

The budgets are engineering targets, not protocol requirements, and are expected to move
once the accounting says which tool actually dominates.

**Honored by.** `crates/akr-mcp/src/budget.rs`, `crates/akr-mcp/src/skew.rs`,
`crates/akr-mcp/src/protocol.rs`, `crates/akr-mcp/src/main.rs`,
`crates/akr-cli/src/args.rs` (`Detail`), `crates/akr-cli/src/commands.rs`,
`scripts/setup-akr-mcp.sh`.

## D-035 — A query is words; a dirty tree is a fact; ancestry is one walk

*Amendment, 2026-08-08.*

**Question.** Three frictions collated from sister projects have the same shape: a
defensible design decision that turns out, in use, to cost more than it buys.

1. `akr search` handed its query straight to FTS5. An agent asked for `HDR slice 6
   non-default feature` and got `no such column: default`; asked something containing a
   comma and got `fts5: syntax error near ","`. It gave up and grepped `.akr/records`,
   which is the one thing the search surface exists to make unnecessary.
2. `akr check --strict` exits 1 on `AKR-G004` alone — uncommitted edits in watched paths.
   An agent mid-task always has those, so "check is clean" was unreachable until commit.
3. `akr check`, `akr build` and `akr lock --update` each took over two minutes on a
   360-record, 330-commit ledger, dominated by one `git merge-base --is-ancestor` process
   per evidence citation.

**Resolution.**

**Queries are words.** `akr search` escapes its query into quoted phrase terms by default,
so punctuation is a token boundary rather than an operator. Raw FTS5 moves behind `--fts`,
and a malformed expression there says `drop --fts to search for the words themselves`. Over
MCP the escaped form is the *only* form: an agent composing a query has no way to know a
comma is an operator, and no reason to want one. This matches what `akr source search`
already did (D-031); the record search was simply the older of the two.

**`AKR-G004` is exempt from the strict promotion.** It is a fact about the working tree,
not about the ledger — it says the reader should not be misled, not that anything is wrong
— and it is exempt for the same reason staleness never changes an exit code (D-024). Left
promotable it offered two bad ways out: commit prematurely to satisfy a check, or run
`--lenient` and lose every other strict signal along with this one.

**Ancestry is one history walk.** `ancestry_over` asked git to compare pairs from inside a
comparison sort — O(n log n) process spawns to answer a question `git rev-list --topo-order`
answers once — and filtered its input with one `rev-parse` per commit. It now uses one
`cat-file --batch-check` for presence and one `rev-list --topo-order` for the order: two
processes rather than hundreds, with the same total order, because topological order is
exactly the guarantee the ancestry table needs.

**Rationale.** The first two are the same mistake in different places: a tool that is
correct about its own internals and wrong about the caller. Exposing FTS5 syntax is only
defensible if the caller knows they are writing FTS5, and neither an agent nor a hurried
human does. Promoting a working-tree observation to an error is only defensible if a clean
working tree is a reasonable precondition, and mid-task it is not.

The third was a straightforward algorithmic error hiding behind a correct answer. It is
worth naming because the shape recurs: a comparator that shells out is a comparator that
turns a sort into a fork bomb.

**Honored by.** `crates/akr-core/src/store/search.rs` (`escape_query`),
`crates/akr-core/src/git/mod.rs` (`contains_all`, `topological_order`, `ancestry_over`),
`crates/akr-cli/src/session.rs` (`is_fatal`), `crates/akr-cli/src/args.rs` (`--fts`),
`crates/akr-cli/tests/search.rs`.

---

## D-036 — Scratch is the one directory nothing empties, so the tool has to say so

*Amendment, 2026-08-19.*

**Question.** The protocol tells agents to put working files in `.agent/scratch/`, and
nothing ever removes them. Every other disposable location in a developer's life is
emptied by something: the OS clears its temp directory, `target/` is understood to be
throwaway and gets deleted without ceremony, `.akr/cache/` is rebuilt from source whenever
its inputs move. Scratch is none of those. It lives inside the repository, it is gitignored
rather than transient, and it survives every session — so it grows until a person notices
and deletes it by hand, reportedly past 30 GB across a set of sibling projects.

The guidance made it worse rather than better. `AGENTS.md` said scratch notes go there and
that "nobody reviews them and nothing depends on them", which reads as permission to forget
them. `scripts/agent-section.md` — the brief actually installed into every agent's global
instruction file, and therefore the only thing an agent working in a *different* project
ever reads — did not mention scratch at all.

**Question behind the question.** Is what is in scratch project knowledge? If it were, it
would want records, and the ledger would grow a row per temporary file. It is not: a
session's working files are not project conclusions, and the ledger exists precisely to
keep that sort of thing out. But one fact about scratch *is* durable and does get lost
between sessions — "this one is still needed, and here is why".

**Resolution.**

**Scratch is measured, reported, and never a ledger diagnostic.** `akr check` prints what
is there as a build fact, beside the stale-record counts it already prints. That is the
same standing staleness has under D-024: a fact about the workspace, not a contradiction in
the ledger, and so never a reason for the compiler to fail by itself.

**Failing is opt-in, and looks exactly like the review queue.** `akr check --scratch-clean`
raises `AKR-G042`, precisely as `--review-clean` raises `AKR-G041`. CI decides whether it
cares; the exit code keeps meaning what `docs/07` §5 says it means. Making it fail by
default would have put disk usage and ledger incoherence behind the same exit code, which
is the distinction the whole diagnostic scheme is built on.

**The keep marker lives in the scratch directory, not the ledger.**
`.agent/scratch/KEEP` is a plain list of `<name> <reason>` lines with `#` comments. It is
read by people at least as often as by the tool, so it is hand-editable, needs no parser to
understand, and survives the directory being moved or copied. An entry it names is never
pruned, whatever its age — an agent tidying up at the end of a session must not be able to
delete the thing the next session was told to expect.

**Pruning is by age, and by default the agent does it.** `akr scratch prune` removes unkept
entries untouched for fourteen days; `--older-than` and `--dry-run` adjust and rehearse it.
Fourteen days is long enough that a directory in use across several sessions survives, and
short enough that last month's never does. Age is measured from the *newest* file beneath an
entry, not the oldest: something touched yesterday is a day old however long ago it began.

**One directory, not two.** Handoffs and plans move from `.agents/` to `.agent/`, so there
is a single agent directory with exactly one ignored subtree inside it. Two names one letter
apart, one tracked and one ignored, produced workspaces containing both.

**And the guidance says all of this.** `scripts/agent-section.md` gains the paragraph it was
missing, which is the half of this that helps a project with no AKR in it at all.

**Consequences.** `akr check` gains a line of output in every workspace, including ones with
no scratch at all — deliberately, because an agent that never sees the directory named has
no reason to believe it persists. `akr scratch` is housekeeping rather than knowledge
compilation, which is a widening of what this tool does; it earns the place because the
directory is one the protocol itself creates.

**Honored by.** `crates/akr-core/src/scratch/mod.rs`, `crates/akr-cli/src/commands.rs`
(`scratch_list`, `scratch_prune`, `scratch_keep`, `scratch_fact`,
`scratch_clean_diagnostic`), `crates/akr-cli/src/args.rs` (`--scratch-clean`),
`crates/akr-cli/tests/scratch.rs`, `AGENTS.md`, `scripts/agent-section.md`.

---

## D-037 — A cited artefact must outlive the citation, and that is a rule, not a flag check

*Amendment, 2026-08-23.*

**Question.** D-036 made `.agent/scratch` an explicitly disposable directory that
`akr scratch prune` empties on an ordinary handoff. An evidence record whose `artifact`
points there is therefore a verified claim whose backing a later session removes without
knowing it was cited — and nothing notices, because a path is not a reference the ledger
resolves. The audit that prompted D-036 found 38 records across five files citing scratch
paths, most of them already dangling. The first fix put a refusal in front of one flag,
`akr evidence add --artifact`, raising `AKR-C004`.

**Question behind the question.** Where does an invariant about record content live? An
evidence record reaches the ledger by four routes: `akr evidence add`,
`akr evidence add-many --from`, `akr propose --kind evidence --from`, and
`knowledge.propose` over MCP. Three of those parse a record rather than building a request,
and never see the flag. A guard on one entry point is not an invariant; it is a
convenience that the next entry point silently repeals. Every other content-slot invariant
in the system — an observation's `observed_at`, evidence's `result` — is a `V-nnn` rule
over the model, and is therefore true of the ledger however a record got into it.

**Resolution.**

**It is V-025, raising `AKR-T023` at type-check.** The rule reads the `artifact` slot of
every evidence record and refuses a path under an unkept `.agent/scratch` entry. An entry
named in `.agent/scratch/KEEP` is explicitly persistent and therefore passes. Being a rule
rather than a flag check makes it true by construction on all four routes, and on any route added later.
It is a type-stage rule because it is a property of one record read against the vocabulary,
which is where V-009 and V-010 already live.

**It is still a refusal at write time.** Every write parses the ledger, applies the change
in memory, validates the *resulting* ledger, and only then writes atomically (`docs/07`
§4). So the rule refuses before anything lands, which is the property the original guard
was reaching for: a warning from a later build arrives after the record is in the ledger,
which is exactly when nobody goes back and moves the file.

**The flag check goes away rather than sitting beside it.** Two implementations of one
invariant is how they drift, and the rule's message covers the flag's case. `AKR-C004` is
no longer raised for this; `akr evidence add --artifact` under scratch now exits 1 with a
ledger diagnostic rather than 2 with a usage error, which is the more honest classification
— the record is wrong, not the command line.

**The comparison is textual, and reads the `artifact` slot only.** A path is not something
the ledger resolves, so there is nothing to look up; the check normalises separators, a
leading `./`, and an absolute path that passes through a scratch directory, because those
all name the same place. It does not read prose: a papercut whose statement quotes a
scratch path is describing the problem, not committing it.

**This does not re-open D-036.** D-036 says scratch is never a ledger diagnostic, and that
stands: it is about the directory's contents, how much is sitting there and how old, which
`akr check` reports as a build fact and never fails on by itself. V-025 is about a record,
and the record is wrong whether or not the file behind it still exists.

**It judges live revisions only.** *Amended 2026-08-25.* The rule first read every
revision in the ledger, which made it the one rule the sanctioned write path could not
satisfy: superseding a bad evidence record leaves the offending revision in the file, so
the repair reproduced the diagnostic it was repairing and `knowledge.revise` was refused
"because the superseded invalid revision still had to validate"
(`evidence-intake-project.papercut.correcting-an-invalid-scratch-backed-evidence`). The
same reading turned every write in a workspace with a long evidence history into a refusal
carrying ~230 diagnostics about records nobody was touching, and left a citation whose
path had already been pruned with no repair at all, because `akr scratch keep` cannot keep
a directory that is gone (`jpegxl-rs.papercut.after-akr-scratch-keep-visibly-added-a-new`,
`saveyourskin.papercut.starting-the-mealtime-furniture-session`). V-023 had settled this
shape already: a sealed revision is a fact of history, not a live claim. So V-025 filters
`is_live`, which restores the repair — revise the evidence to a durable path, and the old
revision seals as `superseded` and stops being judged.

**Consequences.** A ledger whose *live* evidence cites scratch artefacts fails `akr check`
until those records are revised — including on the next unrelated write, because the write
pipeline validates the whole resulting ledger. That is the intended forcing function, and
it is the same standing every other V-rule has, but it is a real migration cost for the
workspaces the original audit was about. `akr scratch keep <entry>` is the escape hatch for
an artefact that should stay where it is, and revising the record to a durable path is the
escape hatch for one whose artefact is already gone.

**Honored by.** `crates/akr-core/src/validate/rules.rs`
(`v025_evidence_artifact_durable`), `crates/akr-core/src/validate/mod.rs`,
`crates/akr-core/src/scratch/mod.rs` (`cited_entry`),
`crates/akr-core/src/diagnostics/mod.rs` (`T023`), `spec/tables/vocabulary.json`,
`spec/diagnostics/codes-lang.md`, `docs/05-validation-rules.md`, `docs/07-cli.md`,
`fixtures/validate/err/v025-evidence-scratch-artifact.akr`,
`fixtures/validate/ok/004-superseded-scratch-artifact.akr`,
`crates/akr-core/tests/v_rules.rs`, `crates/akr-cli/tests/writes.rs`,
`crates/akr-mcp/tests/writes.rs`, `AGENTS.md`.

## D-038 — Revising a record is not redefining it

*Amendment, 2026-08-25.*

**Question.** D-029 narrowed "content change" to mean a change to what a record
*requires*: `last_change` hashes a definitional projection with `state`, `note` and each
check's `verified_by` removed, so that completing a milestone does not count as the
milestone's newest content change and strand the evidence that closes it. That fixed the
in-place case. It does not reach the sealed case, and the sealed case is the ordinary one:
D-015 makes a sealed record uneditable, so revising it is the *only* way to change it.

Revision `n+1` did not exist before the commit that wrote it. Whatever it says, its last
definitional change is that commit by construction, and every piece of evidence it cites
was observed before it — so retargeting a check at a measurement that had already landed
un-satisfied the check, and the report that surfaced this says the rest: "there is no way
to re-land unchanged evidence", because a fresh `observed_at` would have to name a commit
that does not exist yet, and duplicating the evidence record duplicates a measurement
nobody re-ran (`jpegxl-rs.papercut.a-work-revision-that-cites-already-committed`, hit
while pointing a workspace-gates check at evidence from the same day; repaired by dropping
the revision commit, which is not a repair).

**Question behind the question.** D-029 asked which slots are bookkeeping. This asks the
same question one level up: which parts of a *revision* are bookkeeping? Two things
separate `n+1` from `n` whatever else changed — the revision number in the header, and the
`supersedes` edge back to `n` that `akr revise` writes. Neither is something the record
says about the world. They are how a revision is made.

**Resolution.** `last_change` compares definitions across the revision boundary. The
projection `last_change_of` hashes drops the revision number and a `supersedes` target
naming an earlier revision of the same key, on top of everything D-029 already removes;
the history walk, on reaching a commit where revision `n` does not yet exist, continues
against the newest revision at or below `n` rather than stopping. So the answer to "when
was this last redefined?" is the commit where the *definition* last changed, however many
times the record has been revised since.

A `supersedes` naming a *different* key is a real editorial statement and is not touched.
Everything D-029 keeps, this keeps: a changed `intent`, a changed check `statement`,
`method` or `command`, a changed `target` still moves the gate, and evidence from before
it is still too old. Searching at-or-below rather than exactly `n-1` covers a file version
that predates several revisions at once, which squashing and rebasing produce.

**The D-015 seal is untouched**, for the third time and the same reason: a seal attests
the literal bytes of one revision, so it keeps hashing `canonical_record_text`. Only the
freshness gate reads the definitional projection, and only the definitional projection
learns to ignore the revision.

**Consequences.** A revision that changes nothing definitional now preserves its
predecessor's `last_change`, so acceptance verdicts survive a lifecycle edit, a note, a
retitled check citation and a plain retarget. This can *lower* an existing `last_change`
for such a record — a ledger built before this change and after it can disagree about
which commit that was, which is a projection difference and not a diagnostic. Byte
reproducibility is unaffected: the computation remains a pure function of the repository
and the ledger.

**Honored by.** `crates/akr-core/src/resolve/source.rs`
(`revision_independent_definitional_text`, `strip_self_supersession`),
`crates/akr-core/src/git/mod.rs` (`hash_at`, `last_change_of`, `last_changes`,
`definitional_hashes`, `definition_at_or_below`), `docs/10-freshness-and-git.md`,
`crates/akr-core/tests/git_queries.rs`.

## D-039 — A write answers for what it introduces, not for what it inherits

*Amendment, 2026-08-25.*

**Question.** `docs/07` §4 says every write parses the ledger, applies the change in
memory, validates the **resulting** ledger, canonically formats, and only then writes.
"Validates the resulting ledger" was implemented as "the resulting ledger has no errors",
which is a stronger statement and a different one. It means a ledger that is already
invalid refuses every write, including the write that would repair it.

That is not hypothetical. V-025 arrived on 2026-08-23 and judges history: raw-autotune
carries 35 `AKR-T023` errors and Lege-ecosystem 20 more, from evidence recorded before the
rule existed, citing scratch entries `akr scratch prune` has since taken. Lege-ecosystem
carries 8 `AKR-R022`s besides. None of them can be repaired, because each repair is itself
a write and every write must first produce a ledger with none of the other faults in it.
Hand-editing a `.akr` file is the one escape, and the protocol forbids it. Two workspaces
were left unable to record anything at all — "`knowledge.propose` refuses to write because
the ledger does not validate … both pre-existing and unrelated, measured identically at
session start" — and the agent that hit it wrote its findings into a Markdown file
instead, which is exactly the outcome the ledger exists to prevent.

**Question behind the question.** What is the write pipeline actually promising? "The
ledger is always valid" is unenforceable the moment it is false: a rule added later, a bad
merge, a revert, a rewritten history. A promise that cannot survive its own violation is
not an invariant, it is a latch. The promise worth keeping is the one a caller can always
honour: **this write does not make the ledger worse.**

**Resolution.** Step 3 compares. It derives the diagnostics of the resulting ledger, and —
only when there are any — restores the pre-edit text of every file the operation touched,
re-derives, and takes the difference as a multiset keyed on code, rule, subject and
message, never on span. A diagnostic the write introduced refuses it, with nothing
written and the working tree byte-identical, as before. A diagnostic that was already
there does not, and is carried back on `Applied.diagnostics` with a note naming the count
and the codes.

The intake guarantee is unchanged, and this is the point worth checking rather than
assuming: evidence is born `verified` at revision 1, so a record citing scratch *today* is
a new subject, absent from the baseline, introduced, and refused — on all four write
routes, exactly as D-037 intended. What stops is the re-judging of records nobody is
touching.

**Consequences.** A broken ledger is repairable one record at a time through the sanctioned
path, which is the difference between a system that fails loudly and one that bricks. A
write can now land into a ledger that does not build; `akr build`, `akr validate` and
`akr check` are unmoved and still fail, so nothing about this hides drift — the write says
what it is leaving behind, and the build still refuses to call it done. The cost is two
extra derivations, paid only on the path that was about to fail anyway.

`akr_core::evidence`'s `settle` keeps the older reading. It is a pure function over a
candidate ledger with no notion of "the write", it has no production caller, and the
deadlock is a property of the pipeline rather than of validation.

**Two diagnostics went with it,** because the reports that surfaced the deadlock were
also sent the wrong way by the words. `AKR-T023`'s help offered only repairs that need the
artefact to still exist, when the case that produces it at scale is an entry `akr scratch
prune` has already taken; it now also names dropping the `artifact` slot, which is the one
repair left. `AKR-R022` called an off-branch commit an age: `LedgerFacts::ancestry` is a
topological order rather than the commit graph, so it always puts one of two commits
first, and a rewritten history reads as stale evidence. A separate reachability fact
(`LedgerFacts::off_branch`, one `rev-list --not <head>`) now distinguishes them. It moves
the wording only — never a verdict — so no workspace fails newly for being described
accurately.

**Honored by.** `crates/akr-core/src/ops/mod.rs` (`apply_inner`, `inherited_of`,
`introduced_of`, `swap_texts`, `fingerprint`, `inherited_note`),
`crates/akr-core/src/validate/rules.rs` (`commit_order`, `v025_evidence_artifact_durable`),
`crates/akr-core/src/git/mod.rs` (`unreachable_from`),
`crates/akr-core/tests/ops_write.rs`, `crates/akr-core/tests/v_rules.rs`,
`docs/07-cli.md` §4, `docs/05-validation-rules.md`,
`spec/diagnostics/codes-runtime.md` (`AKR-C031`).

---

## D-040 — Compress the state, not the search: an advisor packet has two layers and one of them is withheld

*Addition, 2026-09-06.*

**Question.** An agent doing the work reaches a point where a second, differently capable
model would help. How is the task handed over?

The obvious answer is a summary — "here is the project, here is what I found, here is where
I think the problem is" — and it is wrong in a way that is easy to miss, because it looks
like exactly the compression the receiving model wants.

**Question behind the question.** What is the advisor for?

If it is for confirming a conclusion, a summary is the right handoff. If it is for finding
what the first agent did not notice — which is the only reason worth paying a second model
— then a summary of what the first agent noticed is the worst possible input. It makes the
worker a perceptual bottleneck: the advisor inherits the framing, searches where the worker
searched, and refines instead of forming a view. The worker's blind spot silently becomes
the advisor's boundary, and afterwards nobody can tell, because a narrowed task reads
exactly like a narrow one.

The opposite failure is real too and was measured. An advisor arriving cold spends its
first and most expensive tokens on questions with deterministic answers: what is AKR, what
work is current, what does `base_rev` mean, what are the build commands, does `benches/`
exist, which document is authoritative — plus a cold dependency compile. In one logged
run the advisor had to discover and correct the `base_rev` requirement mid-task. That is
pure waste, and none of it is judgement.

**Resolution.** Both failures are the same mistake made in opposite directions, so state
the rule and build to it:

> **Compress state, not search.**

An **advisor packet** carries two categories in two fields.

**Layer A is administrative fact:** the user's request verbatim, `HEAD` and its subject,
the ledger revision, dirty paths with digests, the session head, declared namespaces, the
governing goal and records, the search envelope, build and test commands with what they
produced, baselines, constraints, evidence and artefacts already made. All of it may be
prepared and compressed, because compressing it loses nothing an advisor wanted.

**Layer B is worker interpretation:** hypotheses, what was examined, what was *not*
examined, proposed approaches, searches already run. The worker may record it and may not
substitute it for the problem.

**The reveal is a separate, recorded act.** `handoff open` renders Layer A and reports only
that Layer B exists. `handoff reveal` releases it and stamps the packet. Recording it is
the point: once you know an advisor read blind and then saw the notes, *"what did either
side miss?"* is a question with an answer, and across tasks that accumulates into evidence
about where each model actually sees — which is worth more than the handoff machinery.

**Three details carry most of the weight.**

*The task is verbatim.* A worker that has concluded the work is about guided chroma
denoising must not be able to replace "review this project and optimise for performance,
both memory and CPU" with its own conclusion. Interpretation has two legitimate homes,
`question` and `worker_notes`, and neither is `task`.

*The envelope defaults to the whole project.* `["**"]`, not the paths the worker touched.
A default drawn from what the worker examined would hand over the blind spot as a boundary
— the one thing the packet exists to prevent. It is a permission, not an instruction:
Layer A says how the kitchen is organised and never which drawer the answer is in.

*The workspace is fingerprinted.* Workers keep working after preparing a packet. `HEAD`,
the source-graph hash and a sha256 per dirty path are enough to detect drift, cost one
status call plus a read of files git already named, and leave the repository itself as the
thing the advisor reads — a full source snapshot would be a second copy of the checkout to
no purpose. `open` and `verify` report `exact` or `drifted` and name what moved. Drift
never changes the exit status: it is a fact about the working tree, not a contradiction in
the ledger, exactly as staleness is under D-024.

**A packet is not a record, and this is not a near miss.** It describes a workspace at a
moment, it is worthless once that tree has moved on, and it holds the worker's private
notes. Hundreds of them in `.akr/records/` would drown the decisions and evidence the
ledger exists to hold, which is D-032's argument against a `commit` kind applied to the
same shape of object. So packets live in `.agent/handoffs/`, gitignored, invisible to
`akr search`, `akr context` and the compiler. What survives a packet is whatever the
review made durable: a record, with evidence, through the ordinary write pipeline.

**AKR does not become the orchestrator.** It owns state, context, packet construction and
retrieval, provenance and snapshot verification. It does not launch models, choose between
them, price them, or know a particular harness's spawn mechanism. A packet is
provider-neutral; a data model coupled to this month's agent harness would date within a
release.

**Consequences.**

`.agent/` now has two ignored subtrees rather than the one D-036 settled on. That line of
D-036 was reasoning about *scratch*, and the reason it gave — two names one letter apart,
one tracked and one ignored — does not apply to a second subtree under the same name. Both
are disposable, both are ignored, and `akr init` writes both entries.

The MCP `Tool::writes` field widens from "writes to `.akr/records/`" to "changes the
workspace". `readOnlyHint` and `--surface read` both read it, and both would be lying if a
tool that created a packet reported itself read-only. A third runner, `run_coordination`,
sits beside `run_read` and `run_write`: it needs git facts, because a fingerprint is the
point, and it keeps the text half, because the useful output is the id to hand over.

`knowledge.handoff_open` gets an unusually large budget (2,000 / 3,500). It is the one read
whose whole purpose is that the second model arrives knowing the workspace. Because a
packet is addressable, the overflow path has somewhere real to send the caller —
`knowledge.handoff_expand`, one section at a time — rather than a truncated everything.

`knowledge.handoff_open` has no reveal-on-open argument, though `akr handoff open` has
`--reveal`. The CLI flag is one process and one recorded act; an MCP flag would have made a
tool declared read-only write, so the MCP surface composes it from two calls instead. The
one-implementation invariant holds: every MCP behaviour is reproducible from the command
line.

`scripts/agent-section.md` gains the workflow, which is the half of this that reaches
projects and harnesses that never read this file — including the instruction that *"call an
advisor"*, in whatever words the user uses, means preparing a packet rather than writing a
summary.

**Honored by.** `crates/akr-cli/src/handoff/` (`mod.rs`, `advisor.rs`, `packet.rs`,
`snapshot.rs`, `session_head.rs`), `crates/akr-cli/src/args.rs` (`parse_handoff`),
`crates/akr-cli/src/commands.rs`, `crates/akr-cli/src/init.rs` (`GITIGNORE_ENTRIES`),
`crates/akr-mcp/src/schema.rs`, `crates/akr-mcp/src/tools.rs` (`run_coordination`,
`handoff_create`), `crates/akr-mcp/src/budget.rs`, `crates/akr-cli/tests/advisor_packet.rs`,
`docs/17-advisor-packets.md`, `spec/diagnostics/codes-runtime.md` (`AKR-C043`),
`scripts/agent-section.md`, `.gitignore`.
