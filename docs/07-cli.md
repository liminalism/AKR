# 07 — Command Line Interface

The complete `akr` command surface: invocation model, global flags, exit codes, the
write pipeline, per-command reference, JSON output, and CI recipes.

Normative for command names, flag names, exit codes, the write pipeline's atomicity
guarantee, and the JSON envelope. Output text shown below is illustrative in wording and
normative in *content*: every field shown is present, and no command prints information
this document does not describe.

---

## 1. Invocation model

```
akr [GLOBAL FLAGS] <command> [COMMAND FLAGS] [ARGUMENTS]
```

Global flags are also accepted **after** the command, so `akr check --format json` and
`akr --format json check` are the same invocation. The grammar above is the canonical
form and the one this document uses; no command flag shares a name with a global one, so
accepting both costs nothing and refusing one would be enforcing punctuation rather than
meaning.

One binary, no daemon, no server, no state outside the workspace. Every command:

1. Locates the workspace by walking up from `--dir` (default: the current directory)
   until it finds a `.akr/` directory. Not found is `AKR-C011`, exit 3.
2. Reads `.akr/project.akr` for namespaces and defaults. Missing is `AKR-C012`, exit 3.
3. Runs whatever part of the pipeline it needs
   ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §1) and then does its work.

Commands divide into four groups, and the division is worth internalising:

| Group | Commands | Pipeline | Writes |
| --- | --- | --- | --- |
| **Read** | `get`, `search`, `context`, `impact`, `why-current`, `review-queue`, `view`, `explain` | A–E | nothing |
| **Verify** | `check`, `fmt --check` | A–D (+F in memory) | nothing |
| **Build** | `build`, `fmt`, `lock` | A–F | cache, views, lock, formatted sources |
| **Write** | `propose`, `revise`, `supersede`, `complete`, `abandon`, `evidence add`, `papercut`, `import`, `init` | A–D, then write | `.akr` source files |

Only the write group touches `.akr/records/`. `akr build` never does (D-003).

## 2. Global flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--dir <path>` | `.` | Where to start looking for the workspace. |
| `--strict` | **on** | Warnings are errors. The default profile (D-013). |
| `--lenient` | off | Warnings stay warnings. Intended for `akr import` on legacy material and nothing else. Combining it with `--strict` is `AKR-C005`. |
| `--format text\|json` | `text` | Output form. `json` is a stable envelope (§5); `text` is for humans and may be reworded between versions. |
| `--at <commit>` | `HEAD` | Resolve the build against this commit instead of HEAD. Changes staleness, acceptance descendancy, and the banner. Unknown commit is `AKR-G013`. Combining it with `--git-diff` is `AKR-C005`. |
| `--today <date>` | the system date | The date used for `review_after` comparisons. Exists so that tests and the worked example are reproducible; it is the only clock input to the build. |
| `--no-color` | auto | Disable ANSI colour. Also disabled when stdout is not a terminal. |
| `--no-rebuild` | off | Fail rather than rebuild the index. For read-only checkouts. Raises `AKR-I031` when a rebuild would be needed. |
| `--quiet` | off | Suppress progress lines; diagnostics and command output still print. |
| `--version`, `--help` | | Print and exit 0. |

An unknown flag is `AKR-C002`; an unknown command is `AKR-C001`; a missing required
argument is `AKR-C003`; a bad flag value is `AKR-C004`. All four exit 2.

## 3. Exit codes

| Code | Meaning | When |
| --- | --- | --- |
| **0** | Success | The command did what it was asked. Includes a `check` that found only build facts such as staleness. |
| **1** | Diagnostics | One or more `AKR-*` diagnostics of effective severity `error` were produced. Under `--strict` that includes warnings. |
| **2** | Usage | The invocation was malformed: `AKR-C001`–`AKR-C005`, `AKR-C041`. Nothing was read and nothing was written. |
| **3** | Environment | The workspace or repository is unusable: `AKR-C011`, `AKR-C012`, `AKR-C042`, `AKR-G001`, `AKR-G003`, `AKR-I003`. Not a ledger problem. |

`AKR-G013` — an unknown revision given to `--at` or to either end of `--git-diff` — is in
neither list, and so exits **1**. The checkout is fine and the invocation is well-formed;
the tool looked the revision up and it was not there, which is a finding about the
repository's contents and is reported as a diagnostic like any other.

The distinction between 1 and 3 is what makes CI logs readable: exit 1 means *fix the
ledger*, exit 3 means *fix the checkout*.

**Staleness never changes an exit code** (D-024). A project with two stale records and
four at-risk records exits 0 from `akr check`. The opt-in gate is `--review-clean`,
which raises `AKR-G041` and exits 1.

## 4. The write pipeline

Every command in the write group performs exactly this sequence, and none of them
performs a partial version of it:

```
   0. claim        exclusive access to this workspace
   1. parse        the current ledger                    (stage A)
   2. apply        the requested change, in memory
   3. validate     the RESULTING ledger                  (stages A-D)
   4. format       every touched record canonically      (D-012)
   5. write        touched files, atomically
```

Consequences, all of them load-bearing:

- **Validation is of the result, not the change.** Adding a record that creates a cycle
  fails, even though the record itself is well formed.
- **A write answers for what it introduces, not for what it inherits** (D-039). Step 3
  derives the resulting ledger's diagnostics and, when there are any, re-derives the
  pre-edit ledger and takes the difference. A diagnostic this write introduced refuses it.
  One the ledger already had does not: it is carried back as a reported diagnostic and a
  note naming the count and the codes. Without this an already-invalid ledger refuses
  every write, including the repair, and the only escape is hand-editing a `.akr` file,
  which the protocol forbids. `akr build`, `akr validate` and `akr check` are unaffected
  and still fail on the whole ledger.
- **Failure writes nothing.** If step 3 finds a diagnostic of effective severity error
  that this write introduced, the command reports them, raises `AKR-C031`, exits 1, and
  leaves the working tree byte-identical to how it found it. There is no partial write and
  no `.bak` file.
- **Every write is canonically formatted**, so a written record and a hand-written one
  are indistinguishable, and `akr fmt` on a freshly written ledger is a no-op.
- **Writes are per-file atomic**: write to a temporary file in the same directory, fsync,
  rename. A crash leaves either the old file or the new one.
- **One writer at a time, and a racing edit is refused rather than overwritten.** Steps 1
  to 5 are a read-modify-write: step 1 reads every source and step 5 writes each touched
  file whole. Per-file atomicity says nothing about two writers, so step 0 takes an
  advisory lock on `.akr/cache/write.lock` — through the operating system's file locking,
  released when the process ends however it ends, so there is no stale lock to reap.
  Writers therefore queue instead of colliding, and all of them succeed. Without it, two
  processes that both read before either wrote each held a ledger that did not know about
  the other's record, and the second rename silently discarded the first while both
  reported success; six concurrent `akr papercut` processes landed two records.

  A lock binds only the processes that take it, so immediately before the renames every
  file about to be replaced is re-read and must still hold the bytes step 1 read. A
  disagreement is `AKR-C034` and writes nothing: that catches an editor, a `git checkout`,
  or a filesystem whose locking does not work, none of which the lock can cover. `base_rev`
  (`docs/08-mcp.md`) is a separate and narrower control — optimistic concurrency over one
  key's head revision, which a newly proposed key does not engage at all.
- **Sealed revisions are refused up front.** Attempting to modify a non-`proposed`
  revision is `AKR-C032`, with the fix named in the message (`akr revise`). The
  build-time equivalent is `AKR-R051` (D-015). Attempting to write a revision that is not
  the head is `AKR-C033`.
- **Nothing is staged or committed.** The tool never runs `git add`. What to commit and
  when is the operator's decision.

## 5. JSON output

`--format json` prints exactly one JSON document to stdout, with this envelope:

```json
{
  "akr": "0.1",
  "tool_version": "0.1.0",
  "command": "check",
  "commit": "e806b3f54a2d7091c5e13b8a26f490dc7b135e64",
  "source_graph_hash": "sha256:4d1f8a0c93b7e256a1c4f0d8b39e6725c081af43d2e97b6051fc3a8d7e204b19",
  "ok": true,
  "exit_code": 0,
  "diagnostics": [],
  "result": { }
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `akr` | string | Envelope version. Bumped only on a breaking change to the envelope. |
| `tool_version` | string | Semver of the binary. |
| `command` | string | The command as invoked, with subcommand: `"evidence add"`. |
| `commit` | string or null | 40 hex for the newest commit that exactly contains the loaded AKR source graph; null outside Git or while those source bytes are dirty. Later projection-only commits do not advance it. |
| `source_graph_hash` | string | `sha256:` + 64 hex. |
| `ok` | boolean | `exit_code == 0`. |
| `exit_code` | integer | Mirrors the process exit status. |
| `diagnostics` | array | See below. Present and empty on success. |
| `result` | object | Command-specific; documented per command. |

The source-graph hash is the exact identity of the bytes a command read. A mid-flight
`akr build` may still refresh views, the lock, and derived indexes, but it does not attach
the pre-change `HEAD` as provenance for dirty AKR sources: the envelope and index store a
null/empty commit until those bytes are committed.

A diagnostic object:

```json
{
  "code": "AKR-R001",
  "severity": "error",
  "effective_severity": "error",
  "stage": "resolve",
  "rule": "V-012",
  "message": "two live revisions of one key",
  "path": ".akr/records/sys/work.akr",
  "line": 48, "column": 1,
  "key": "sys.work.m3-plan", "rev": 2,
  "notes": [ { "message": "revision 1 is also active", "line": 12, "column": 1 } ],
  "help": "supersede revision 1, or withdraw it (see V-012)"
}
```

`severity` is the diagnostic's declared severity. `effective_severity` applies the active
profile: under the default `--strict` profile a declared warning can therefore have an
effective severity of `error`, while intrinsic errors remain visibly distinct.

Rules for JSON output, which exist so that a script can rely on it:

- Object keys are emitted in a fixed order; arrays follow the ordering guarantees of
  [`06-compiler-pipeline.md`](06-compiler-pipeline.md) §11.
- Nothing is written to stdout except the document. Progress lines go to stderr, and
  `--quiet` silences them.
- Diagnostics appear in `diagnostics` **and** are not duplicated in `result`.
- `fmt` and `init` have no JSON form: their output is a file-system effect, not data.
  Requesting one is `AKR-C041`, exit 2.

---

## 6. Command reference

### `akr init`

```
akr init [--dir <path>] [--project <name>] [--namespace <name> ...]
```

Creates `.akr/project.akr`, `.akr/records/`, `.akr/archive/`, an `AGENTS.md` stub
([`08-mcp.md`](08-mcp.md) §8), and `.gitignore` entries for `.akr/cache/` and
`.agent/scratch/`.

An existing `.akr/` is `AKR-C013`; `akr init` never overwrites. A project name outside
key-segment form is `AKR-C023`.

```
$ akr init --project save-your-skin --namespace sys --namespace sim --namespace lege
created .akr/project.akr
created .akr/records/, .akr/archive/
created AGENTS.md
appended 2 entries to .gitignore
```

Exit 0, or 1 on `AKR-C013`/`AKR-C023`, or 3 if the directory is unwritable.

---

### `akr fmt`

```
akr fmt [--check] [<path> ...]
```

Parses each `.akr` file and re-emits it in canonical form (D-012): canonical slot order,
four-space indentation, arrays on one line under 96 columns and one element per line
above it, prose blocks dedented and re-indented, comments preserved with their D-006
attachment, exactly one blank line between records and none within them.

With no paths, formats every `.akr` file in the workspace including `akr.lock`. With
`--check`, writes nothing and reports differences as `AKR-F` diagnostics, exiting 1 if
any. `--format json` is `AKR-C041`.

```
$ akr fmt --check
error[AKR-F001]: file is not canonically formatted
  --> .akr/records/sim/work.akr:23:5
   |
23 |     state blocked
   |     ^^^^^^^^^^^^^ slot `state` must precede `intent`
   |
help: run `akr fmt`
1 file needs formatting
```

Exit 0 if clean, 1 if differences (with `--check`) or a parse error.

---

### `akr check`

```
akr check [--review-clean] [--views-current] [--at <commit>] [--today <date>]
akr validate [same flags]
```

Runs stages A–D and reports every diagnostic. This is the command CI runs.

`validate` is the same command under the name the MCP surface uses. The agent protocol
says "run `knowledge.validate` before handing work back", and from a shell that was not a
command at all — so the reader either found `check` by inference or gave up on the check.
One behaviour with two names in two places is not the rule this workspace keeps; one
behaviour with two names, both of which work, is the smaller wrong.

| Flag | Effect |
| --- | --- |
| `--review-clean` | Additionally fail if the review queue is non-empty: `AKR-G041`, exit 1. Opt-in, because staleness is a build fact and not a defect (D-024). |
| `--views-current` | Additionally run stage F in memory and compare against the committed views: `AKR-E011` (differs), `AKR-E012` (missing), `AKR-E013` (bad banner), `AKR-E014` (unexpected file). This is the D-025 gate. |

```
$ akr check
akr check — save-your-skin
  40 records, 42 revisions, 19 files
  commit e806b3f5, grammar 0.1, vocabulary 0.1

  stage A  parse         42 revisions        ok
  stage B  type-check    42 revisions        ok
  stage C  link          118 references      ok
  stage D  resolve       40 heads            ok

  build facts (not diagnostics):
    2 records stale, 4 at risk — see `akr review-queue`

no diagnostics
```

Exit 0. With diagnostics:

```
$ akr check
error[AKR-R014]: superseding plan does not dispose of an unfinished child
  --> .akr/records/sys/work.akr:61:1
   |
61 | record sys.work.m3-plan/2 : work {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ supersedes @sys.work.m3-plan/1
   |
note: @sys.work.m3-audio-pass is part_of the superseded plan and is in state `ready`
  --> .akr/records/sys/work.akr:96:1
help: add a `disposition @sys.work.m3-audio-pass { outcome ... }` block (see V-017)

1 error
```

Exit 1.

---

### `akr build`

```
akr build [--check] [--at <commit>] [--today <date>]
```

Runs stages A–F: everything `akr check` does, then the index, then the views, then
`akr.lock`. The exact sequence is [`06-compiler-pipeline.md`](06-compiler-pipeline.md)
§13. With `--check`, all stages still run, but outputs are validated in-memory and no
files are written.

```
$ akr build --check
akr build --check — save-your-skin
  42 records, 40 revisions, 19 files
  commit e806b3f, grammar 0.1, vocabulary 0.1

  stage A  parse         42 revisions        ok
  stage B  type-check    42 revisions        ok
  stage C  link          118 references      ok
  stage D  resolve       40 heads            ok
  stage E  emit (in memory)      6 views        ok
    roadmap                  current
    active-work              current
    review-required          current
    current-state            current
    open-questions           current
    decision-history         current
  stage F  index (in memory)   40 revisions, 37 indexed    ok
  lock check                   unchanged

  no diagnostics
```

`--check` exits 1 on parity errors and 0 when everything matches current sources.

```
$ akr build
parsed 42 revisions in 19 files
resolved 40 heads, 2 superseded revisions
2 stale records, 4 at risk (see akr review-queue)
wrote .akr/cache/index.sqlite
wrote docs/generated/ (6 views, 2 changed)
akr.lock unchanged
```

Exit 0, or 1 on any diagnostic — in which case nothing is written, because the pipeline
halts at the failing stage boundary.

---

### `akr view`

```
akr view <name> [--format text|json]
```

Renders one view to stdout without writing it. `<name>` is a catalogue name from
[`11-projections.md`](11-projections.md) §2, case-insensitively and with or without the
`.md` suffix: `roadmap`, `current-state`, `active-work`, `review-required`,
`open-questions`, `decision-history`. Anything else is `AKR-E003`.

Text output is the exact bytes `akr build` would write, banner included, which makes
`akr view roadmap | diff - docs/generated/ROADMAP.md` a hand-rollable version of
`--views-current`. JSON output is the view's model — sections and rows — before
rendering.

---

### `akr get`

```
akr get <ref> [--rev <n>] [--history] [--relations] [--format text|json]
```

Retrieves one record. `<ref>` is any of the four forms of D-009. With no revision, the
current head. `--history` lists every revision with its state and supersession edges;
`--relations` adds inbound edges, which are not visible in the source text.

A planning record's `acceptance` block is part of the body, with each check's statement,
method, command, citations and current verdict. `akr complete` demands a mapping for every
check, so a reader who cannot see the checks cannot supply one: the block used to appear
only under `--detail canonical`, and the first `complete` on a ten-check work item was
always refused for ten mappings its author had never been shown.

```
$ akr get @sys.policy.tandem-work
sys.policy.tandem-work/1 : policy    state active    head
  title    Engine and simulator advance in tandem
  scope    all
  topic    tandem-work
  freshness  at_risk (depth 2, via @sys.assessment.projection-gaps
                                -> @sim.obs.projection-gaps)

  rule
    No engine change lands without the matching simulator change in the
    same commit, except on the tracks listed under exceptions.

  claims
    #lag-bound     Permitted simulator lag is at most one milestone.
    #same-commit   Matching changes ship in one commit.

  relations (outbound)
    exceptions     -> @sys.track.lighting/1
    supported_by   -> @sys.assessment.projection-gaps/1
  relations (inbound)
    implements     <- @sys.decision.view-generation/1
```

Exit 0, or 1 if the reference does not resolve.

---

### `akr search`

```
akr search <query> [--kind <kind> ...] [--state <state> ...] [--limit <n>]
```

Full-text search over live revisions (`records_fts`), ranked by BM25 and then by key.
Filters are applied *before* ranking.

Ordinary multi-word queries match any term and rank records matching more of them first.
This makes a natural-language task description useful even when no one record repeats
every word. `--fts` is the explicit escape hatch for callers that need raw FTS5 operators.

**Search ranks; it never authorises.** Nothing enters a context bundle because it matched
a query ([`09-context-assembly.md`](09-context-assembly.md) §1). `akr search` is a
navigation aid for a human or an agent that already knows roughly what it is looking for.

```
$ akr search "frame budget" --kind constraint --kind observation
  0.91  sys.constraint.frame-budget-16ms/1  constraint  active    16 ms frame budget at p99
  0.74  lege.obs.frame-budget-headroom/1    observation verified  p99 frame time is 11.4 ms
2 results
```

A malformed query is `AKR-X031`; an unavailable backend is `AKR-X032`; a cache built
without FTS5 is `AKR-I022`. Exit 0 even with zero results — an empty result set is an
answer. If `.akr` sources changed since the last index build, search silently rebuilds
the disposable cache before querying it. Successful JSON output therefore carries
`"index_stale": false`. With `--no-rebuild`, a missing or stale cache is `AKR-I031` and
remains untouched.

---

### `akr start`

```
akr start <task> [--paths <glob> ...] [--budget <tokens>]
```

Use this when a task arrives without an exact planning key. Before task matching, it emits
one compact **session head**: the latest Git commit, the latest reachable commit carrying
`AKR-Work`, the current heads named by that commit and their related records, every live
planning branch, unsatisfied acceptance and review attention, and a semantic summary of
valid uncommitted AKR changes. It then searches live milestones, work and tracks and
returns candidates plus a ready-made `akr context` request when one match is unambiguous.
It is the CLI counterpart of `knowledge.start`.

The session head is a deterministic projection, not a record and not authority. When the
working ledger validates, it is shown as a labelled overlay on HEAD. When it does not,
start parses and validates the committed `.akr` tree separately, excludes the invalid
overlay, and orients the task from that snapshot. If neither validates, start refuses to
invent a briefing. `--budget` bounds the combined handoff and task orientation; omitted
lower-priority entries are counted and point to focused follow-up calls.

If no planning record matches, the result explicitly says that this is a ledger-coverage
miss, not evidence that no plan exists. It also includes a bounded workspace text scan of
tracked and untracked files. Those hits are labelled `workspace text scan;
non-authoritative and not AKR knowledge`; they are discovery leads, not planning authority.
For multi-word tasks, a fallback hit must contain at least two useful task terms so generic
words do not swamp more specific intake material.

---

### `akr context`

```
akr context --goal <key> [--paths <glob> ...] [--budget <tokens>]
            [--format text|json]
```

Assembles the deterministic context bundle specified in
[`09-context-assembly.md`](09-context-assembly.md). The `--goal` must be a live
`milestone`, `work` or `track` record: unresolvable is `AKR-X001`, terminal is
`AKR-X002`, wrong kind is `AKR-X003`. A pin to the current head is normalized; an older
pin is `AKR-X004`, and an anchored reference is `AKR-X005`, both with retrieval guidance.
A malformed `--paths` glob is `AKR-X011`; one that
matches nothing is `AKR-X012` (warning). A budget too small for the mandatory sections is
`AKR-X021`; prose truncation is `AKR-X022` (warning).

This is the single most important command for agent use, and the one an agent should
call first in any session. A full worked bundle is in `09-context-assembly.md` §7 and in
[`../examples/save-your-skin/transcripts/akr-context.txt`](../examples/save-your-skin/transcripts/akr-context.txt).

---

### `akr impact`

```
akr impact <ref> | --git-diff <A>..<B> [--depth <n>] [--format text|json]
```

Two modes.

**Record mode** — `akr impact @key` — reports what depends on a record: the reverse
closure along `supported_by`, `depends_on` and `derived_from`, with the path and depth of
each dependent. Answers "what breaks if I supersede this?".

**Git mode** — `akr impact --git-diff C4..C5` — reports what a *commit range* would make
stale: which `watches` globs the range's touched paths match, which records own them, and
what propagates from there. Answers "what does this branch invalidate?", which makes it
the natural pre-commit and pre-merge hook
([`10-freshness-and-git.md`](10-freshness-and-git.md) §6).

```
$ akr impact --git-diff C4..C5
range 5d9c2a70..e806b3f5, 1 commit, 2 touched path groups
  lege/src/render/**    matches watches of:
    lege.obs.frame-budget-headroom/1   observed_at e806b3f5 — already current
  docs/generated/**     matches no watches

newly stale: none
newly at risk: none
```

An unknown revision on either side of `..` is `AKR-G013`. Combining `--git-diff` with
`--at` is `AKR-C005`. Exit 0; `impact` reports, it does not judge.

---

### `akr why-current`

```
akr why-current <ref> [--format text|json]
```

Explains, mechanically, why the tool considers a record's head to be the head and its
freshness to be what it is. Prints the supersession chain, the head-resolution verdict,
the lock entry, the freshness derivation with the deciding commit or date, and the
propagation path if the record is at risk.

```
$ akr why-current @sys.assessment.projection-gaps
sys.assessment.projection-gaps — head is revision 1

  head resolution
    revision 1  verified   LIVE      -> head
    (no other revisions)
    lock: resolved to /1, hash sha256:9c02… (sealed, matches)

  freshness
    not stale: no `watches` globs, no `review_after`
    AT RISK (depth 1)
      @sim.obs.projection-gaps/1 is stale
        cause: watches "sim/src/project/**" matched by 5d9c2a70
        observed_at 7c41d0ba, which 5d9c2a70 descends from
      propagated via: supported_by
```

This command exists because "why does the tool think that?" is the question a knowledge
system must always be able to answer. Exit 0, or 1 if the reference does not resolve.

---

### `akr explain`

```
akr explain <code> | <rule>
```

Prints the registry entry for a diagnostic code (`akr explain AKR-R014`) or the rule
catalogue entry for a validation rule (`akr explain V-017`): title, severity, stage, the
rule it enforces or the code it raises, the message template, cause, fix, and a minimal
reproducing source.

Needs no workspace: it reads only the compiled-in registries. Exit 0, or 2 if the
argument is neither a registered code nor a known rule.

---

### `akr propose`

```
akr propose <key> --kind <kind> [--title <text>] [--from <file>] [--edit]
```

Creates revision 1 of a new key in the initial state of its class — `proposed` for
normative and planning kinds, `open` for `question`, `verified` for empirical kinds,
which have no proposal state. Writes into the conventional file for the key's namespace
and kind group, creating it if needed.

An existing key is an error: use `akr revise`. An undeclared namespace is `AKR-L004`.
The write pipeline of §4 applies in full, so a proposal that would break the ledger is
refused and nothing is written (`AKR-C031`).

**A body source is effectively mandatory.** `--from` and `--edit` read as optional, and
they are not: §4 validates the *resulting* ledger, every kind requires its prose slot
(`AKR-T001`), and V-008 refuses an empty one. A `propose` with neither flag therefore
produces a record with no `definition`, `statement`, `intent` or `rule` — and is refused
before anything reaches the disk. The flags are optional in the grammar and required in
practice; nothing is written either way, so the failure is safe, but it is worth knowing
in advance rather than discovering.

```
$ akr propose sys.term.day-loop --kind term --title "The day loop"
error[AKR-C031]: write aborted: this write would introduce 1 diagnostic(s); nothing was written
error[AKR-T001]: term requires slot `definition`
nothing written
```

---

### `akr revise`

```
akr revise <key> [--from <file>] [--edit] [--state <state>]
```

Creates revision *n+1* of an existing key by copying the head and applying edits. This is
the only way to change a settled record (D-015), and `AKR-C032` names it when someone
tries the other way.

**A revise on a sealed head retires the old head in the same write.** Revision *n+1* is
created, it gains a `supersedes` edge to *n*, and *n* moves to `superseded` — one atomic
write, not two. The old revision's *body* is untouched and its `supersedes` chain is
exactly what `akr supersede` would have produced; only its `state` slot changes.

**Live non-historical pins of *n* follow onto *n+1* in that same write.** A `supported_by`,
`depends_on`, `implements`, `verified_by` or other non-historical pin of `@key/n` would
become `AKR-L021` the moment *n* is superseded, and the referrer cannot be moved to *n+1*
first because *n+1* does not exist yet. The write rewrites those pins onto the successor
and does not create a revision of the referrer: it is the same accompanying graph
maintenance that retires the old head's state. Historical pins (`supersedes`,
`contradicts`, `derived_from`) stay on *n* — they cite that revision for good. So does
`part_of`: V-017 dispositions name the children of that plan revision, and moving the pin
would make every disposition miss. The lock then records the earlier pin as `AKR-R052`
until the next `akr build`, not as a sealed body edit (`AKR-R051`).

This is not a convenience. Two live revisions of one key is `AKR-R012` (V-012), and §4
refuses to write a ledger that does not validate — so a revise that created *n+1* and left
*n* live would have to be refused, and the "intermediate state" in which the old head is
still live cannot be written at all. `akr revise` on a sealed head and `akr supersede`
share one implementation for that reason;
[`04-references-and-versioning.md`](04-references-and-versioning.md) §2.1 states the same
rule from the model's side.

One consequence follows for planning records: **a revise on a sealed planning head demands
a disposition for every unfinished `part_of` child**, exactly as `akr supersede` does
(D-017). The requirement belongs to the act of replacing a plan, not to the name of the
command that does it, and `--disposition` is accepted here for that reason.

A `proposed` head is edited in place instead, and no revision is created: D-015 makes
proposed revisions editable, and revision 2 of a proposal nobody accepted would be noise.
`--in-place` forces the in-place path and is `AKR-C032` on a sealed head.

Because every write changes a record's canonical text, it changes its content hash, and
`akr.lock` records the old one. `akr revise` therefore reports that the lock is stale and
`akr check` raises `AKR-R052` until the next `akr build`. That is correct: a lock records
a *build* (D-014) and no write operation may invent one.

`--state` moves the new revision along its class's lifecycle. An illegal transition is
`AKR-T011` (V-007). If `--state` is omitted while changing sealed content, the successor
starts `proposed` so the changed knowledge must be accepted again. A `state` slot in a
`--from` file is the same request: it is honoured as `--state` would be. The write
prints a note when it has reset a sealed head to the class initial state.

`--from` is a slot-list overlay, not a replacement of the record. The successor is the
head with the named slots applied, which is the same merge `knowledge.revise` already
does. Unmentioned content slots, relations, scope, acceptance, claims and provenance
stay; a slot named in the file replaces that slot, including emptying it. A fragment
that only rewrites `intent` therefore cannot drop an `acceptance` block the author
never mentioned.

---

### `akr supersede`

```
akr supersede <old-key> --with <new-key> [--disposition <child>=<outcome>[:<into>] ...]
```

With a different key, the replacement must already exist as a `proposed` record of the
same kind: propose its complete body first, then run this command. The second step adds
the pinned `supersedes` edge and moves the old head to `superseded`; for planning records
it also requires a `disposition` block for every
unfinished `part_of` child of the old record (D-017).

The command **lists the children it needs a disposition for and refuses to write until
each has one**. This is the single most valuable interaction in the tool: it is the
moment the author knows the answer and the only moment anyone will ever ask.

```
$ akr supersede sys.work.m3-plan --with sys.work.m3-plan
error[AKR-R014]: superseding plan does not dispose of an unfinished child
  2 unfinished children of @sys.work.m3-plan/1:
    @sys.work.m3-lighting-pass   ready
    @sys.work.m3-audio-pass      ready
help: rerun with, for example,
  --disposition sys.work.m3-lighting-pass=carried_forward:sys.track.lighting
  --disposition sys.work.m3-audio-pass=intentionally_dropped
nothing written
```

Exit 1, working tree untouched.

---

### `akr complete`

```
akr complete <key> [--check <id>=<evidence-ref> ...]
```

Moves a `milestone` or `work` record to `completed`. Every acceptance check must be
satisfied by evidence with `result pass` whose `observed_at` commit descends from the
last commit that changed the record's content (D-016). An unsatisfied check is
`AKR-R022` (V-020) and nothing is written.

```
$ akr complete sys.milestone.m3-playable-day
error[AKR-R022]: acceptance check is not satisfied
  --> .akr/records/sys/milestones.akr:74:9
   |
74 |         check no-placeholder-assets {
   |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^ no evidence with result `pass`
   |
help: run the check and record it with
  `akr evidence add sys.evidence.asset-audit --result pass --method command`
  then `akr complete sys.milestone.m3-playable-day
        --check no-placeholder-assets=@sys.evidence.asset-audit/1`
nothing written
```

**Completing a milestone requires its plan of record to be retired first.** V-019 refuses
a live `work` record whose `plan_of_record` resolves to a terminal milestone, so completing
the milestone while its plan is still `active` produces a ledger that does not validate,
and §4 refuses to write it:

```
$ akr complete sys.milestone.m3-playable-day \
      --check no-placeholder-assets=@sys.evidence.asset-audit/1
error[AKR-C031]: write aborted: this write would introduce 1 diagnostic(s); nothing was written
error[AKR-R021]: sys.work.m3-plan/2 is active but `plan_of_record` resolves to
                 sys.milestone.m3-playable-day/1, which is completed
help: repoint the reference, or revise this record (see V-019)
nothing written
```

The order is `akr complete` or `akr abandon` on the plan, then `akr complete` on the
milestone. This is the rule doing its job rather than getting in the way: a plan is a
statement about work that is still to be done, and a plan of record for a finished
milestone is a claim that has stopped being true. The tool makes you say which — the plan
was completed, or it was abandoned — at the moment you know.

---

### `akr abandon`

```
akr abandon <key> --reason <text> [--disposition <child>=<outcome>[:<into>] ...]
```

Moves a planning record to `abandoned`. Like `supersede`, it demands a disposition for
every unfinished child — abandoning a plan silently is exactly the failure D-017 exists
to prevent.

`--reason` is required and lands in the record's **`note` slot** (D-026), not in a
comment. The distinction matters twice over: a comment is excluded from the seal hash by
D-015, and it is invisible to every generated view. An abandonment reason is durable
knowledge, so it is stored as content and
[`11-projections.md`](11-projections.md) §3 renders it wherever a terminal planning record
appears — `ACTIVE-WORK.md`, `ROADMAP.md` and `DECISION-HISTORY.md`. The operator who
abandons a plan on Tuesday leaves something the Thursday reader can see.

Nothing is deleted; the record stays, terminal, and its references keep resolving.

---

### `akr papercut`

```
akr papercut -m <agent> "message" [--namespace <ns>]
```

Logs a small friction hit while working — a tool call that missed and had to be
retried, a confusing or undocumented setup step, a flaky command, a stale cache, a
misleading error, a non-obvious gotcha — as a `papercut` record (D-027). One or two
sentences: what you were doing, what got in the way; a guess at the cause or fix is a
bonus. Logged proactively, in the moment: none of these block, and together they show
where the project needs sanding down.

The message is the whole ceremony. The key is allocated
(`<namespace>.papercut.<slug-of-message>`, suffixed on collision), `observed_at`
defaults to HEAD, the `-m` value lands in `author`, and the date in `created_at`. The
write runs the full pipeline of §4 like every other write.

`--namespace` chooses where the key goes, and is never required. With several declared,
the default is the namespace this project's papercuts already live in, by count; a
workspace that has logged none falls back to the namespace carrying the most records —
the project-wide one in practice — and then to the alphabetically first. So the first
papercut of a workspace picks a place and every one after it follows, without being told.
Declaration order is not available to decide this: `project.akr`'s namespaces are a set
in the model, not a list.

The aggregate is `docs/generated/PAPERCUTS.md`, emitted by `akr build` once at least
one papercut exists, newest first.

Mining a whole session for papercuts afterwards is a language-model act and lives
outside this tool (D-020): a harness command reads the transcript and calls
`akr papercut` once per finding, user-triggered.

#### `akr papercut collate`

```
akr papercut collate [--projects <dir>] [--about <subject>]... [--all]
                     [--namespace <ns>] [--dry-run]
```

Collates the live papercut heads of the sister projects around the AKR workspace into
one master `papercut` record (D-030). `--projects` names the directory to scan (direct
subdirectories holding an `.akr/`); default is the siblings of the workspace root. The
sisters are read, never written.

Each absorbed key lands in the master record's `collated` slot — the dedup set the next
run checks, so a source papercut is processed once and a second run with nothing new
exits 0 and writes nothing. Terminal master records still contribute to this historical
dedup set: resolving or withdrawing the master does not make its source reports new.
A broken sister workspace is skipped, not fatal. `--namespace` is needed only when the
project declares several.

**The scan is global; the decision is local.** Every sister ledger is read, but only the
papercuts aimed at *this* project should land here, and what marks one is the free-text
`about` subject that whichever agent hit the friction happened to type. Three things
follow, and together they are what makes a global scan usable from one workspace:

- `--about` is **repeatable** and compares subjects with case and punctuation folded away,
  because one tool is not one name: `akr`, `AKR` and `akr-mcp` are three strings for work
  the same person owns. Nothing else is folded — `akr` and `akr-mcp` stay distinct, since
  merging them would be the same silent over-reach in the other direction.
- Whatever the filter leaves behind is reported **broken down by subject, with a count
  each**, and the largest is offered as a copyable `--about`. A bare total tells a reader
  they have missed something and nothing about what to do next; `FFF 10, fff 8, ripgrep
  30` tells them the first two are theirs under two spellings and the third is not.
- `--dry-run` prints what would be absorbed, entry by entry, and writes nothing.

`--about` and `--all` are mutually exclusive. Absent both, the filter is this project's
namespace.

`collate` as the sole positional is the subcommand **whatever other flags are present**,
including `-m`. It did not use to be: `-m` marked the logging path, so
`akr papercut collate -m <agent>` — the natural spelling, since plain `akr papercut`
requires `-m` — silently logged a one-word papercut instead of collating. `--` is the
escape for a message that really is that word:

```
akr papercut -m claude -- collate
```

---

### `akr scratch`

```
akr scratch list
akr scratch prune [--older-than <days>] [--dry-run]
akr scratch keep <name> --reason <text> | --forget
```

`.agent/scratch/` is where the protocol tells agents to put working files, and it is the
one disposable location in a workspace that nothing empties on its own. The OS clears its
own temp directory; `target/` is deleted without a second thought; `.akr/cache/` is rebuilt
whenever its inputs move. Scratch is gitignored but lives *inside* the repository and
survives every session, so it accumulates until a person clears it by hand (D-036).

`list` shows every top-level entry, largest first, with its size, its age in days, and
whether it is kept or prunable.

`prune` removes entries that are unkept **and** untouched for at least `--older-than` days,
default 14. Age is taken from the newest file beneath an entry, not the oldest: a directory
touched yesterday is a day old however long ago it was created, and pruning by start date
would throw away live work. `--dry-run` prints what would go and removes nothing. A file
that cannot be removed is reported and the rest still go, so the command is worth running
again rather than being all-or-nothing.

`keep` records that an entry is still needed, in `.agent/scratch/KEEP` — one
`<name> <reason>` per line, `#` for comments, editable by hand. **A kept entry is never
pruned, at any age.** That is the point: an agent tidying up at the end of a session must
not be able to delete what the next session was told to expect. `--reason` is required,
because a marker with no reason outlives whatever made it necessary. `--forget` releases it.

`akr check` prints the total as a build fact beside the stale-record counts, and
`akr check --scratch-clean` turns anything prunable into `AKR-G042` and exit 1 — the same
opt-in shape `--review-clean` has for the review queue. Neither command ever deletes
anything; removal is always explicit.

---

### `akr evidence add`

```
akr evidence add <key> --result pass|fail|inconclusive
                       --method manual|command|observation
                       [--command <text>] [--artifact <path>] [--summary <text>]
                       [--observed-at <commit>]
```

Creates an `evidence` record. `--observed-at` defaults to HEAD; a commit not in the
repository is `AKR-G011`.

`--artifact` may not name a path under an unkept `.agent/scratch` entry (`AKR-T023`,
V-025). Scratch is
documented as disposable and `akr scratch prune` deletes from it on an ordinary handoff
(D-036), so an artefact cited from there is a verified claim whose backing some later
session removes without knowing it was cited — and nothing notices, because a path is not
a reference the ledger resolves. Move the artefact somewhere durable and cite that, or
`akr scratch keep <entry>` first and cite it where it is.

The refusal is a validation rule rather than a check on this flag, because evidence
reaches the ledger by four routes — this command, `akr evidence add-many --from`,
`akr propose --kind evidence --from`, and `knowledge.propose` over MCP — and a guard on
the flag leaves the other three open. It is still a refusal at write time, since §4's
pipeline validates the resulting ledger before it writes anything, which is the property
that matters: a warning from a later build arrives after the record is in the ledger,
which is when nobody goes back and moves the file.

The command deliberately offers **no** flag for "what this verifies". Evidence never
declares what it verifies (D-016); the link is authored on the check, with
`verified_by [ @key/n ]`, or supplied to `akr complete --check`. The absence of the flag
is the enforcement of the one-directional rule at the ergonomic level, where it matters.

---

### `akr review-queue`

```
akr review-queue [--stale-only] [--at-risk-only] [--kind <kind> ...]
                 [--format text|json]
```

Lists everything the build flagged: stale records first with their cause, then at-risk
records ordered by propagation depth, then key
([`10-freshness-and-git.md`](10-freshness-and-git.md) §7). This is the human-facing half
of the freshness model; `REVIEW-REQUIRED.md` is the committed half.

**Exit 0 regardless of queue length.** A non-empty queue is normal and healthy — it means
the project is moving and the ledger noticed. Projects that want a gate use
`akr check --review-clean`.

---

### `akr import`

```
akr import <path> [--namespace <ns>] [--tracking <key>] [--lenient] [--dry-run]
```

Reads a legacy document, proposes records for the durable claims in it, attaches a
`source { kind legacy … }` block with an excerpt to each, and creates or updates the
tracking `work` record whose acceptance checks enumerate the claims (D-022). Everything
lands in `proposed` state for review (`AKR-M042` if not).

`--lenient` is the one place warnings are downgraded, and it is per-invocation: without
it, an import that produced warnings fails with `AKR-M041` and writes nothing. A missing
source is `AKR-M001`; an unimportable format is `AKR-M002`; a colliding key is
`AKR-M012`; an undeclared namespace is `AKR-M013`.

Full workflow, boundaries, and a worked example: [`12-migration.md`](12-migration.md).

---

### `akr lock`

```
akr lock [--check] [--update]
```

`--check` recomputes what `akr.lock` should contain and reports drift without writing:
a sealed revision whose immutable content no longer matches is `AKR-R051`; a legal
lifecycle-only transition or a head resolution absent from an otherwise-current lock is
`AKR-R052`. `--update` rewrites the lock from the current build, which is also what step
11 of `akr build` does.

The lock is written in AKR syntax with an `akr-lock 0.1` header and is committed
(D-014). Its schema is [`../spec/schema/akr-lock.md`](../spec/schema/akr-lock.md).

---

## 7. CI recipes

### The gate

One command, and it is the whole story:

```yaml
- name: AKR
  run: |
    akr check --views-current
```

`akr check` fails on any diagnostic. `--views-current` additionally fails if
`docs/generated/` was hand-edited or was not rebuilt after a ledger change, which is what
gives `sys.policy.no-hand-edited-views` force rather than good intentions (D-025).

Note what is **not** in the gate: `--review-clean`. Staleness is a build fact, and a
project whose CI fails because knowledge aged will delete the knowledge rather than
review it.

### Reporting, without gating

```yaml
- name: AKR review queue
  if: always()
  run: akr review-queue --format json > akr-review.json
```

Exit 0 regardless of contents; post it as a PR comment or a job summary.

### Pre-commit

```bash
#!/bin/sh
# .git/hooks/pre-commit
akr fmt --check || exit 1
akr check       || exit 1
akr impact --git-diff HEAD..  # informational; never fails the commit
```

`akr impact` on the staged range tells the author which observations their change is
about to invalidate, at the one moment they are equipped to fix it.

### Nightly

```bash
akr check --review-clean --format json > queue.json || true
```

`review_after` dates pass with the calendar rather than with commits, so a nightly job is
the only thing that notices them promptly. Run it as a notification, not a gate.

### Release

```bash
scripts/verify-distribution.sh
```

Runs the full distribution verification: tracked files, all tests, design coherence checks,
and build parity via `akr build --check`.

---

Next: [`08-mcp.md`](08-mcp.md) for the agent-facing form of these commands,
[`09-context-assembly.md`](09-context-assembly.md) for what `akr context` computes, or
[`../examples/save-your-skin/transcripts/`](../examples/save-your-skin/transcripts/) for
real output from a real ledger.
