# AKR Data Model

This document defines what a record is, what kinds exist, what states they move
through, how they relate, and what the compiler does with each of those facts. It is
the normative source for meaning. `spec/tables/vocabulary.json` is the normative source
for names, and the two are checked against each other.

Everything here follows from `docs/DECISIONS.md`; where a decision is load-bearing it is
cited inline as (D-nnn).

---

## 1. Record anatomy

A record is the unit of knowledge. Not a document, not a section, not a paragraph — a
record. It is small enough that one person can agree or disagree with it in one sitting,
and it is addressed by a name that does not change when the file it lives in does.

```
record sys.policy.tandem-work/2 : policy {
    title "Engine and simulator advance in tandem"
    state active
    scope [ ref @sys.milestone.m3-playable-day, path "sim/**" ]
    topic tandem-work
    rule """
        No engine change lands without the matching simulator change in the same
        commit, except on the tracks listed under exceptions.
        """
    exceptions [ @sys.track.lighting ]
    supported_by [ @sys.assessment.projection-gaps ]
    author "dkoepke"
    created_at 2026-03-04
}
```

Six things are always present, in this order:

| Part | Example | Meaning |
| --- | --- | --- |
| **Key** | `sys.policy.tandem-work` | Stable identity. Never changes, never reused, never derived from a filename (D-018). |
| **Revision** | `/2` | Which version of that key this is. Monotonic from 1. |
| **Kind** | `: policy` | What sort of claim this is. One of thirteen (D-001, D-027). |
| **Title** | `title "..."` | One-line human label. Required on every kind, because every generated view needs a heading and deriving one from prose is not deterministic. |
| **State** | `state active` | Where in its lifecycle this revision sits. Drawn from the kind's class (D-002). |
| **Body** | the remaining slots | Content slots, blocks, relations, and metadata. |

The pair (key, revision) is a **revision identifier** and is what every reference
ultimately resolves to. A key on its own is a **logical record**: the whole history of
one idea. "The current head of `sys.policy.tandem-work`" is a question with exactly one
answer at any commit, and computing it is the resolver's job (`docs/04`).

### 1.1 Slots and blocks

The body is made of slots and blocks.

A **slot** is `name value`. Slots are unique within their record or block; writing one
twice is `AKR-P031` (D-012). Multi-valued content uses an array with a plural name —
`aliases`, `watches`, `exceptions` — and relation slots keep the relation name verbatim
and are always arrays even when they hold one element.

A **block** is `name [head] { ... }`. Five blocks exist, and only these five:

| Block | Head | Repeatable | Appears in |
| --- | --- | --- | --- |
| `claim` | anchor identifier | yes | any kind |
| `acceptance` | none | no | `milestone`, `work` |
| `check` | check identifier | yes | `acceptance` |
| `source` | none | yes | any kind |
| `disposition` | a reference | yes | `work`, `milestone`, `track` |

There is no general nesting. A block contains slots and, in the single case of
`acceptance`, `check` blocks. You cannot invent a block, and there is no escape hatch
for arbitrary metadata — see §12.

### 1.2 What a record is not

A record is not a container for a document. If you find yourself writing eight
paragraphs into one `statement`, you have several records. The compiler will not stop
you, but nothing downstream can help you either: staleness, supersession, scope, and
acceptance all operate at record granularity, so a record that says four things can only
be superseded, scoped, or invalidated as a unit.

The rough test: a record should be something a reviewer can accept or reject with one
decision.

---

## 2. The four classes

Twelve kinds would mean twelve lifecycles, twelve rule sets, and twelve places to make
the same mistake. Instead every kind belongs to exactly one class, and the class carries
the rules (D-002).

| Class | Kinds | Says | Lifecycle | Scope required | `topic` allowed |
| --- | --- | --- | --- | --- | --- |
| **normative** | `term`, `requirement`, `policy`, `constraint`, `decision` | what *ought* to be true | proposed → active → superseded | yes | yes |
| **empirical** | `observation`, `evidence`, `assessment` | what *was found* to be true, at a stated commit | verified → disproven / superseded | no | no |
| **planning** | `milestone`, `work`, `track` | what is *intended*, in what order, and when it is done | proposed → ready → active → completed | no | no |
| **inquiry** | `question` | what is *not yet known* | open → resolved | no | no |

The class also determines:

- **Staleness.** Only empirical records go stale, because only they claim something
  about a particular commit (D-024). A policy does not become false when the code
  changes; an observation might.
- **Context ordering.** Assembly walks planning → normative → empirical → inquiry, which
  is the order a reader needs them in (`docs/09`).
- **Which relations may start or end at the kind.** See §7.

The classes are not extensible in 0.1. If a thirteenth kind is ever justified, it joins
one of these four; a fifth class would mean the model was wrong.

---

## 3. The thirteen kinds

Eight kinds are written. Five are **retired** (D-046): they parse, validate, hash and
render exactly as they always did, so a ledger's history reads as it was written, but no
new key may take one (`AKR-T008`). §3.2 says where each went.

| Kind | Class | Purpose in one line | Required content | Distinctive slots |
| --- | --- | --- | --- | --- |
| `policy` | normative | A standing rule, requirement or limit the project must respect | `rule` | `exceptions`, `rationale`, `measure`, `aliases` |
| `decision` | normative | A choice made between alternatives, and what follows | `decision` | `context`, `consequences` |
| `observation` | empirical | What was found true of the system at a specific commit, or a judgement drawn from such findings | `statement`, `observed_at` | `watches`, `review_after`, `method`, `confidence` |
| `evidence` | empirical | The outcome of a check that was actually run | `result`, `method`, `observed_at` | `command`, `artifact`, `summary` |
| `papercut` | empirical | A small friction hit while working, logged in the moment (D-027) | `statement`, `observed_at` | — |
| `milestone` | planning | A named point at which defined checks pass | `intent`, `acceptance` | `target`, `note` |
| `work` | planning | A unit of intended change | `intent` | `acceptance`, `disposition`, `target`, `note`, `trace` |
| `question` | inquiry | An open matter that blocks or endangers something | `question` | `resolution` |
| `term` *(retired)* | normative | Fixes the project's meaning for a word that would otherwise drift | `definition` | `aliases` |
| `requirement` *(retired)* | normative | Something the delivered system must do or be | `statement` | `rationale` |
| `constraint` *(retired)* | normative | A limit the project must respect but did not choose | `statement` | `measure` |
| `assessment` *(retired)* | empirical | A judgement drawn from observations | `statement` | `confidence`, `as_of` |
| `track` *(retired)* | planning | Standing work no milestone contains | `intent` | `cadence`, `note` |

Every kind also accepts the common slots: `title`, `state`, `scope`, `retired_claims`,
`acknowledged`, `author`, `created_at`, any relation slot permitted by §7, and any number
of `claim` and `source` blocks.

### 3.1 The three distinctions people got wrong

These are the distinctions the original vocabulary asked authors to make. Three months
of real use showed that they were not made reliably (`docs/19`), which is why D-046
retired the kinds on the far side of each. They are kept here because sealed records
still carry those kinds and a reader needs to know what was meant.

**`requirement` versus `policy`.** A requirement is about the artifact; a policy is
about the work. "The simulator must be reproducible from a seed" is a requirement — it
describes the shipped thing. "Engine and simulator changes land in the same commit" is a
policy — it describes how the team behaves. If the project shipped tomorrow and the team
disbanded, requirements would still be checkable and policies would be meaningless.

**`constraint` versus `requirement`.** A constraint is imposed; a requirement is chosen.
"16 ms frame budget because we target 60 Hz on this hardware" is a constraint — the
hardware decided. "The viewer must not import engine types" is a requirement — someone
chose it and could unchoose it. The distinction matters because a decision that violates
a constraint is a different conversation than one that violates a requirement.

**`observation` versus `evidence` versus `assessment`.** An observation says what is
true of the code. Evidence says what happened when a check was run. An assessment says
what someone concluded from those. They are separate kinds because they go stale
differently and because collapsing them is how "the tests passed" quietly becomes "the
system works".

### 3.2 Retired kinds and where they went

| Retired | Write this instead | What carried over |
| --- | --- | --- |
| `assessment` | `observation` | `confidence`; an observation may now declare `supported_by` |
| `requirement` | `policy` | may declare `part_of` and `verified_by` |
| `constraint` | `policy` | `measure`; may declare `blocks`; a legal `ref` scope and `exceptions` target |
| `term` | `policy` | `aliases`, still searched |
| `track` | `milestone`, or `policy` for standing work that never ends | every structural role; `cadence` is not carried over |

A record that already has a retired kind may still be revised and moved through its
lifecycle. It leaves the kind by supersession: `akr supersede <key> --with <new-key>
--from <body>` creates the replacement and retires the old head in one write, and
`supersedes` may cross kinds along this table and nowhere else (V-014). Nothing folds
automatically, because the replacement kinds have obligations the retired ones did not —
an observation needs `observed_at`, a milestone needs acceptance checks — and a tool
cannot invent those. `akr check` reports how many live records of retired kinds a ledger
holds, as a build fact.

### 3.3 Where reasoning goes

The ledger exists to answer *why was this done*, not *what was done* — git answers the
second. Three tiers (D-047):

| What is being explained | Where it goes |
| --- | --- |
| A line of code | A comment beside it. Never a record. |
| A step of work: a reason, a parameter choice, a dead end | An entry in the work record's `trace` (§4.10). |
| A choice that binds the project, or a finding that will be re-read | A `decision` (`context`, `consequences`) or an `observation`. |

---

## 4. Kind reference

Each section gives the kind's slots, an example, and the mistakes that actually get
made. Slot order in every example is canonical (D-012); `akr fmt` will produce exactly
this ordering.

### 4.1 `term` (normative)

Fixes a word. Terms exist because the alternative is every record silently using its own
definition of "done", "day", or "boundary".

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `definition` | prose | yes | What the word means here. Not a dictionary entry — a project decision. |
| `aliases` | string[] | no | Other spellings that mean the same thing. Not synonyms with shades of difference. |

```
record sys.term.playable-day/1 : term {
    title "Playable day"
    state active
    scope [ all ]
    definition """
        One in-game day, from the morning wake state to the following morning wake
        state, played end to end by one player without a crash, a soft-lock, or a
        placeholder asset.
        """
    aliases [ "playable day", "day-loop build" ]
    claim day-boundary {
        text """
            A day boundary is the morning wake state, not midnight.
            """
    }
}
```

*Common mistakes.* Writing a term for a word nobody disputes. Writing a definition that
describes the current implementation rather than the intent — that is an observation.
Using `aliases` for near-synonyms that should be separate terms.

### 4.2 `requirement` (normative)

Something the delivered system must do or be.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `statement` | prose | yes | Written so that it is checkable, or so that its unfulfillment is recognisable. |
| `rationale` | prose | no | Why. Saves the next person from relitigating it. |

```
record sys.req.deterministic-sim/1 : requirement {
    title "The simulator is reproducible from a seed"
    state active
    scope [ path "sim/**" ]
    statement """
        Given the same seed and the same input sequence, two runs of the simulator
        produce byte-identical state at every tick boundary.
        """
    rationale """
        Reproducibility is what makes a simulation bug a bug report rather than a
        story.
        """
    verified_by [ @sim.evidence.determinism-suite-pass ]
}
```

*Common mistakes.* Writing a requirement that is really a decision ("we will use a fixed
timestep" is a choice, not an obligation). Writing several requirements in one, joined
by "and". Omitting scope, so nothing can tell whether it applies to a given change.

### 4.3 `policy` (normative)

A standing rule about how the project works.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `rule` | prose | yes | Written in the imperative. It should be possible to violate it. |
| `rationale` | prose | no | Why the rule exists: what it protects, and what was seen without it. |
| `exceptions` | ref[] | no | Milestones, tracks, work items, constraints or policies where the rule is suspended. |
| `measure` | string | no | The limit as a number or a testable condition, when there is one (D-046). |
| `aliases` | string[] | no | Other words a reader might search for the rule, or for the word it defines (D-046). |

```
record sys.policy.tandem-work/1 : policy {
    title "Engine and simulator advance in tandem"
    state active
    scope [ all ]
    topic tandem-work
    rule """
        No engine change lands without the matching simulator change in the same
        commit, except on the tracks listed under exceptions, where the simulator may
        lag by at most one milestone.
        """
    rationale """
        Divergence between the two has cost more time than the coupling does.
        """
    exceptions [ @sys.track.lighting ]
    supported_by [ @sys.assessment.projection-gaps ]
}
```

`exceptions` is a content slot, not a relation: it names where the rule does not apply,
which is part of the rule's meaning. It is checked for kind-correctness (V-005) but
carries no other mechanical consequence.

*Common mistakes.* Writing the exception into the prose instead of the `exceptions`
slot, which makes it invisible to scope resolution. Giving two policies the same
`topic` without meaning to (§10). Writing a policy that is really a decision — a policy
governs indefinitely, a decision was made once.

### 4.4 `constraint` (normative)

A limit the project must respect but did not choose.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `statement` | prose | yes | What the limit is, and where it comes from. |
| `measure` | string | no | The limit in numbers, if it has any. |
| `rationale` | prose | no | |

```
record sys.constraint.frame-budget-16ms/1 : constraint {
    title "16 ms frame budget"
    state active
    scope [ path "lege/**", path "sim/step.rs" ]
    statement """
        The target hardware runs at 60 Hz. A frame that takes longer than 16 ms drops,
        and dropped frames in the day loop read as a broken game rather than a slow
        one.
        """
    measure "16 ms at p99, measured over a 20-minute session"
}
```

Constraints are the most common target of `scope [ ref ... ]` in other records, because
"this applies wherever that limit applies" is a frequent thing to say.

*Common mistakes.* Recording a constraint the team actually chose, which should be a
decision or a requirement. Putting the number only in prose, so nothing can cite it.

### 4.5 `decision` (normative)

A choice made between alternatives.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `decision` | prose | yes | What was decided. One choice per record. |
| `context` | prose | no | What was true when it was made. |
| `consequences` | prose | no | What follows, including what got worse. |

```
record lege.decision.renderer-boundary/2 : decision {
    title "The viewer consumes a frame snapshot"
    state active
    scope [ path "lege/**" ]
    topic renderer-boundary
    decision """
        The viewer reads an immutable frame snapshot produced by the simulator at each
        tick boundary. It does not call into the simulator and does not name engine
        types in any signature.
        """
    context """
        Revision 1 let the viewer call the simulator directly, which put engine types
        in viewer signatures and made the viewer untestable in isolation.
        """
    consequences """
        One extra allocation and copy per frame, measured at 0.4 ms. Accepted against
        the 16 ms budget.
        """
    derived_from [ @lege.obs.viewer-imports-engine/1 ]
    implements [ @lege.req.no-engine-types-in-viewer ]
    resolves [ @lege.question.text-rendering-owner ]
    supersedes [ @lege.decision.renderer-boundary/1 ]
}
```

An `active` decision must cite something: a requirement, a policy, a constraint, or
evidence (V-021, `AKR-R031`). A decision resting on nothing is a preference, and the
rule exists to make you notice which one you have.

*Common mistakes.* Bundling two choices into one decision, so half of it can never be
superseded independently. Leaving `consequences` empty when the decision had a real cost
— that is exactly the sentence the next reader needs.

### 4.6 `observation` (empirical)

What was found to be true of the system, at a specific commit.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `statement` | prose | yes | What is true. Present tense, about the code as it was. |
| `observed_at` | commit | **yes** | The commit the observation is about (V-009). |
| `method` | enum | no | `manual`, `command`, `instrumented`, `observation`. |
| `watches` | glob[] | no | Paths whose change should make someone re-check this. |
| `review_after` | date | no | A date after which this should be re-checked regardless. |
| `confidence` | enum | no | `low`, `medium`, `high`: how far a judgement, as opposed to a measurement, can be trusted (D-046). |

```
record sim.obs.projection-gaps/1 : observation {
    title "Projection coverage is thinnest at day boundaries"
    state verified
    scope [ path "sim/src/project/**" ]
    statement """
        Across the projection suite the least-covered paths cluster at the transition
        from one in-game day to the next: 41 percent line coverage there against 88
        percent for steady-state paths.
        """
    observed_at git:7c41d0ba92e6f37518a3cd406b5e2f91d8074a63
    method command
    watches [ "sim/src/project/**" ]
    review_after 2026-11-01
}
```

`observed_at` is required and unabbreviated (D-008) because an observation without a
commit is a rumour. `watches` is what makes staleness work: when a commit reachable from
HEAD but not from `observed_at` touches a matching path, the record is flagged stale and
everything resting on it is flagged at risk (D-024). Nothing is ever declared false —
the flag says "someone should look", not "this is wrong".

**`needs-review` is not a state.** The planning notes listed it as one; it is not
(D-003). Staleness is derived at build time from `observed_at`, `watches`,
`review_after`, and the current commit. The build never writes a source file, so it can
never mark a record as needing review — it computes the fact, reports it in
`akr review-queue` and `REVIEW-REQUIRED.md`, and leaves the ledger alone.

*Common mistakes.* Writing an observation with no `watches`, which means nothing can
ever tell you it went stale. Writing a conclusion ("the projection pass is a problem")
rather than a finding — that is an assessment.

### 4.7 `evidence` (empirical)

The outcome of a check that was actually run.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `result` | enum | yes | `pass`, `fail`, `inconclusive`. |
| `method` | enum | yes | `manual`, `command`, `observation`. |
| `observed_at` | commit | yes | The commit the check ran against. |
| `command` | string | no | The exact command, if there was one. |
| `artifact` | string | no | Path or URL to the output. |
| `summary` | prose | no | What it showed, briefly. |

```
record sim.evidence.determinism-suite-pass/1 : evidence {
    title "Determinism suite green"
    state verified
    result pass
    method command
    observed_at git:5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285
    command "cargo test -p sim --test determinism -- --seed-sweep 512"
    artifact "artifacts/2026-06-30-determinism.log"
    summary """
        512 seeds, 10 000 ticks each, byte-identical state at every tick boundary.
        """
}
```

**Evidence never says what it verifies.** The `verified_by` relation runs one way only,
from the thing being verified to the evidence (D-016). A milestone's acceptance check
points at evidence; evidence points at nothing. This keeps acceptance readable in one
place and lets one evidence record satisfy several checks without any reconciliation
rule.

`result fail` is a perfectly good record and should be kept. A failed check that was
recorded is knowledge; a failed check that was deleted is a bug you will rediscover.

*Common mistakes.* Recording evidence with `result pass` for a check that was not
actually run. Omitting `command`, which makes the evidence unreproducible. Trying to add
a `verifies` slot — there isn't one.

### 4.8 `assessment` (empirical)

A judgement drawn from observations.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `statement` | prose | yes | The judgement. |
| `confidence` | enum | no | `low`, `medium`, `high`. |
| `as_of` | commit | no | The state of the world the judgement was made about. |

```
record sys.assessment.projection-gaps/1 : assessment {
    title "Projection gaps put the M3 date at risk"
    state verified
    scope [ ref @sys.milestone.m3-playable-day ]
    statement """
        The uncovered day-boundary paths are on the critical path for M3. On current
        evidence the rewrite is two weeks of work that has not been scheduled.
        """
    confidence medium
    as_of git:5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285
    supported_by [ @sim.obs.projection-gaps ]
}
```

Assessments are the main consumers of `supported_by` and the main carriers of
propagated staleness: when the observation under an assessment goes stale, the
assessment is flagged at risk, which is exactly the signal a plan reader wants.

*Common mistakes.* Writing an assessment that restates its observation without adding a
judgement. Omitting `supported_by`, which makes the judgement unfalsifiable and, more
practically, unflaggable.

### 4.9 `milestone` (planning)

A named point at which a defined set of acceptance checks passes.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `intent` | prose | yes | What is true when this milestone is reached. |
| `target` | date | no | Intended date. Never enforced; missing it is not an error. |
| `note` | prose | no | Operator commentary (D-026). Informational; no rule reads it. |
| `acceptance` | block | **yes** | What "done" means. See §9. |

```
record sys.milestone.m3-playable-day/1 : milestone {
    title "M3 — playable day"
    state active
    intent """
        A player can start, play and finish one in-game day without a crash, a
        soft-lock, or a placeholder asset.
        """
    target 2026-09-30
    acceptance {
        check full-day-demo {
            statement """
                A recorded session shows one complete in-game day, start to finish.
                """
            method observation
            verified_by [ @sys.evidence.playable-day-demo ]
        }
        check no-placeholder-assets {
            statement """
                The asset audit reports zero placeholder assets on the day-loop path.
                """
            method command
            command "cargo run -p tools -- audit-assets --path content/day-loop"
        }
    }
    after [ @sys.milestone.m2-deterministic-sim ]
    depends_on [ @sys.term.playable-day ]
}
```

`acceptance` is required on milestones, without exception. A milestone whose definition
of done is "we'll know it when we see it" is the failure this whole system exists to
prevent, so the grammar does not let you write one.

Completing a milestone with any unsatisfied check is `AKR-R022` (V-020).

*Common mistakes.* Writing checks that restate the intent instead of describing an
observable outcome. Using `after` to mean "should probably follow" rather than "cannot
start before" — `after` is a hard ordering constraint and its graph must be acyclic.

### 4.10 `work` (planning)

A unit of intended change. A work record designated `plan_of_record` for a milestone or
track is what other systems would call a plan; AKR has no separate `plan` kind (D-001).

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `intent` | prose | yes | The outcome wanted and what makes it necessary. Not a list of files: the diff shows those. |
| `target` | date | no | |
| `note` | prose | no | Operator commentary (D-026). `akr abandon --reason` writes it. |
| `trace` | string[] | no | Reasoning steps, appended as the work proceeds (D-047). See below. |
| `acceptance` | block | no | Optional; a work item may borrow its milestone's. |
| `disposition` | block[] | conditional | Required when superseding a planning record with unfinished children (§8.3). |

```
record sys.work.m3-plan/2 : work {
    title "M3 plan of record"
    state active
    intent """
        Land the renderer boundary first, then the sim step rewrite, then the asset
        audit. Lighting moves to the standing track; ambient audio is dropped.
        """
    target 2026-09-15
    disposition @sys.work.m3-audio-pass {
        outcome intentionally_dropped
        note """
            Ambient audio is not part of a playable day. Revisit after M4.
            """
    }
    disposition @sys.work.m3-lighting-pass {
        outcome carried_forward
        into @sys.track.lighting
    }
    implements [ @sys.policy.tandem-work ]
    plan_of_record [ @sys.milestone.m3-playable-day ]
    supersedes [ @sys.work.m3-plan/1 ]
}
```

At most one live work record may be `plan_of_record` for a given milestone or track
(V-018). Two live plans for one milestone is the ambiguity the invariant exists to
forbid.

### The `trace` slot

`trace` is where a work record says *why*: one short entry per reasoning step — why this
approach, what the code as found forced, what was tried and dropped (D-047).

```
trace [
    "renderer boundary first: the sim step cannot be tested while the viewer owns the passes",
    "dropped the shared clock: it made a day boundary depend on frame rate"
]
```

- **Appended in place.** `akr trace <work> "<why>"` (`knowledge.trace`) adds an entry to
  the head revision whether or not it is sealed. No revision is minted, no acceptance
  check is put back in question (`trace` is bookkeeping under D-029, like `note`), and
  `akr build` reseals the record. Entries are never edited or removed.
- **Short.** An entry over 25 words draws a note; one over 40 is refused (V-026,
  `AKR-T024`). Explain in proportion to the expected impact of the change by adding
  entries, never by lengthening them.
- **Never required.** Nothing refuses a work record with no trace. Completing one draws a
  note.
- **Linked to commits by derivation.** `akr get <work>` prints each entry beside the
  commit that introduced it, computed from git history; no commit id is stored (D-032).
  A commit made through `akr change` carries the entries it adds as `AKR-Why:` trailers.

A trace entry is not a changelog line. "Added `Cache::warm` and three tests" restates the
diff. "Warmed on start: a cold first query took 4 s and users read it as a hang" is why.

### The `note` slot

`note` is free-form operator commentary on a planning record, and it is the one slot in
the vocabulary that no rule reads (D-026). Nothing requires it, nothing validates it, and
nothing fails if it is absent. `akr abandon --reason` writes the reason there, and views
render it for records in terminal states, so the reason a plan was dropped is visible in
`ACTIVE-WORK.md` and `DECISION-HISTORY.md` rather than buried in a source comment.

It exists only on the planning kinds. Normative and empirical records already have a home
for every kind of prose they should carry — `rationale`, `context`, `consequences`,
`summary` — and a general commentary slot on them would become the metadata bag §12
refuses to have. Planning records are the ones operators abandon, carry forward and
re-schedule mid-flight, and that is the commentary this is for.

*Common mistakes.* Superseding a plan without dispositioning its children — the single
most valuable check in the system, and the one people most want to skip (§8.3). Making a
work item `part_of` a milestone directly when it belongs to the plan. Using `note` to
carry something a typed slot already holds, which puts knowledge where nothing can check
it.

### 4.11 `track` (planning)

Standing work that no milestone contains, and that does not end.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `intent` | prose | yes | What the track keeps doing. |
| `cadence` | string | no | How often, in whatever units make sense. |
| `note` | prose | no | Operator commentary (D-026). Informational; no rule reads it. |

```
record sys.track.lighting/1 : track {
    title "Lighting"
    state active
    scope [ path "lege/src/light/**", path "content/**/light/**" ]
    intent """
        Standing lighting work that no milestone contains: one pass per milestone,
        plus fixes as scenes land.
        """
    cadence "one pass per milestone"
}
```

Tracks are the answer to work that would otherwise be forced into a milestone it does
not belong to, and they are the usual destination for `carried_forward` dispositions.
A track normally stays `active` for the life of the project.

*Common mistakes.* Using a track for work that does have an end — that is a milestone or
a work item. Giving a track acceptance checks; it has no acceptance block because it has
no completion.

### 4.12 `question` (inquiry)

An open matter that blocks or endangers something.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `question` | prose | yes | The question, in a form that could be answered. |
| `resolution` | prose | conditional | Required when `state resolved` (V-011). |

```
record sim.question.timestep-vs-budget/1 : question {
    title "Does a 4 ms timestep fit the frame budget?"
    state open
    question """
        At 4 ms the simulator runs four steps per rendered frame. Does the resulting
        per-frame cost fit inside the 16 ms budget on target hardware, with the
        renderer's share unchanged?
        """
    blocks [ @sim.decision.timestep-4ms, @sim.work.rewrite-projection ]
    depends_on [ @sys.constraint.frame-budget-16ms ]
}
```

A question in `resolved` state needs both a `resolution` slot and a live `resolves` edge
pointing at it from whatever resolved it (V-011). Answering a question in prose without
recording what answered it loses the link between the answer and the decision that used
it.

`closed-without-resolution` is a real and useful state: the question stopped mattering.
It is not the same as `resolved`, and conflating them is how a project forgets that it
never actually found out.

A question that blocks nothing is still worth recording. Give it a `blocks` edge when
something really is waiting on the answer, and leave it bare when nothing is: an
unscheduled question surfaces in `OPEN-QUESTIONS.md` either way. Two real examples from
`examples/sys-tandem/`: `engine.question.voice-direction` is deferred with no scheduled
milestone, and `tandem.question.step-6-polish` is explicitly not a milestone blocker.
Inventing a `blocks` edge for either would be asserting a dependency the project does not
have.

*Common mistakes.* Adding a `blocks` edge the project does not actually have, so that a
milestone reads as stalled on a question nobody is waiting for. Deleting questions once
answered instead of resolving them.

---

## 5. Lifecycles

Four state machines, one per class (D-002). Every kind uses its class's machine
unchanged.

A state is **live** or **terminal**. Live means the record still speaks for itself;
terminal means it does not. Only live records participate in ordinary context assembly,
and exactly one revision of a key may be live at a time (V-012, D-004a). A completed
planning record is the deliberate exception to dependency liveness: completion satisfies
a `depends_on` prerequisite, so the edge remains valid and preserves the plan's history.

### 5.1 Normative: `term`, `requirement`, `policy`, `constraint`, `decision`

```
                 accept
    proposed ─────────────► active
       │                      │
       │ reject               │ supersede
       ▼                      ▼
    rejected              superseded
       ▲                      ▲
       │ supersede            │
       └──────────────────────┘
       │ withdraw             │ withdraw
       ▼                      ▼
    withdrawn ◄───────────────┘
```

| From | To | Trigger | Meaning |
| --- | --- | --- | --- |
| `proposed` | `active` | accept | It now binds. |
| `proposed` | `rejected` | reject | Considered and declined. Kept, because "we said no to that" is knowledge. |
| `proposed` | `withdrawn` | withdraw | Taken back before anyone ruled on it. |
| `proposed` | `superseded` | supersede | Replaced by a later revision while still a proposal. |
| `active` | `superseded` | supersede | Replaced. The replacement must exist and must say so. |
| `active` | `withdrawn` | withdraw | No longer binds, and nothing replaces it. |

Live: `proposed`, `active`. Terminal: `rejected`, `superseded`, `withdrawn`.

There is no transition from `active` back to `proposed`, and none out of any terminal
state. To revive a withdrawn policy, write a new revision — the history stays legible.

### 5.2 Empirical: `observation`, `evidence`, `assessment`

```
                 disprove
    verified ──────────────► disproven
       │
       │ supersede               withdraw
       ├────────► superseded     ├────────► withdrawn
```

| From | To | Trigger | Meaning |
| --- | --- | --- | --- |
| `verified` | `disproven` | disprove | Someone checked and it is not true. |
| `verified` | `superseded` | supersede | A newer observation of the same thing replaces it. |
| `verified` | `withdrawn` | withdraw | Retracted, usually because it was never properly observed. |

Live: `verified`. Terminal: `disproven`, `superseded`, `withdrawn`.

Empirical records are authored `verified` — they are recorded *because* someone looked.
There is no `proposed` observation; a guess is an assessment with `confidence low`, or it
is nothing.

`disproven` is deliberately distinct from `superseded`. Superseded means "a newer
measurement replaces this one"; disproven means "this was wrong". Both stay in the
ledger, and the difference matters to anyone reading the history of a bug.

**Staleness is not a state.** A stale record is still `verified`; staleness is a derived
property of (record, commit) computed at build time (D-003, D-024). See §6.

### 5.3 Planning: `milestone`, `work`, `track`

```
    proposed ──ready──► ready ──start──► active ──complete──► completed
        │                 │                │  ▲
        │                 │ block          │  │ unblock
        │                 ▼                ▼  │
        │              blocked ◄────block───  │
        │                 │                   │
        └─── abandon ─────┴──── abandon ──────┘──► abandoned

    any live state ── supersede ──► superseded
```

| From | To | Trigger |
| --- | --- | --- |
| `proposed` | `ready` | ready |
| `proposed` | `abandoned` / `superseded` | abandon / supersede |
| `ready` | `active` | start |
| `ready` | `blocked` | block |
| `ready` | `abandoned` / `superseded` | abandon / supersede |
| `active` | `blocked` | block |
| `active` | `completed` | complete |
| `active` | `abandoned` / `superseded` | abandon / supersede |
| `blocked` | `active` | unblock |
| `blocked` | `abandoned` / `superseded` | abandon / supersede |

Live: `proposed`, `ready`, `active`, `blocked`. Terminal: `completed`, `abandoned`,
`superseded`.

Two rules give these states teeth. `completed` requires every acceptance check satisfied
(V-020). `blocked` is expected to be justified by a live `blocks` edge pointing at the
record — a blocked item with no blocker is a work item nobody wants to admit is stalled.

`abandoned` and `superseded` are different: abandoned means the work is not happening,
superseded means a newer revision replaces this plan. Superseding a planning record with
unfinished children requires dispositioning them (§8.3).

### 5.4 Inquiry: `question`

```
    open ──defer──► deferred ──reopen──► open
      │                │
      │ resolve        │ resolve
      ▼                ▼
    resolved      resolved

    open / deferred ──close──► closed-without-resolution
    open / deferred ──supersede──► superseded
```

Live: `open`, `deferred`. Terminal: `resolved`, `closed-without-resolution`,
`superseded`.

`deferred` means the question still matters but not now; `closed-without-resolution`
means it stopped mattering. The distinction is the whole point of having four terminal
options instead of a boolean.

### 5.5 Illegal combinations

The type checker rejects any state not in the kind's class machine (V-007,
`AKR-T011`). `state completed` on a policy, `state active` on an observation, and
`state needs-review` on anything are all errors at stage B, before any graph is built.
The error names the kind, the class, and the legal set.

---

## 6. Derived properties

Three properties are computed, never authored. They appear in the index, in views, and
in context bundles, and never in a `.akr` file.

| Property | Applies to | Computed from |
| --- | --- | --- |
| **head** | every key | The one live revision (D-004a, `docs/04` §3) |
| **stale** | live empirical records | `observed_at` versus HEAD against `watches`; or `review_after` versus today (D-024) |
| **at_risk** | any live record | Transitive closure of `supported_by`, `depends_on`, `derived_from` from a stale record (D-024) |

Staleness carries no diagnostic code and never changes an exit status. A project with
stale knowledge still builds — building is how you find out. Projects wanting a hard gate
opt in with `akr check --review-clean`.

---

## 7. Relations

Twelve relations, each with a fixed domain, range, and mechanical consequence. A
relation slot is always an array of references, even with one element, and always uses
the relation's own name.

| Relation | Domain | Range | Card. | Acyclic | Carries staleness | Consequence |
| --- | --- | --- | --- | --- | --- | --- |
| `supported_by` | all but empirical | `observation`, `evidence`, `assessment` | many | yes | **yes** | The source's standing rests on the target. |
| `depends_on` | all but `evidence` | any but `question` | many | yes | **yes** | A completed planning target satisfies the dependency; other terminal targets invalidate it (V-019). |
| `supersedes` | any | same kind | many | yes | no | Puts the target in `superseded`; triggers disposition checks. |
| `contradicts` | any | any | many | n/a | no | Symmetric. Must be dispositioned. Always surfaced. |
| `implements` | `work`, `decision` | `requirement`, `policy`, `constraint`, `decision` | many | yes | no | Ties change to what motivates it. |
| `resolves` | `decision`, `observation`, `evidence`, `work` | `question` | many | yes | no | A `resolved` question needs one. |
| `derived_from` | any | any | many | yes | **yes** | Provenance between records. |
| `part_of` | `work`, `milestone`, `requirement`, `question` | `milestone`, `track`, `work` | **one** | yes | no | Defines the child set for disposition and ref-scope overlap. |
| `after` | `milestone`, `work` | `milestone`, `work` | many | yes | no | Hard ordering. Graph must be acyclic. |
| `blocks` | `question`, `work`, `observation`, `constraint` | `milestone`, `work`, `decision` | many | yes | no | A live blocker justifies `blocked` and surfaces in context. |
| `verified_by` | `milestone`, `work`, `requirement`, `assessment`, `observation`, `check` | `evidence` | many | yes | **yes** | Satisfies acceptance under the descendant-commit rule. One direction only. |
| `plan_of_record` | `work` | `milestone`, `track` | **one** | yes | no | Designates the authoritative plan. At most one live per target (V-018). |

Exact domain and range lists are in `spec/tables/vocabulary.json`; the table above is the
readable form of the same data and is checked against it.

### 7.1 Notes on the ones that trip people

**`supported_by` versus `depends_on`.** `supported_by` points at empirical records and
means "this is why I believe it". `depends_on` points at anything and means "this must
remain valid or be completed successfully". A policy is `supported_by` an assessment; a
work item `depends_on` a decision or prerequisite milestone. Both carry staleness; only
`supported_by` reads as evidence in a view.

**`part_of` is single-parent.** A record has at most one parent. Two parents means the
child set used for disposition is ambiguous, and disposition is the check worth
protecting.

**`contradicts` is symmetric.** Declare it once, from either side; the resolver treats
it as an undirected edge. It is the only relation with no acyclicity requirement,
because a contradiction cycle is exactly what you want reported rather than rejected.

**`after` is not `part_of`.** `after` says M3 cannot start before M2. `part_of` says
this work item belongs to that plan. Using `after` for containment produces a plan with
no children and a disposition check that never fires.

### 7.2 Inverses

Inverses are computed, never written. The index materialises them and the CLI exposes
them (`akr get --with-inverse`, `akr impact`), but there is no `supports` or
`part_of_inverse` slot to author. One direction per fact, always.

---

## 8. Supersession, revision, and disposition

### 8.1 Revisions

A revision is created by `akr revise`, which copies the current head, increments the
revision number, and leaves the previous revision for you to mark `superseded`. Both
revisions live in the same file (V-003), so the whole history of a key is one diff away.

A revision in any state other than `proposed` is **sealed**: its content hash is
recorded in `akr.lock`, and changing its text afterwards is `AKR-R051` (D-015). This is
what makes "accepted bodies are immutable" enforceable rather than aspirational.
`proposed` revisions are editable, which is what makes `proposed` worth having.

### 8.2 Supersession chains

`supersedes` forms a directed acyclic graph, checked at resolve (V-014). Usually it is a
simple chain within one key: `/3` supersedes `/2` supersedes `/1`. It may also cross
keys, when one record genuinely replaces another under a different name — the target
must be the same kind.

Following a chain forward from any revision reaches at most one live head. Following it
backward gives the history a reader wants when asking "why is this the way it is". Both
walks are in `docs/04`.

### 8.3 Disposition

When a planning record supersedes another that has unfinished children, the superseding
record must say what happened to each one (D-017, V-017, `AKR-R014`). An unfinished
child is any record in a live planning state related to the superseded record by
`part_of`.

```
disposition @sys.work.m3-lighting-pass {
    outcome carried_forward
    into @sys.track.lighting
}
```

| `outcome` | `into` | Meaning |
| --- | --- | --- |
| `carried_forward` | required | Still to be done, under the target named by `into`. |
| `completed_elsewhere` | required | Already done, by the work named by `into`. |
| `intentionally_dropped` | **forbidden** | Decided against. The `note` should say why. |
| `still_required_separately` | optional | Still needed, but not part of any current plan. The honest answer when there is no home for it yet. |

This is the most valuable check in the system. Work silently vanishing across a replan is
the failure that makes long-running agent-driven projects untrustworthy: nobody decided
to drop the audio pass, it just stopped being mentioned. The check costs one block at the
moment the author knows the answer, and it converts "we must have decided that" into a
sentence with a name on it.

Note what the rule does *not* do: it does not require the child's state to change.
`sys.work.m3-audio-pass` can be dispositioned `intentionally_dropped` and remain `ready`
until someone abandons it. The disposition records the decision; the state change is a
separate write. The worked example deliberately sits between those two steps, because
that is the state a reviewer actually encounters.

---

## 9. Acceptance and checks

Acceptance is how a planning record says what "done" means, and it is the only thing
that lets `completed` mean anything.

```
acceptance {
    check determinism-suite-green {
        statement """
            The determinism suite passes across a 512-seed sweep.
            """
        method command
        command "cargo test -p sim --test determinism -- --seed-sweep 512"
        verified_by [ @sim.evidence.determinism-suite-pass ]
    }
}
```

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `statement` | prose | yes | The observable outcome. Not the activity — "the suite passes", not "run the suite". |
| `method` | enum | yes | `manual`, `command`, `observation`. |
| `command` | string | no | Required in practice for `method command`, or the check is not reproducible. |
| `verified_by` | ref[] | no | Evidence records. Absent until something is actually run. |
| `passed_at` | commit | no | The commit at which a `command` check was run and passed (D-048). |

### 9.1 When a check is satisfied

A check is satisfied in one of two ways.

**By its own recorded pass (D-048).** A check with `method command` that carries
`passed_at`, naming a commit this repository has. `akr complete <key> --pass <check>`
writes it, defaulting to `HEAD`. This is the ordinary case — the command is already on
the check, so the commit is the only fact left to record, and no evidence record is
needed. Any write that changes what the record requires drops every `passed_at` it would
have carried forward, so a stamp that is present was written against the definition
beside it. It is refused on a `manual` or `observation` check, which has no recorded
procedure: what was seen has to be written down, and that is evidence.

**By evidence.** At least one referenced evidence record has:

1. `result pass`, and
2. an `observed_at` commit that is a **descendant of** the last commit that changed the
   content of the planning record's current revision.

Use evidence for a measurement, a captured artefact, a failure or an inconclusive run,
and anything a later reader will want to read.

Condition 2 is what stops a green test run from 200 commits ago closing a milestone
whose definition changed yesterday. If you edit a milestone's acceptance, its evidence
stops counting until something is re-run — which is correct, and occasionally annoying,
and worth it.

`akr complete` refuses to complete a record with an unsatisfied check, and a hand-written
`state completed` fails the build with `AKR-R022`.

### 9.2 Acceptance on `work`

Optional. A work item under a milestone often borrows the milestone's acceptance and
needs none of its own. Give a work item acceptance when it can be finished
independently, and especially for migration tracking records, where the checks enumerate
the disposition of a legacy document's claims (D-022).

---

## 10. Scope and `topic`

### 10.1 Scope terms

`scope` is an array of terms. Three forms, no others (D-010):

| Term | Example | Means |
| --- | --- | --- |
| `all` | `scope [ all ]` | Project-wide. |
| `ref <ref>` | `scope [ ref @sys.track.lighting ]` | Wherever that milestone, track, or constraint applies. |
| `path <glob>` | `scope [ path "sim/src/**" ]` | These files. |

Scope is required on normative kinds and optional elsewhere. On an observation it
records what was looked at; on a policy it records what is governed; and on both it is
what lets `akr context --paths` decide whether the record is relevant to the change in
front of an agent.

### 10.2 The overlap algorithm

Overlap decides the exclusivity rule (§10.3), so it must be decidable, cheap, and
identical across implementations. Two scopes overlap if **any** term of one overlaps
**any** term of the other:

```
overlaps(A, B):
    for a in A.terms:
        for b in B.terms:
            if term_overlaps(a, b): return true
    return false

term_overlaps(a, b):
    if a is all or b is all:            return true
    if a is ref and b is ref:           return a.key == b.key
                                            or part_of_ancestor(a.key, b.key)
                                            or part_of_ancestor(b.key, a.key)
    if a is path and b is path:         return glob_prefixes_comparable(a, b)
    return false                        # a ref and a path never overlap

glob_prefixes_comparable(a, b):
    pa = literal segment prefix of a, up to its first wildcard segment
    pb = literal segment prefix of b, up to its first wildcard segment
    return pa is a prefix of pb or pb is a prefix of pa
```

The test is deliberately **conservative**: it may report overlap where none exists in
practice, and it must never miss one. A false positive is resolved by narrowing a scope
or dropping a `topic`; a false negative would silently permit two contradictory policies
to govern the same code.

Worked examples:

| Scope A | Scope B | Overlap? | Why |
| --- | --- | --- | --- |
| `[ all ]` | `[ path "sim/**" ]` | yes | `all` overlaps everything |
| `[ path "sim/**" ]` | `[ path "sim/src/project/**" ]` | yes | `sim` is a prefix of `sim/src/project` |
| `[ path "sim/**" ]` | `[ path "lege/**" ]` | no | Neither prefix contains the other |
| `[ path "sim/*/mod.rs" ]` | `[ path "sim/src/**" ]` | **yes** | Both literal prefixes are `sim`; conservative, and in fact they do intersect |
| `[ path "sim/*/mod.rs" ]` | `[ path "sim/tests/**" ]` | **yes** | Conservative false positive: prefixes both `sim`. Narrow the first scope if this bites. |
| `[ ref @sys.track.lighting ]` | `[ ref @sys.track.lighting ]` | yes | Same key |
| `[ ref @sys.work.m3-lighting-pass ]` | `[ ref @sys.track.lighting ]` | yes | The first is `part_of` the second |
| `[ ref @sys.track.lighting ]` | `[ path "lege/src/light/**" ]` | no | A ref and a path never overlap directly |

The last row is the one to remember: if a record must be compared against both an
organisational scope and a code scope, declare both terms. The alternative — inferring
paths from a track's contents — would make overlap depend on the whole graph and change
whenever an unrelated record moved.

### 10.3 `topic` and normative exclusivity

`topic` is an optional identifier on normative records. Two live normative records that
share a `topic` and whose scopes overlap is `AKR-R002` (V-013, D-004b).

```
record sys.policy.tandem-work/1 : policy {
    topic tandem-work
    scope [ all ]
    ...
}
```

A second live policy with `topic tandem-work` and any overlapping scope fails the build,
naming both records. Without a `topic`, neither record is ever in conflict by this rule.

Why opt-in rather than inferred? Because "these two speak to the same thing" is a
judgement about meaning, and inferring it from prose would need exactly the kind of
reasoning the compiler is not allowed to do (D-020). Writing one identifier is cheap; a
compiler guessing at semantic conflict is not.

Use a `topic` when you expect the rule to be revised and want the build to catch a
half-finished revision that left both versions active. Skip it for one-off decisions
that will never have a competitor.

---

## 11. Sources and legacy provenance

`source` is a repeatable block recording where a record's content came from.

| Slot | Type | Required | Notes |
| --- | --- | --- | --- |
| `kind` | enum | yes | `legacy`, `external`, `internal`. |
| `path` | string | no | Repo-relative path. |
| `url` | string | no | For `external`. |
| `excerpt` | prose | no | The passage this record came from. |

```
source {
    kind legacy
    path "docs/legacy/ROADMAP.md"
    excerpt """
        M3 — playable day. Ship the day loop; lighting can trail.
        """
}
```

Migration adds no kinds (D-022). A legacy document being imported gets one tracking
`work` record whose acceptance checks enumerate the disposition of its durable claims;
the document is archived only when that work record reaches `completed`, which by V-020
requires every check satisfied. `source { kind legacy }` on each imported record is the
trail back.

Note that `path` here is provenance, not identity. A record does not become invalid when
its source file is deleted — that is the entire point of importing it.

---

## 12. Deliberately absent

The following do not exist, and their absence is a design position rather than an
oversight.

**No free-form metadata.** No `tags`, no `labels`, no `metadata { ... }` escape hatch.
Every slot means something to the compiler or to a defined view. A metadata bag becomes
a second, unvalidated schema within a month, and then the real schema is in nobody's
head.

**No priority or severity field.** Priority is a property of a plan at a moment, not of
a record. It belongs in the ordering of a plan's children and in what is `active` versus
`ready`. A `priority high` slot that nobody re-sorts is worse than nothing.

**No assignee.** AKR records knowledge, not task assignment. `author` says who wrote the
revision; it is free text and not an identity system. Who is doing the work lives in
whatever tracker the project already has.

**No percent-complete.** A work item is `proposed`, `ready`, `active`, `blocked`,
`completed`, or `abandoned`. "70 percent" is a feeling, and it is the feeling that
precedes a missed milestone.

**No `plan` kind.** A plan is a `work` record with `plan_of_record` (D-001).

**No `goal` kind.** The planning notes used `@sys.goal.playable-day`; that is a
`milestone` when it has acceptance and a `term` when it is a definition. Both existed
already.

**No arbitrary nesting.** Records do not contain records. A record that wants children
uses `part_of` from the children, which keeps every record independently addressable,
supersedable, and scopable.

**No inverse relation slots.** One direction per fact (§7.2).

**No cross-project references.** `@key` resolves within one project in 0.1. Federation
raises questions about authority across trust boundaries that the model has no answer
for yet, and a bad answer would be hard to withdraw.

---

## 13. Where to go next

- `docs/03-syntax.md` — how all of this is written down, and what `akr fmt` guarantees.
- `docs/04-references-and-versioning.md` — keys, heads, reference modes, and the lock.
- `docs/05-validation-rules.md` — the twenty-four rules, with failing and passing
  examples for each.
- `examples/save-your-skin/` — every mechanism above, exercised on one small project.
