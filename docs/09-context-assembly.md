# 09 — Context Assembly

How AKR answers the question an agent asks at the start of every session: *what do I need
to know before I touch this?*

Normative for the assembly algorithm, section order, selection predicates, sort keys,
exclusion rules, budgeting, and the bundle format. Rules V-121–V-123 are catalogued in
§9; the codes they raise are registered in
[`../spec/diagnostics/codes-runtime.md`](../spec/diagnostics/codes-runtime.md).

---

## 1. The principle: assembly is deterministic; search only ranks

> **Membership in a context bundle is computed from the graph. Search reorders what the
> graph already authorised, and nothing else.**

This is the single most important sentence in the document, and it is what separates AKR
from a retrieval system.

A retrieval system answers "what looks relevant?" — a similarity question, answered
probabilistically, with a different answer each time the embedding model changes. A
compiler answers "what governs this?" — a graph question, answered by traversal, with
the same answer every time.

Concretely, a record enters a bundle because:

- it is the goal, or
- it is related to something already in the bundle by a specific relation named in §4,
  or
- its declared `scope` overlaps the request's paths under the test of D-010, or
- it contradicts something in the bundle.

A record never enters a bundle because its prose resembles the request. `akr search`
exists, it is useful, and it is a different command. When ranking *is* used — to order
records within a section that has no meaningful natural order — a ranking failure
degrades to lexical order with `AKR-X033` and changes nothing about which records are
present.

Three consequences worth stating explicitly:

1. **Two agents asking the same question at the same commit get the same bundle**, byte
   for byte. Divergent behaviour between two sessions is therefore always explicable.
2. **A bundle is auditable.** Every record in it can be justified by naming the step that
   selected it and the edge it came in on. `akr why-current` does the same job for a
   single record.
3. **The bundle cannot be gamed by wording.** Writing a record with more keyword overlap
   does not raise its standing. Standing comes from state, scope, and relations.

## 2. What a bundle is for

A bundle is the input to a piece of work. It answers, in order: what am I trying to
achieve, what is the current plan, what work exists under it, what rules constrain me,
what is blocking, what does "done" mean, what is known about the code I am about to
touch, what is still unknown, what is disputed, and what should not be trusted without
re-checking.

It is emphatically **not** a summary of the project. A bundle for M3 does not contain M5,
does not contain lighting policy for a track M3 does not touch, and does not contain
finished work. Everything in it is there because something connects it to the goal.

## 3. Request

```
akr context --goal <key> [--paths <glob> ...] [--budget <tokens>] [--format text|json]
```

| Parameter | Required | Meaning |
| --- | --- | --- |
| `--goal` | yes | The anchor. Bare keys, `@key`, and a pin to the current head are accepted. Historical pins (`AKR-X004`), anchors (`AKR-X005`), terminal records (`AKR-X002`), and non-planning kinds (`AKR-X003`) remain retrieval-only and return a ready-made next command. |
| `--paths` | no | Repeatable. Repo-root-relative globs, D-008 subset. Malformed is `AKR-X011`; matching nothing at HEAD is `AKR-X012` (warning). These are the files the caller expects to touch. |
| `--budget` | no | Approximate token budget. §6. Too small is `AKR-X021`. |
| `--format` | no | `text` (default) or `json`. Anything else is `AKR-X041`. |

The goal must be a planning record because a bundle is organised around *intent*. Asking
for context around a policy is a different question, answered by `akr get --relations`.

## 4. The algorithm

Eleven steps, run in order, producing eleven sections in the same order. Each step states
its **selection predicate** and its **sort key**. A record selected by an earlier step is
not repeated by a later one, except in the acceptance, questions, contradictions and
staleness sections, which are cross-cutting and reference records by key.

Questions are on that list for a reason the worked example of §8 already shows: a question
that blocks a selected work item is selected twice over, by step 6 as a blocker and by step
9 as an open question, and it belongs in both. In step 6 it is a reason to stop; in step 9
it is a thing nobody knows. Dropping either occurrence would cost the reader one of those
two facts.

Throughout: *live* means the record's state is in its class's live set
(`spec/tables/vocabulary.json`); *head* means whatever the two-tier resolution of
[`04-references-and-versioning.md`](04-references-and-versioning.md) §3 returns — the
live revision if there is one, otherwise the end of the supersession chain. A floating
`@key` always resolves; liveness is a separate question, and the exclusion rules of §5
are where this algorithm asks it.

---

**Step 1 — `goal`**

*Select:* the head revision of `--goal`.

*Sort:* single record.

Rendered in full: title, state, intent, target, and the `part_of` chain up to the root of
its containment tree.

---

**Step 2 — `milestone`**

*Select:* the transitive `part_of` ancestors of the goal that are `milestone` or `track`
records, live only. If the goal is itself a milestone or track, this section names it and
adds nothing.

*Sort:* by distance from the goal, ascending; ties by key.

This is the section that answers "what larger thing is this in service of?", and it is
what stops an agent optimising a work item at the expense of the milestone containing it.

---

**Step 3 — `work-items`**

*Select:* the union of

- the transitive `part_of` descendants of the goal, live only;
- the transitive `part_of` descendants of the goal's plan of record — **of any revision
  in its supersession chain**, not only the head;
- live `work` records whose declared `scope` overlaps any `--paths` glob (D-010).

*Sort:* by state in lifecycle order (`active`, `blocked`, `ready`, `proposed`), then by
key.

The second bullet is not a convenience. `part_of` **pins** to a plan revision
([`04-references-and-versioning.md`](04-references-and-versioning.md) §5): a child of a
superseded plan keeps pointing at the revision that owned it, and is legal precisely
because the superseding plan dispositions it. Following only the head's children would
make exactly the dispositioned items — the ones most at risk of being forgotten —
invisible to every agent that asks for context. They are included, each rendered with the
disposition that governs it:

```
`ready` @sys.work.m3-lighting-pass/1 · part of @sys.work.m3-plan/1
    dispositioned by @sys.work.m3-plan/2: carried_forward into @sys.track.lighting
```

Blocked items are rendered with the live `blocks` edges that hold them, so the reason is
adjacent to the fact. The plan of record appears here as a one-line pointer and in full
in step 4.

---

**Step 4 — `plan-of-record`**

*Select:* the head of the live `work` record whose `plan_of_record` edge targets the
goal, or the goal's nearest containing milestone or track from step 2. At most one exists
(V-018).

*Sort:* single record.

Rendered in full, including its `disposition` blocks. Those blocks are the record of what
happened to the previous plan's unfinished children, and they are the most commonly
needed and least commonly written-down fact in a replanned project (D-017).

There is no `plan` kind. A plan is a `work` record designated `plan_of_record` (D-001).

---

**Step 5 — `normative`**

*Select:* live records of the normative class (`term`, `requirement`, `policy`,
`constraint`, `decision`) whose `scope` overlaps either

- a `ref` term naming the goal or any record from step 2, under the D-010 `part_of`
  reachability rule, or
- any `--paths` glob,

plus every live normative record whose scope is `[ all ]`.

*Sort:* by kind in the order `term`, `constraint`, `policy`, `requirement`, `decision`,
then by key.

The kind order is deliberate: definitions first, then the limits the project did not
choose, then the rules it did, then what it must deliver, then what it decided. That is
the order in which the records constrain each other.

Scope overlap is the conservative test of D-010: it may include a record that turns out
not to matter, and it must never omit one that does. A bundle with one irrelevant policy
costs a few tokens; a bundle missing the policy that governs the change costs a rewrite.

---

**Step 6 — `dependencies`**

*Select:* for every record already selected, the head of every target of a `depends_on`
or `implements` edge, and the head of every source of a live `blocks` edge pointing at
it. Transitive to a depth of 3 by default. Empirical targets are **not** added here; they
are left to step 8, which knows how to order and mark them.

*Sort:* blockers first, then dependencies; within each, by depth, then by key.

Blockers are separated from dependencies because they mean different things: a dependency
is something to read, a blocker is a reason to stop.

This step routinely pulls in records that the scope test of step 5 excluded, and that is
correct. Scope answers "does this govern the code I am touching?"; the dependency graph
answers "does the thing I am touching rest on this?". A record that fails the first test
and passes the second belongs in the bundle.

---

**Step 7 — `acceptance`**

*Select:* every `check` block of every selected `milestone` and `work` record.

*Sort:* by owning record key, then by check id.

Each check is rendered with its statement, method, command if any, its `verified_by`
references, and — the point of the section — its **satisfaction verdict** and the reason
for it (D-016):

| Verdict | Rendered as |
| --- | --- |
| Satisfied | `satisfied by @key/n (pass at <commit>, descends from <last-change>)` |
| No evidence | `not satisfied — no evidence` |
| Failing evidence | `not satisfied — @key/n reports <result>` |
| Evidence too old | `not satisfied — @key/n observed at <commit>, which does not descend from <last-change>` |

The last row is the one that earns its keep. Evidence that passed before the definition
changed does not count, and an agent that cannot see *why* a check is unsatisfied will
assume the check is wrong.

---

**Step 8 — `observations`**

*Select:* live empirical records (`observation`, `evidence`, `assessment`) that any of

- have a `scope` or `watches` glob overlapping any `--paths` glob;
- have a `scope` `ref` term overlapping the goal or a step-2 record under D-010;
- are the target of a `supported_by`, `verified_by`, `derived_from` or `depends_on` edge
  from any already-selected record, **including one selected by this step** — the
  selection is run to a fixpoint.

A `verified_by` reference inside a `check` block counts as such an edge, which is how the
evidence behind a satisfied acceptance check reaches the bundle.

The fixpoint is not a refinement. In the worked example of §8,
`sys.assessment.m3-readiness/1` arrives by its `ref` scope on the goal and carries
`supported_by @sim.obs.timestep-drift/1`; the observation is reachable no other way, and it
is one half of the bundle's only contradiction. A single pass would present the
contradiction with one side missing.

*Sort:* stale records first, then by `observed_at` commit in reverse history order, then
by key.

Stale first is not a ranking judgement; it is the section's declared sort. An agent
reading top-down meets the questionable knowledge before the settled knowledge, which is
the order in which it is useful.

---

**Step 9 — `questions`**

*Select:* live `question` records (`open` or `deferred`) that either `blocks` a selected
record or are `part_of` a selected record.

*Sort:* `open` before `deferred`, then by key.

---

**Step 10 — `contradictions`**

*Select:* every `contradicts` edge with at least one endpoint among the selected records.
The relation is symmetric regardless of which side declared it (D-023).

*Sort:* by the lexicographically smaller endpoint key, then the larger.

**Contradictions are always surfaced (V-121).** Three properties, none optional:

- They appear even when one side has been superseded, withdrawn, or disproven, and the
  terminal side is named as such. Exclusion rules (§5) do not apply here.
- They appear even when `acknowledged true`. An acknowledged contradiction is one
  somebody decided to live with, not one that stopped existing.
- They are never dropped by budgeting (§6) and never suppressed by ranking.

The whole value of the `contradicts` relation is that a disagreement somebody noticed is
never quietly lost. A bundle that hides it under a token budget would defeat the point.

---

**Step 11 — `staleness`**

*Select:* every selected record flagged `stale` or `at_risk` by stage D
([`10-freshness-and-git.md`](10-freshness-and-git.md) §3).

*Sort:* stale first, then at-risk by propagation depth ascending, then by key.

Each entry names the cause: for stale, the watch glob and the commit that matched it, or
the `review_after` date that passed; for at-risk, the propagation path.

**These are warnings, not diagnostics** (D-024). They tell the reader which parts of the
bundle to re-check before relying on them. They never say a record is false, and they
never change an exit status.

## 5. Exclusion rules

A record is excluded from an ordinary bundle when any of these holds. The exclusion is
checked after selection and is enforced as V-122 (`AKR-X052`).

| Excluded | Reason |
| --- | --- |
| Any non-head revision | Only heads are current. Pinned references in a bundled record still *display* their pinned revision; the record itself is not added. |
| Any record in a terminal state | `superseded`, `rejected`, `withdrawn`, `abandoned`, `completed`, `closed-without-resolution`, `disproven`. |
| Any record under `.akr/archive/` | D-018. Archived records still resolve; they do not enter bundles. |
| Any record whose scope does not overlap the request | The conservative D-010 test decides. |

**The one exception is step 10.** A contradiction is surfaced even when one side is
terminal or archived, and the terminal side is labelled. Nothing else overrides these
rules — not ranking, not budgeting, not a `--paths` match.

The bundle reports the exclusions in aggregate, by reason, so that a reader can tell
"there is nothing else" from "there is more and it was filtered".

## 6. Budgeting

`--budget` is an approximate token ceiling on the **delivered** bundle: the rendered text
and the JSON half together, which is what a caller pays for, rather than the records the
selection chose, which is a much smaller number and used to be the only thing measured. The
command renders, measures both halves, and re-assembles to proportionally less when the
result overshoots — three passes, because the ratio is stable enough that the second lands.
The result reports `budget.requested_tokens` and `budget.delivered_tokens`, and says so
when the two cannot be reconciled, which happens when the mandatory sections alone cost
more than the request (§6, V-123): that is not a failure, it is the ceiling this document
puts under every bundle, and the fix is a narrower `--paths`.

When the assembled bundle exceeds it, content
is reduced in a fixed order, and the order encodes what the design considers expendable:

```
   1. prose bodies of step 8 observations      truncate to 2 sentences
   2. prose bodies of step 5 normative records truncate to 2 sentences
   3. rationale / context / consequences slots drop entirely
   4. claim texts on non-goal records          truncate to 1 sentence
   5. step 6 dependency depth                  reduce 3 -> 2 -> 1
   6. step 8 observations beyond the 10 most   drop, and say how many
      recent
```

**What never truncates (V-123):**

- Any relation. Every edge shown is shown in full, with its target key and revision. A
  truncated relation set is a lie about the graph, and the graph is the part a reader
  cannot reconstruct from anywhere else.
- The goal, the plan of record, and their `disposition` blocks.
- Any acceptance check, or any check's verdict.
- Any contradiction.
- Any staleness or at-risk warning.
- Any state, scope, key or revision anywhere in the bundle.

If the mandatory content alone exceeds the budget, the command fails with `AKR-X021`
rather than silently dropping something load-bearing. Truncation that did occur is
reported as `AKR-X022` (a warning) and listed in `truncated_prose`, so a consumer knows
exactly what it is missing and can fetch it with `knowledge.get`.

Truncation points are deterministic — sentence boundaries computed by a fixed rule, not
by a model. Summarising a record to fit is a job for the *consumer* of the bundle, above
the LLM boundary (D-020).

## 7. Bundle format

Text form. The JSON form carries the same sections, in the same order, with the schema of
[`08-mcp.md`](08-mcp.md) §3.

```
AKR CONTEXT BUNDLE
goal        <key>/<rev> — <title>
commit      <40 hex>
paths       <glob> [, <glob> ...]
generated   akr <version>, source-graph sha256:<hash>

── 1. GOAL ──────────────────────────────────────────────────────────
...
── 11. STALENESS ────────────────────────────────────────────────────
...

── EXCLUDED ─────────────────────────────────────────────────────────
  superseded revisions   n
  archived               n
  terminal               n
  out of scope           n
```

Every section header is printed even when the section is empty, and an empty section
prints `(none)`. A missing section header would be ambiguous between "nothing here" and
"not computed"; the whole point of a deterministic bundle is that the reader can tell.

## 8. Worked example

Against [`../examples/save-your-skin/`](../examples/save-your-skin/), at HEAD = C5
(`e806b3f5…`), today 2026-08-03. The full transcript is
[`../examples/save-your-skin/transcripts/akr-context.txt`](../examples/save-your-skin/transcripts/akr-context.txt);
the shape is summarised here.

```
$ akr context --goal sys.milestone.m3-playable-day --paths "sim/src/project/**"
```

| Section | Selected | Why |
| --- | --- | --- |
| 1 goal | `sys.milestone.m3-playable-day/1` | the anchor |
| 2 milestone | *(the goal itself)* | goal is a milestone |
| 3 work-items | `lege.work.extract-render-graph/1`, `sim.work.rewrite-projection/1`, `sys.work.m3-audio-pass/1`, `sys.work.m3-lighting-pass/1` | `part_of` the goal, or `part_of` a revision of its plan |
| 4 plan-of-record | `sys.work.m3-plan/2` | `plan_of_record` → the goal |
| 5 normative | `sys.term.playable-day/1`, `sys.term.tandem-work/1`, `sys.constraint.single-threaded-sim/1`, `sys.policy.tandem-work/1`, `sys.req.deterministic-sim/1` | scope `[ all ]`, or `path "sim/**"`, whose prefix `sim/` is comparable with `sim/src/project/` |
| 6 dependencies | blocker `sim.question.timestep-vs-budget/1`; then `lege.decision.renderer-boundary/2`, `lege.req.no-engine-types-in-viewer/1`, `lege.term.renderer-boundary/1`, `sys.constraint.frame-budget-16ms/1` | live `blocks`; `depends_on` and `implements` to depth 3 |
| 7 acceptance | M3 `full-day-demo` **satisfied**, M3 `no-placeholder-assets` **not satisfied** | §4 step 7 |
| 8 observations | `sim.obs.projection-gaps/1` (stale), `sim.obs.timestep-drift/1` (stale), `sys.assessment.m3-readiness/1`, `sys.assessment.projection-gaps/1`, `lege.evidence.boundary-lint-pass/1`, `sim.evidence.determinism-suite-pass/1`, `sys.evidence.playable-day-demo/1` | path overlap, `ref` scope on the goal, or `supported_by`/`verified_by`/`depends_on` from a selected record |
| 9 questions | `sim.question.timestep-vs-budget/1` (open) | `blocks` a selected work item |
| 10 contradictions | `sim.obs.timestep-drift/1` ↔ `sim.evidence.determinism-suite-pass/1`, acknowledged | declared, symmetric |
| 11 staleness | 2 stale, 4 at risk | stage D |

Twenty-three records. The three exclusions the frozen manifest calls out explicitly:

- **`sys.work.m3-plan/1`** — not the head. Its content is superseded by revision 2, whose
  `disposition` blocks say what happened to its children. Including it would present a
  retired plan alongside the live one. Note that its *children* are present: they still
  carry `part_of [ @sys.work.m3-plan/1 ]`, and step 3 follows the chain precisely so that
  a dispositioned item cannot vanish from view.
- **`lege.decision.renderer-boundary/1`** — superseded, so not the head. Revision 2 **is**
  present, arriving in step 6 as a `depends_on` target of
  `lege.work.extract-render-graph/1`, even though its scope `path "lege/**"` does not
  overlap `sim/src/project/**`.
- **`sys.policy.weekly-demo/1`** — archived and `withdrawn`. It still resolves, so a
  historical reference to it works; it never enters a bundle.

Two records are worth naming because they show the two selectors disagreeing, which is
the mechanism working rather than failing:

- **`sys.constraint.frame-budget-16ms/1`** fails step 5. Its scope is
  `path "lege/**", path "sim/src/step.rs"`, and neither prefix is comparable with
  `sim/src/project/`. It arrives anyway in step 6, at depth 2, because the open question
  blocking the work item depends on it. Scope asks "does this govern the code I am
  touching?"; the graph asks "does what I am touching rest on this?". Both answers belong
  in the bundle.
- **`sim.req.fixed-timestep/1`** fails both. Its scope is `path "sim/src/step.rs",
  path "sim/src/tick/**"` — adjacent to the request but not overlapping — and nothing
  selected depends on it or implements it. It is absent, correctly: rewriting the
  projection pass does not touch the timestep.

The exclusion summary for this request:

```
── EXCLUDED ─────────────────────────────────────────────────────────
  superseded revisions   2
  archived               1
  terminal               4
  out of scope          12
```

## 9. Rules

| Rule | Statement | Code |
| --- | --- | --- |
| **V-121** | Every `contradicts` edge with an endpoint among the selected records appears in the contradictions section, including when an endpoint is terminal or archived, and including when `acknowledged true`. | `AKR-X051` |
| **V-122** | No non-head revision, terminal record, or archived record appears in a bundle outside the contradictions section. | `AKR-X052` |
| **V-123** | Budgeting truncates prose only. Relations, states, scopes, keys, revisions, acceptance verdicts, contradictions and staleness warnings are never reduced; if they alone exceed the budget, the request fails. | `AKR-X021`, `AKR-X022` |

V-121 and V-122 are internal invariants: they fire only on a defect in the assembler, not
on anything a ledger author can write. They are stated as rules, and checked, because
they are the two properties every consumer of a bundle relies on without being able to
verify.

Beyond the rules, one invariant with no code, because it is checked by construction
rather than at run time: **a bundle is a pure function of (ledger, commit, request)**.
No wall clock beyond `--today`, no environment, no network, no model. It follows from the
determinism contract of [`01-architecture.md`](01-architecture.md) §4 and is what makes
bundles cacheable for a session and comparable between sessions.

## 10. Search design

Search is a separate command with a separate contract.

**Now: FTS5.** `records_fts` ([`../spec/schema/index.sql`](../spec/schema/index.sql))
indexes `title`, the kind's required prose body, concatenated claim text, and term
aliases, over live revisions only. Ranking is BM25 with key as the tiebreak, so results
are stable. Aliases are indexed so that searching a synonym finds the `term` record that
fixes the project's meaning for it — which is usually the record the searcher actually
needed.

**Later: embeddings.** A vector index may be added as an alternative ranker. It changes
`ORDER BY` and nothing else. The contract it must satisfy:

1. **Ranking only.** No record's membership in any bundle, and no record's authority,
   depends on any ranker. A ranker that is unavailable degrades to lexical order with
   `AKR-X033` and the tool keeps working.
2. **Outside the build.** Embedding generation happens above the LLM boundary (D-020),
   never in stages A–F. A build must not require a model to be reachable.
3. **Cache only.** Vectors live beside the SQLite index, are gitignored, are rebuildable,
   and are never authoritative (D-019).
4. **Reproducible or absent.** If a ranker cannot produce the same order twice for the
   same inputs, its output is presentation only and must not be relied on by any other
   command.

**What search will never do:** authorise. There is no configuration in which a record
enters a context bundle because it scored highly, and there is no ranking signal that
raises a record's authority. Standing comes from state, scope, and relations, which are
declared, checked, and reviewable in a diff.

---

Next: [`10-freshness-and-git.md`](10-freshness-and-git.md) for where the staleness flags
in step 11 come from, or [`08-mcp.md`](08-mcp.md) §9 for an agent using a bundle.
