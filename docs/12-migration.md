# 12 — Migration

Getting an existing Markdown pile into the ledger without losing anything and without
pretending the pile was already structured.

Normative for the import workflow, the tracking-record pattern, `--lenient` semantics,
the LLM boundary during import, and the `AKR-M` codes.

---

## 1. The shape of the problem

A project adopting AKR has a `docs/` directory holding some mixture of: a roadmap that is
partly true, three architecture notes of which one is current, a decisions file nobody
updated, retrospectives, and a dozen files whose status nobody can state. Between five
and fifty per cent of it is durable knowledge. The rest is status chatter, superseded
plans, and notes to a person who has left.

Two failure modes bracket the space, and the workflow exists to avoid both:

- **Import everything.** The ledger inherits the pile's problems with added ceremony, and
  the first `akr check` produces four hundred diagnostics nobody triages.
- **Import nothing.** Somebody rewrites from memory, and every claim whose provenance
  mattered loses it. Six months later nobody can say why a rule exists.

The middle path is *dispositioning*: every durable claim in a legacy document is either
promoted to a record or explicitly declined, and the document is archived only when every
claim has an answer. That is the same discipline D-017 applies to unfinished children at
supersession, applied to sentences instead of work items — and for the same reason.

## 2. Migration adds no kinds (D-022)

There is no `legacy-source` kind, and there will not be one. Migration is a workflow, not
a category of knowledge, and a thirteenth kind would outlive the migration it was created
for.

Two existing mechanisms carry the whole thing:

**Provenance is a `source` block**, repeatable on any record:

```
source {
    kind legacy
    path "docs/legacy/WORKING-AGREEMENTS.md"
    document "WORKING-AGREEMENTS-a1b2c3d4"
    excerpt """
        Engine and sim should stay in step. (No exceptions were listed.)
        """
}
```

`kind` is `legacy`, `external`, or `internal`. The `excerpt` is what makes the import
auditable rather than a rewrite: a reviewer can see the sentence the record claims to be
a structured form of, without opening the legacy file. The `path` is where the document
was read from; the `document` id is the word-for-word copy `akr import` saved into
`sources/` automatically, so the excerpt stays verifiable after the original moves.

**Progress is a `work` record.** One tracking record per legacy document, whose
acceptance checks enumerate the disposition of its durable claims. Migration then shows
up in `ACTIVE-WORK.md` like any other work, is blocked like any other work, and completes
under V-020 like any other work.

## 3. The workflow

Six steps. Steps 1, 4 and 5 are human judgement; the rest are mechanical.

```
   1. INVENTORY  ──►  2. EXTRACT  ──►  3. DRAFT  ──►  4. REVIEW
                                                          │
                          6. ARCHIVE  ◄──  5. ACCEPT  ◄────┘
```

### Step 1 — Inventory

List every legacy document and give each a verdict before reading it closely:

| Verdict | Meaning | Next |
| --- | --- | --- |
| `migrate` | Contains durable claims | Steps 2–6 |
| `reference` | Useful but not normative — a tutorial, a diagram | Leave in place, cite with `source { kind external }` where needed |
| `drop` | Status chatter, superseded, or obsolete | Record the decision to drop it, then delete or archive |

`drop` needs a written decision somewhere. "We looked at it and decided it held nothing"
is a fact worth keeping; "it vanished" is not.

Create the tracking `work` record for every `migrate` document at this step, before any
extraction, so the inventory itself is in the ledger.

### Step 2 — Extract durable claims

A claim is **durable** when it will still be worth knowing in six months. The test that
works in practice is the one from `00-overview.md` §1: can you say what *kind* of record
it is?

| Legacy sentence | Durable? | Kind |
| --- | --- | --- |
| "The simulator must produce the same run from the same seed." | yes | `requirement` |
| "We decided to put a snapshot between the sim and the viewer." | yes | `decision` |
| "Lighting is ongoing; no milestone owns it." | yes | `track` |
| "M3 is about 60% done." | no | — status |
| "Ask Dana about the audio pipeline." | no | — a note to a person |
| "TODO: fix the projection pass." | maybe | `work`, if still intended |

An LLM is genuinely useful here, and this is exactly where D-020 permits it: reading
prose and proposing a structure is drafting, and everything it proposes is reviewed.
§6 draws the line precisely.

### Step 3 — Draft records

`akr import` first saves the document itself into `sources/` word for word,
content-hashed and reused by hash on re-import, then writes one `proposed`
record per extracted claim, each with a `source { kind legacy … }` block
carrying the path, the saved copy's `document` id, and the excerpt.

Everything lands in `proposed` state (`AKR-M042` if not), which matters for two reasons:
`proposed` revisions are not sealed and may be edited freely during review (D-015), and a
`proposed` record cannot be mistaken for something the project has actually agreed to.

One exception is forced by the model, and it is benign: the inquiry class has no
`proposed` state, so an imported `question` lands `open` — its only initial state. An
open question asserts nothing normative, and §8 already says why importing one open is
right: it is the single most valuable thing in a legacy pile *because* it is open.

Keys are proposed, never invented silently: the namespace comes from `--namespace` or
from the document's location, and a key whose namespace is not declared in `project.akr`
is `AKR-M013`. A key that already exists is `AKR-M012` — import only ever adds new keys.
Revising an existing record from legacy material is a deliberate `akr revise`.

### Step 4 — Human review

The reviewer reads the drafted records against the excerpts and decides, per record:

- **Accept** — the record says what the document said, and the project still means it.
- **Edit then accept** — the claim is right, the drafting is not.
- **Reject** — the claim was real but the project no longer holds it. Record it as
  `rejected` (normative) or `withdrawn`, *with* its source block, so the history survives.
- **Split** — one legacy paragraph is two claims.
- **Decline** — not durable after all. Delete the draft and mark the corresponding
  acceptance check satisfied by an evidence record recording the judgement.

This step is not optional and is not automatable. It is where the project decides what it
still believes.

### Step 5 — Accept and mark mapped

Accepting moves a record from `proposed` to its class's live state (`akr revise --state
active`, or `resolve` for a question). The corresponding acceptance check on the tracking
record is then satisfied by an evidence record naming what happened.

### Step 6 — Archive when fully dispositioned

When every check on the tracking record is satisfied, `akr complete` moves it to
`completed` — which V-020 permits only if every check really is satisfied — and the legacy
document is moved to `docs/legacy/archive/` or deleted.

Archiving a document whose tracking record is not `completed` is `AKR-M032`. A migrated
document — one a tracking record still points at — that no longer exists at HEAD is
`AKR-M022`, a warning: the excerpt is now unverifiable, which a reader of the record
should know. Both checks are *anchored on the tracking record*; a bare `source { kind
legacy }` citation with no tracker is §2 provenance, not a migration, and the audit
leaves it alone (see §4).

At import time, a link inside the imported document that resolves to a live file with
no saved copy in `sources/` is `AKR-M023`, also a warning: a plain path with no
immutable copy behind it is tomorrow's dead link, so register it with `akr source add`
while the bytes are still there.

## 4. The tracking record pattern

One `work` record per migrated document. Its acceptance checks are the disposition list.

Shape, following the constructs of [`../spec/exemplar.akr`](../spec/exemplar.akr):

- `title` — "Import the legacy roadmap"
- `intent` — what the document is and why it is being migrated
- `source { kind legacy path "…" document "…" }` — the document itself, with its saved copy's id
- `acceptance` with one `check` per durable claim found, each with a `statement` naming
  the claim, `method manual`, and `verified_by` filled in as claims are dispositioned

Why acceptance checks rather than a checklist in prose:

- **They are counted.** `ACTIVE-WORK.md` and `ROADMAP.md` show "2 of 3 satisfied".
- **They cannot be closed early.** V-020 refuses `akr complete` while any check is
  unsatisfied (`AKR-R022`), so the document cannot be archived with claims unaccounted
  for.
- **They carry evidence.** Each disposition names the record that resulted, or the
  evidence recording the decision not to migrate.
- **They are reviewable in a diff**, one line per claim, at the moment the claim is
  dispositioned.

A tracking record is required for a *migration*: `akr import` always writes one, so a set
of imported records with no tracker for their document is `AKR-M031` at the moment of
import, and an imported record with no `source { kind legacy }` block at all is
`AKR-M021`. This is an import-time guarantee, not a standing check over the ledger: once
records are in force, a `source { kind legacy }` block is ordinary provenance (§2),
repeatable on any record, and a mature ledger cites the documents its knowledge came from
without a tracker for each — the deliberate steady state of
[`../examples/sys-tandem/MANIFEST.md`](../examples/sys-tandem/MANIFEST.md) §8. The
check-time audit therefore cannot tell an unfinished import from a permanent citation, so
it does not re-derive `AKR-M031`; it reasons only about documents a tracking record still
claims (the `AKR-M022` and `AKR-M032` of §3).

## 5. `akr import`

```
akr import <path> [--namespace <ns>] [--tracking <key>] [--lenient] [--dry-run]
```

| Flag | Effect |
| --- | --- |
| `--namespace` | Namespace for proposed keys. Undeclared is `AKR-M013`. |
| `--tracking` | The tracking `work` record. Created if absent. |
| `--lenient` | Downgrade warnings. §5.1. |
| `--dry-run` | Print what would be written and write nothing. The recommended first invocation. |

A missing source is `AKR-M001`; a format outside Markdown and plain text is `AKR-M002`;
a document from which nothing durable was extracted is `AKR-M011`, a warning — the tool
does not archive a document it read nothing from, and a human decides whether that is
right.

### 5.1 `--lenient` semantics

`--lenient` is the **only** place in AKR where warnings are downgraded (D-013), and it
exists for exactly this command.

The reason is specific. Legacy material reliably produces warnings that are true of the
source and not faults of the importer: a `watches` glob pointing at code that has since
moved (`AKR-G022`), a `review_after` date already in the past (`AKR-G031`), a legacy path
that no longer exists (`AKR-M022`). Under the default strict profile every one of those
is an error and no import can complete. Requiring them to be fixed *before* the material
is in the ledger inverts the workflow: the whole point of importing is to get the claims
somewhere they can be reviewed.

Three limits keep the escape hatch from becoming a habit:

1. **Per-invocation only.** There is no configuration setting for it, and no way to make
   a project lenient. `akr check` on the resulting ledger is strict like everywhere else.
2. **Warnings are still reported and still counted.** `--lenient` changes the exit
   status, not the output. Without it, an import that produced warnings fails with
   `AKR-M041` and writes nothing.
3. **Everything lands `proposed`.** Nothing imported has authority until a human accepts
   it, so a downgraded warning cannot quietly become a governing rule.

The workflow that follows from this: `akr import --dry-run`, read the warnings,
`akr import --lenient`, fix the warnings during review, and `akr check` strictly before
accepting anything.

## 6. LLM boundaries during import

Import is the most model-assisted operation in the toolchain and is still entirely on the
permitted side of D-020, because everything a model does here produces a *proposal*.

**A model may:**

- Read a legacy document and propose which paragraphs contain durable claims.
- Propose a kind, a title, a key, and a first draft of the prose slots.
- Propose which existing records a draft relates to, and by which relation.
- Propose the acceptance-check list for the tracking record.
- Summarise a long document for a reviewer.

**A model may not:**

- Decide that a claim is durable. That is the reviewer's decision in step 4.
- Accept a record. Nothing moves out of `proposed` without a human write.
- Determine authority, scope overlap, head resolution, staleness, acceptance, or
  supersession — none of which are import-specific; they are the same prohibitions that
  hold everywhere (D-020).
- Run inside `akr import` in a way that makes the command's output non-deterministic
  without saying so. Model-assisted extraction runs *before* the write pipeline and its
  output is an ordinary input to it.
- Write the `excerpt`. Excerpts are copied verbatim from the source, because their whole
  function is to let a reviewer check the drafting against the original. A paraphrased
  excerpt would defeat the audit.

The structural safety property: every model-produced artefact passes through the same
parse → validate → format → write pipeline as a hand-written one
([`07-cli.md`](07-cli.md) §4), lands in `proposed`, and is read by a human before it has
any standing.

## 7. Worked example: the legacy roadmap

From [`../examples/save-your-skin/MANIFEST.md`](../examples/save-your-skin/MANIFEST.md).
The project has one pre-AKR document, `docs/legacy/ROADMAP.md`, and the tracking record
`sys.work.legacy-roadmap-import` (state `proposed`, three acceptance checks).

**The document** contains, among status chatter, three durable claims:

| # | Claim | Check id |
| --- | --- | --- |
| 1 | M3 is "one playable day", scoped to the day loop and no further | `m3-scope-claim` |
| 2 | Lighting is standing work that no milestone owns | `lighting-standing-claim` |
| 3 | The team builds a demo every Friday | `weekly-demo-claim` |

**Step 1 — inventory.** Verdict `migrate`. Tracking record created:

```
$ akr propose sys.work.legacy-roadmap-import --kind work \
      --title "Import the legacy roadmap"
created sys.work.legacy-roadmap-import/1 (proposed) in .akr/records/sys/work.akr
```

**Step 2–3 — extract and draft.**

```
$ akr import docs/legacy/ROADMAP.md --namespace sys \
      --tracking sys.work.legacy-roadmap-import --dry-run
docs/legacy/ROADMAP.md — 3 durable claims, 11 paragraphs skipped

  would propose  sys.milestone.m3-playable-day   milestone   (claim 1)
  would propose  sys.track.lighting              track       (claim 2)
  would propose  sys.policy.weekly-demo          policy      (claim 3)
  would add      3 checks to @sys.work.legacy-roadmap-import

warning[AKR-M022]: source path "docs/legacy/PLAN-v1.md" does not exist at e806b3f5
  --> docs/legacy/ROADMAP.md:4:1
nothing written (--dry-run)
```

**Step 4 — review.** The reviewer reaches three different verdicts, and that is the point
of the example:

1. **Claim 1 — accept, as a revision of existing knowledge.** `sys.milestone.m3-playable-day`
   already exists in the ledger, so the import would collide (`AKR-M012`). The reviewer
   instead adds the `source { kind legacy }` block to the existing record with
   `akr revise`, recording where the milestone's scope originally came from.
2. **Claim 2 — accept as drafted.** `sys.track.lighting` is exactly right, and the
   legacy sentence is the reason the track exists.
3. **Claim 3 — accept, then immediately withdraw.** The weekly demo really was the
   practice, and really has been abandoned. Importing it as `withdrawn` with its source
   block and then archiving it preserves the history; declining to import it would lose
   the fact that the project once worked that way and stopped. This is why
   `sys.policy.weekly-demo` sits in `.akr/archive/sys/policies-archived.akr` in state
   `withdrawn`, still resolving, appearing in `DECISION-HISTORY.md` and nowhere else.

**Step 5 — accept and mark mapped.** Each check gains a `verified_by` reference to an
evidence record recording the disposition:

```
$ akr evidence add sys.evidence.legacy-claim-2-mapped \
      --result pass --method manual \
      --summary "Claim 2 mapped to @sys.track.lighting/1."
created sys.evidence.legacy-claim-2-mapped/1 (verified)
```

**Step 6 — archive.** When all three checks are satisfied:

```
$ akr complete sys.work.legacy-roadmap-import
sys.work.legacy-roadmap-import/1 -> completed (3 of 3 checks satisfied)
$ git mv docs/legacy/ROADMAP.md docs/legacy/archive/ROADMAP.md
```

Until then, `akr complete` refuses with `AKR-R022` naming the unsatisfied checks, and
archiving the document anyway is `AKR-M032`. In the frozen example inventory the tracking
record is still `proposed` with none of its three checks verified, which is what a
migration in progress looks like.

## 8. Migrating a whole project

Ordering advice, learned from the shape of the dependency graph rather than from
sentiment:

1. **Terms first.** Vocabulary drift is the cheapest thing to fix and the most expensive
   to leave, and every later record refers to terms.
2. **Constraints and policies next.** They are the things future work must respect, and
   they are usually the most durable content in a legacy pile.
3. **Milestones and tracks, then work.** Planning structure before planning content, so
   `part_of` has somewhere to point.
4. **Decisions.** By now the requirements and policies a decision `implements` exist.
5. **Observations and evidence last, and sparingly.** A legacy observation with no
   recoverable `observed_at` commit is nearly worthless — it fails V-101 or is pinned to
   a commit nobody can justify. Prefer re-observing at HEAD to importing.
6. **Questions whenever you find them.** An open question in a legacy document is the
   single most valuable thing in it, because it is the one piece of information nobody
   reconstructs later.

Two rules of thumb worth stating: **do not import status**, ever — states and acceptance
checks carry it, and a status sentence is stale on arrival. And **stop when the marginal
claim stops being worth a review**; a partly migrated pile with a complete disposition
record is a better outcome than a fully migrated pile nobody read.

## 9. Codes

The full registry is
[`../spec/diagnostics/codes-runtime.md`](../spec/diagnostics/codes-runtime.md).

| Code | Fault |
| --- | --- |
| `AKR-M001` | Import source not found |
| `AKR-M002` | Unsupported import source format |
| `AKR-M011` | No durable claim extracted (warning) |
| `AKR-M012` | Imported key collides with an existing key |
| `AKR-M013` | Imported key's namespace is not declared |
| `AKR-M021` | Imported record lacks a `source { kind legacy }` block |
| `AKR-M022` | Legacy source path does not exist at HEAD (warning) |
| `AKR-M023` | Linked path has no saved copy in `sources/` (warning) |
| `AKR-M031` | No tracking work record for an imported document |
| `AKR-M032` | Legacy document archived while its tracking record is incomplete |
| `AKR-M041` | Import produced warnings under the strict profile |
| `AKR-M042` | Imported record is not in `proposed` state |

Migration raises no rule identifiers of its own. The invariants it relies on — V-020 for
completion, V-101 for `observed_at`, V-003 for file placement — are the ordinary ones.
That is the design working: migration is a workflow over the existing model, not a mode
the model has to know about.

---

Next: [`13-implementation-roadmap.md`](13-implementation-roadmap.md) §3 phase P8 for when
the import tooling is built, or [`07-cli.md`](07-cli.md) for `akr import`'s flags.
