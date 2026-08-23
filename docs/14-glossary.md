# 14 — Glossary

Every term this design set uses in a specific sense, one sentence each, with the document
that defines it normatively. Where a term is easy to get wrong, the wrong version is
stated too.

Terminology here matches [`../spec/tables/vocabulary.json`](../spec/tables/vocabulary.json)
exactly. `tools/check-design.py` checks that no document uses a name the vocabulary does
not have.

---

## Structure

**Record** — The unit of knowledge: a key, a revision number, a kind, a state, and a body
of typed slots. Not a document, not a page, not a file. → [`02-data-model.md`](02-data-model.md)

**Key** — A record's stable identity, two to eight lowercase dotted segments
(`sys.policy.tandem-work`), whose first segment is a declared namespace. Never derived
from a path or a title. → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Namespace** — The first segment of a key, declared in `.akr/project.akr`. An undeclared
namespace is `AKR-L004`. → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Revision** — A numbered version of a record, written `@key/2`. Revisions are created,
never edited once sealed. → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Head** — The revision a floating `@key` resolves to: the live revision if there is one,
otherwise the end of the supersession chain. A floating reference always resolves;
liveness is a separate question. → [`04-references-and-versioning.md`](04-references-and-versioning.md) §3

**Slot** — A named, typed field inside a record (`title`, `state`, `rule`,
`observed_at`). Unique within its scope; snake_case, never hyphenated. →
[`03-syntax.md`](03-syntax.md)

**Block** — A named brace group inside a record: `claim`, `acceptance`, `check`, `source`,
`disposition`. Some repeat, `acceptance` does not. → [`02-data-model.md`](02-data-model.md)

**Claim** — An individually addressable assertion inside a record,
`claim <anchor> { text … }`, cited as `@key/2#anchor`. Versioned with its record, never
independently. → [`02-data-model.md`](02-data-model.md)

**Anchor** — A claim's or check's identifier, in key-segment form. Stable across revisions
while the meaning is unchanged. → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Retired claim** — An anchor a previous revision defined and this one drops, listed in
`retired_claims` so that a stale citation gets "retired at revision N" rather than "not
found". → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Reference** — One of exactly four forms: `@key`, `@key/2`, `@key#anchor`,
`@key/2#anchor`. No wildcards, no ranges, no cross-project references in 0.1. →
[`04-references-and-versioning.md`](04-references-and-versioning.md)

**Floating reference** — `@key` or `@key#anchor`: resolves to the head at build time, and
every such resolution is recorded in `akr.lock`. → [`04-references-and-versioning.md`](04-references-and-versioning.md)

**Pinned reference** — `@key/2` or `@key/2#anchor`: resolves to that revision, always. →
[`04-references-and-versioning.md`](04-references-and-versioning.md)

**Ledger** — The canonical layer: every `.akr` file under `.akr/`. The only source of
truth. → [`01-architecture.md`](01-architecture.md) §2

## Kinds and classes

**Kind** — One of exactly thirteen (D-001, extended by D-027): `term`, `requirement`, `policy`, `constraint`,
`decision`, `observation`, `evidence`, `assessment`, `milestone`, `work`, `track`,
`question`. → [`02-data-model.md`](02-data-model.md), D-001

**Class** — One of four groupings that carry the rules: **normative** (what ought to be
true), **empirical** (what was found to be true, at a stated commit), **planning** (what
is intended), **inquiry** (what is not known). Lifecycles, staleness behaviour and context
ordering are defined per class. → [`02-data-model.md`](02-data-model.md), D-002

**Plan of record** — The `work` record designated authoritative for a milestone or track
by a `plan_of_record` edge; at most one live per target. **There is no `plan` kind** — a
plan is a `work` record with a relation. → D-001, [`02-data-model.md`](02-data-model.md)

**Goal** — In this design set, only the `--goal` argument to `akr context`, naming the
planning record a bundle anchors on. **There is no `goal` kind.** →
[`09-context-assembly.md`](09-context-assembly.md)

**Track** — Standing, non-terminating work that no milestone contains. →
[`02-data-model.md`](02-data-model.md)

**Milestone** — A named point at which a defined set of acceptance checks passes.
Requires an `acceptance` block. → [`02-data-model.md`](02-data-model.md)

**Observation** — An empirical record of what was found true of the system at a specific
commit; requires `observed_at`. → [`02-data-model.md`](02-data-model.md)

**Evidence** — An empirical record of the outcome of a check that was actually run:
`result`, `method`, `observed_at`. It never declares what it verifies. →
[`02-data-model.md`](02-data-model.md), D-016

**Assessment** — A judgement drawn from observations, kept distinct from the observations
themselves. → [`02-data-model.md`](02-data-model.md)

## Lifecycle

**State** — A record's lifecycle state, drawn from its class's state set. Authored, never
derived. → [`02-data-model.md`](02-data-model.md)

**Live** — A state in its class's live set: `proposed`/`active` (normative), `verified`
(empirical), `proposed`/`ready`/`active`/`blocked` (planning), `open`/`deferred`
(inquiry). → [`02-data-model.md`](02-data-model.md)

**Terminal** — Any non-live state. Terminal records still resolve and still satisfy
historical references; they never enter an ordinary context bundle. →
[`02-data-model.md`](02-data-model.md)

**Sealed** — Any revision in a state other than `proposed`. Its content hash is recorded
in `akr.lock`, and editing it is `AKR-R051`. → D-015,
[`../spec/schema/akr-lock.md`](../spec/schema/akr-lock.md)

**Supersession** — Replacing a record with a new revision via a `supersedes` edge, which
puts the target into `superseded` state. Never a newest-wins tiebreak. → D-004,
[`04-references-and-versioning.md`](04-references-and-versioning.md)

**Disposition** — A block on a **superseding** planning record stating what happened to
each unfinished `part_of` child of the record it supersedes: `carried_forward`,
`completed_elsewhere`, `intentionally_dropped`, `still_required_separately`. → D-017

**Unfinished child** — A record in a live planning state related to a superseded record by
`part_of`; each needs a disposition or the build fails with `AKR-R014`. → D-017

**Archived** — Living under `.akr/archive/`, a filesystem convention with one consequence:
excluded from context bundles and from every view but `DECISION-HISTORY.md`. What makes a
record terminal is its *state*, not its path. → D-018

## Scope and governance

**Scope** — An array of scope terms declaring what a record governs or was observed about.
Required on normative kinds. → D-010, [`02-data-model.md`](02-data-model.md)

**Scope term** — `all`, `ref @key` (a milestone, track or constraint), or `path "glob"`. →
D-010

**Scope overlap** — The conservative test deciding whether two scopes touch: `all` overlaps
everything; two `ref` terms overlap if equal or connected by `part_of`; two `path` terms
overlap if their literal prefixes are prefix-comparable; a `ref` and a `path` term never
overlap. May report a false positive, never a false negative. → D-010

**Literal prefix** — The portion of a glob before its first wildcard, used for overlap and
watch matching. → [`10-freshness-and-git.md`](10-freshness-and-git.md)

**Topic** — An optional identifier on a normative record making it exclusive: two live
normative records sharing a topic with overlapping scope is `AKR-R002`. Records with no
topic never conflict by this rule. → D-004(b)

**Contradiction** — A declared `contradicts` edge, symmetric regardless of which side
declared it, which must be resolved or `acknowledged true`. The compiler does not infer
contradictions from prose. → D-023

**Acceptance** — The block of `check` blocks defining what "done" means for a milestone or
work record. → D-016

**Check** — One acceptance criterion: a `statement`, a `method`, an optional `command`, and
`verified_by` references to evidence. → D-016

**Satisfied** — A check with at least one referenced evidence record reporting `result
pass` whose `observed_at` commit **descends from** the last commit that changed the
verified record's content. → D-016

## Freshness

**Freshness** — Collectively, the derived `stale` and `at_risk` flags. Computed in stage D,
never authored, never written back to a source file. → [`10-freshness-and-git.md`](10-freshness-and-git.md)

**Stale** — A live empirical record whose watched path moved since its `observed_at`, or
whose `review_after` date has passed. A question raised, never an answer given. → D-024

**At risk** — A record that rests on a stale one along `supported_by`, `depends_on` or
`derived_from`, transitively. Carries the propagation path and depth. → D-024

**needs-review** — **Not a state.** Staleness is derived; there is no authored
`needs-review`, and nothing in `akr build` ever writes a `.akr` file. → D-003

**Build fact** — Something the build computes and reports that is not a diagnostic:
staleness and at-risk flags. No `AKR-*` code, no effect on exit status. → D-024

**`observed_at`** — The commit at which an observation or evidence record was made; 40
lowercase hex digits after `git:`, never abbreviated. → D-008,
[`10-freshness-and-git.md`](10-freshness-and-git.md)

**`watches`** — Globs on an observation answering "what change to the code would make this
wrong?". → [`10-freshness-and-git.md`](10-freshness-and-git.md)

**`review_after`** — A date on an observation answering "when should someone look at this
again regardless of the code?". → [`10-freshness-and-git.md`](10-freshness-and-git.md)

**Review queue** — The list of stale and at-risk records, ordered stale-first then by
propagation depth then by key; `akr review-queue` and `REVIEW-REQUIRED.md` are its two
faces. → [`10-freshness-and-git.md`](10-freshness-and-git.md) §7

## Pipeline

**Stage** — One of A parse, B type-check, C link, D resolve, E index, F emit. Stages
collect all their diagnostics and halt at the boundary if any error was collected. →
[`06-compiler-pipeline.md`](06-compiler-pipeline.md)

**Resolved model** — Stage D's output and the input to every later stage and every read
command: heads, relations, acceptance verdicts, freshness flags, diagnostics. →
[`06-compiler-pipeline.md`](06-compiler-pipeline.md) §6

**Determinism contract** — A build is a pure function of (source files, git commit, tool
version), byte-identical across machines. → [`01-architecture.md`](01-architecture.md) §4

**LLM boundary** — Stages A–F contain no model inference of any kind. Models may draft,
import, rank and summarise; they never determine authority, head resolution, scope
overlap, cycles, staleness, acceptance or supersession. → D-020

**Index** — `.akr/cache/index.sqlite`, the stage E cache: gitignored, rebuildable, never
authoritative, never read by anything outside the tool. → D-019,
[`../spec/schema/index.sql`](../spec/schema/index.sql)

**View** — A generated Markdown rendering of a query over the resolved model, committed to
the repository and never hand-edited. Six of them. → D-025,
[`11-projections.md`](11-projections.md)

**Banner** — The four-line comment every generated file opens with, naming the
source-graph hash, the commit and the tool version. No timestamp, deliberately. → D-025

**`akr.lock`** — The committed lock file, written in AKR syntax, recording build inputs,
source hashes, every floating resolution, and every sealed revision's content hash. →
D-014, [`../spec/schema/akr-lock.md`](../spec/schema/akr-lock.md)

**Revision content hash** — SHA-256 over the *canonically formatted* text of one record.
Reformatting cannot change it; editing a comment can. →
[`06-compiler-pipeline.md`](06-compiler-pipeline.md) §9

**Source-graph hash** — SHA-256 over the sorted `(path, raw-file-hash)` pairs of every
source file the build read. → [`06-compiler-pipeline.md`](06-compiler-pipeline.md) §9

**Context bundle** — The deterministic, eleven-section answer to "what do I need to know
before I touch this?". Membership is computed from the graph, never ranked. →
[`09-context-assembly.md`](09-context-assembly.md)

**Search ranks, never authorises** — The rule that no record's membership in a bundle and
no record's authority depends on any ranker. → [`09-context-assembly.md`](09-context-assembly.md) §1

**Scratch** — `.agent/scratch/`, the disposable layer. Gitignored, unreviewed, and the
reason the ledger does not have to absorb every thought. →
[`01-architecture.md`](01-architecture.md) §2

## Diagnostics

**Diagnostic** — A coded, span-bearing report of a fault: `AKR-<stage-letter><nnn>`. →
[`../spec/diagnostics/README.md`](../spec/diagnostics/README.md)

**Severity** — `error` or `warning`, and no third. Under the default `--strict` profile
warnings are errors. → D-013

**Strict / lenient** — `--strict` (default) makes warnings errors; `--lenient` downgrades
them and exists for `akr import` on legacy material, per invocation only. → D-013,
[`12-migration.md`](12-migration.md)

**Validation rule** — A `V-nnn` invariant. `V-001`–`V-025` are the language and graph
rules; `V-101`–`V-149` cover freshness, emission and context. A rule names the code it
raises. → [`05-validation-rules.md`](05-validation-rules.md)

**Registry** — One of the two diagnostic catalogues:
[`../spec/diagnostics/codes-lang.md`](../spec/diagnostics/codes-lang.md) for
`AKR-P/F/T/L/R`, [`../spec/diagnostics/codes-runtime.md`](../spec/diagnostics/codes-runtime.md)
for `AKR-I/E/X/G/C/M`. → D-013

## Terms this design set does not use

Stated so the checker can enforce them, and so a reader who expects them knows they are
absent by decision rather than by oversight.

| Not used | Use instead |
| --- | --- |
| a `plan` kind | a `work` record designated `plan_of_record` (D-001) |
| a `goal` kind | a `milestone`, `track` or `work` record (D-010) |
| `needs-review` as a state | the derived `stale` build fact (D-003) |
| `legacy-source` as a kind | a `source { kind legacy }` block (D-022) |
| "newest wins" | supersession, disposition, `topic`, `acknowledged` (D-004) |
| "document" for the unit of knowledge | "record" (D-018) |
| "page", "article", "note" | "record" |
| "frontmatter" | typed slots |
| "index" for the ledger | the ledger is `.akr/`; the index is a cache (D-019) |
| "invalid" or "false" for a stale record | "stale" — the compiler never declares a record false (D-003) |
