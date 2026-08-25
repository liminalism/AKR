# Validation Rules

The catalogue of everything `akr check` enforces, and — just as importantly — what it
deliberately does not. Twenty-five rules, `V-001` through `V-025`, each raising exactly
one diagnostic code from the language registry (`spec/diagnostics/codes-lang.md`).

Rule identifiers and their codes are frozen in `spec/tables/vocabulary.json`. Freshness,
emission, and context rules are numbered `V-101`+ and live in `docs/10`, `docs/11`, and
`docs/09`.

---

## 1. The diagnostic model

### 1.1 Severity

Two severities, `error` and `warning`. The default profile is `--strict`, in which
warnings are errors and the build fails. `--lenient` downgrades them and exists for one
purpose: `akr import` on legacy material (D-022). A warning that never fails a build is a
warning nobody fixes.

All twenty-four rules below are errors. There are no warning-severity rules in 0.1;
several `AKR-P*` and `AKR-F*` codes are warnings, and they are listed in the registry.

### 1.2 Staleness is not a diagnostic

Stale and at-risk records carry no code, never enter the diagnostic stream, and never
change an exit status (D-024). A project with stale knowledge still builds — building is
how you find out. `akr check --review-clean` is the opt-in gate for projects that want
staleness to fail CI.

### 1.3 Shape

Every diagnostic has a code, a severity, a primary span, and at least one of a note or a
help line. Nothing is emitted without a span.

```
error[AKR-R014]: superseding plan does not dispose of an unfinished child
  --> .akr/records/sys/work.akr:64:1
   |
64 | record sys.work.m3-plan/2 : work {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ supersedes sys.work.m3-plan/1
   |
note: sys.work.m3-audio-pass is `ready` and is part_of the superseded plan
  --> .akr/records/sys/work.akr:112:1
help: add a `disposition @sys.work.m3-audio-pass { outcome ... }` block (V-017)
```

`akr explain AKR-R014` prints the registry entry. Diagnostics are sorted by file path,
then by span, so output is stable across runs and machines.

### 1.4 Collection

Within a stage, all diagnostics are collected and reported together. Between stages, the
build halts: there is no point link-checking a file that did not parse, and cascading
errors from a bad parse are noise. This is why the rules below are grouped by stage — the
grouping is a real execution boundary, not an editorial one.

---

## 2. Rule catalogue

Each rule gives its stage, code, statement, why it exists, a failing example, a passing
example, and the fix.

---

### V-001 — Every reference resolves

**Stage** link · **Code** `AKR-L001` (with `AKR-L002` for a key with no live revision)

**Statement.** Every reference resolves to a declared key, an existing revision, and —
when an anchor is present — an existing anchor.

A floating `@key` resolves even when the key has no live revision: head resolution falls
back to the end of the supersession chain (`docs/04` §3), so completing a milestone does
not break every reference to it. `AKR-L002` is raised only when a key's revisions cannot
be reduced to a single head.

**Why.** A dangling reference is the failure mode Markdown has and cannot fix. If
references can dangle, nothing downstream can be trusted to be complete.

```
# fails
supported_by [ @sim.obs.projection-gap ]     # key is projection-gaps
```

```
# passes
supported_by [ @sim.obs.projection-gaps ]
```

**Fix.** Correct the key, or create the record. `akr search` finds near misses; the
diagnostic suggests the closest existing key by edit distance.

---

### V-002 — Namespaces are declared

**Stage** link · **Code** `AKR-L004`

**Statement.** The first segment of every key must be declared with a `namespace` line in
`project.akr`.

**Why.** The cheapest available defence against typo-drift. Without it, `ledge.viewer.x`
quietly creates a second knowledge graph that nobody notices until two policies disagree
and neither is findable.

```
# fails — `ledge` is not declared
record ledge.decision.renderer-boundary/1 : decision { ... }
```

```
# passes
record lege.decision.renderer-boundary/1 : decision { ... }
```

**Fix.** Correct the namespace, or declare it in `project.akr` if it is genuinely new.
Adding a namespace should be a deliberate, reviewed act.

---

### V-003 — One key, one file

**Stage** link · **Code** `AKR-L006`

**Statement.** Every revision of a key lives in the same source file.

**Why.** Review ergonomics, not semantics. A key's history should be one diff, and
supersession review is impossible when `/1` and `/2` are in different files nobody
opened together.

```
# fails
# records/sys/work.akr      : record sys.work.m3-plan/1 : work { ... }
# records/sys/plans-v2.akr  : record sys.work.m3-plan/2 : work { ... }
```

```
# passes — both revisions in records/sys/work.akr, sorted by revision
```

**Fix.** Move the revisions into one file. Identity does not depend on which one
(D-018), so pick whichever reads better.

---

### V-004 — Anchors exist, and retired anchors say so

**Stage** link · **Code** `AKR-L012`

**Statement.** A reference with an anchor must name a `claim` block in the target
revision, or a `check` block in its `acceptance` block. An anchor listed in the target's
`retired_claims` produces a distinct "retired at revision N" diagnostic.

**Why.** "No such anchor" makes the reader reconstruct what happened. "Retired at
revision 2" tells them, and points at the replacement (D-011).

```
# fails — no-exceptions was dropped in revision 2
supported_by [ @sys.policy.tandem-work#no-exceptions ]
```

```
# passes — pin to the revision that had it, or cite what replaced it
supported_by [ @sys.policy.tandem-work/1#no-exceptions ]
supported_by [ @sys.policy.tandem-work#lag-bound ]
```

**Fix.** Pin the historical reference, or cite the replacement claim.

---

### V-005 — Relation and slot targets are kind-correct

**Stage** link · **Code** `AKR-L031`

**Statement.** Every relation's source and target kinds must be in its declared domain
and range. The same applies to kind-restricted content slots (`exceptions`, `into`, and
`ref` scope terms).

**Why.** A relation's meaning *is* its domain, range, and consequence. `implements`
pointing at an observation has no meaning the resolver can act on.

```
# fails — implements ranges over normative kinds, not observations
implements [ @sim.obs.projection-gaps ]
```

```
# passes
implements [ @sim.req.fixed-timestep ]
supported_by [ @sim.obs.projection-gaps ]
```

**Fix.** Use the relation that fits. The diagnostic prints the relation's domain and
range and names the relations that would accept the target kind.

---

### V-006 — Invalid terminal records are cited, not built on

**Stage** link · **Code** `AKR-L021`

**Statement.** A reference to a record in a terminal state is an error unless the
referring slot is historical, it has an explicit structural exemption, or it is
`depends_on` a completed planning record.

**Why.** Historical relations exist to point backwards. Every other relation asserts
something about how the project currently works, and pointing one at a withdrawn policy
is a mistake worth catching at build time. But completion satisfies a prerequisite;
requiring callers to erase that dependency before completing the predecessor both adds
bookkeeping and destroys the milestone chain.

```
# fails — the weekly-demo policy is withdrawn
depends_on [ @sys.policy.weekly-demo ]
```

```
# passes
derived_from [ @sys.policy.weekly-demo/1 ]
```

**Fix.** Point at the live head, or, if the historical fact is what you meant, express it
with a historical relation and pin the revision.

---

### V-007 — The kind and state combination is legal

**Stage** type · **Code** `AKR-T011`

**Statement.** A record's `state` must belong to its kind's class lifecycle.

**Why.** Twelve kinds share four state machines (D-002). Nonsensical combinations —
`completed` on a policy, `active` on an observation, `needs-review` on anything — are
caught before any graph is built.

```
# fails
record sys.policy.tandem-work/1 : policy {
    state completed
```

```
# passes
record sys.policy.tandem-work/1 : policy {
    state active
```

**Fix.** Use a state from the class machine. The diagnostic names the kind, its class,
and the legal set. Note that `needs-review` is not a state anywhere: staleness is derived
(D-003).

---

### V-008 — Required slots present, unknown slots rejected

**Stage** type · **Code** `AKR-T001` (missing) / `AKR-T002` (unknown)

**Statement.** Every slot required by the kind is present; no slot outside the kind's
declared set appears. Slots that a more specific rule owns are **excluded**: V-009 owns
an observation's `observed_at`, and V-010 owns evidence's `result`, `method` and
`observed_at`. V-008 says nothing about those, so one missing slot raises one code.

**Why.** Rejecting unknown slots is what keeps the vocabulary closed. A tolerated typo
becomes a de-facto extension, and then the schema lives in nobody's head.

The exclusion exists because the overlap was real: before it, a single missing
`observed_at` raised both `AKR-T001` ("observation requires slot `observed_at`") and
`AKR-T021` ("observation requires `observed_at`"). The second says everything the first
does and explains why it matters. A reader gains nothing from the pair, and a fixture
asserting both would be pinning an implementation detail rather than a rule.

```
# fails — `rule` is a policy slot; a decision uses `decision`
record sys.decision.view-generation/1 : decision {
    rule """ ... """
```

```
# passes
record sys.decision.view-generation/1 : decision {
    decision """ ... """
```

**Fix.** Use the kind's slots (`docs/02` §4), or reconsider the kind.

---

### V-009 — Observations carry `observed_at`

**Stage** type · **Code** `AKR-T021`

**Statement.** Every `observation` has an `observed_at` commit.

**Why.** An observation without a commit is a rumour. It also cannot go stale, which
means it will be believed forever.

```
# fails
record sim.obs.projection-gaps/1 : observation {
    statement """ ... """
```

```
# passes
    observed_at git:7c41d0ba92e6f37518a3cd406b5e2f91d8074a63
```

**Fix.** Record the commit the observation was made against. Abbreviated hashes are
rejected at parse (`AKR-P021`).

---

### V-010 — Evidence carries `result`, `method`, `observed_at`

**Stage** type · **Code** `AKR-T022`

**Statement.** Every `evidence` record has all three.

**Why.** Evidence exists to satisfy acceptance checks (D-016). Without a result there is
nothing to satisfy; without a commit the descendant rule cannot be evaluated; without a
method nobody can reproduce it.

```
# fails — no result
record sim.evidence.determinism-suite-pass/1 : evidence {
    method command
    observed_at git:5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285
```

```
# passes
    result pass
    method command
    observed_at git:5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285
    command "cargo test -p sim --test determinism -- --seed-sweep 512"
```

**Fix.** Record all three. `result fail` is a legitimate and valuable record.

---

### V-011 — A resolved question has a resolution and a resolver

**Stage** type · **Code** `AKR-T031`

**Statement.** A `question` in state `resolved` has a `resolution` slot and is the target
of at least one live `resolves` edge.

**Why.** An answer with no link to whatever produced it loses the connection between the
answer and the decision that used it — which is the connection somebody will want in six
months.

```
# fails — resolved, but nothing resolves it and no resolution recorded
record lege.question.text-rendering-owner/1 : question {
    state resolved
    question """ ... """
```

```
# passes
    state resolved
    question """ ... """
    resolution """
        The viewer owns text layout; the simulator emits no glyph data.
        """
# ...and in the decision:
    resolves [ @lege.question.text-rendering-owner ]
```

**Fix.** Add the `resolution`, and add `resolves` to the record that answered it. If
nothing answered it, the state is `closed-without-resolution`.

---

### V-012 — One live revision per key

**Stage** resolve · **Code** `AKR-R001`

**Statement.** At most one revision of a key may be in a live state.

**Why.** The core identity invariant (D-004a). Two live revisions means "the current
policy" has two answers, and every alternative resolution — newest wins, first wins,
alphabetical — silently picks one nobody chose. Newest-wins is an explicit anti-goal.

```
# fails — both active
record sys.policy.tandem-work/1 : policy { state active  ... }
record sys.policy.tandem-work/2 : policy { state active  ... }
```

```
# passes
record sys.policy.tandem-work/1 : policy { state superseded ... }
record sys.policy.tandem-work/2 : policy { state active     ... }
```

**Fix.** `akr supersede sys.policy.tandem-work/1`, or withdraw the one that should not be
live. This error appearing mid-revision is normal and expected — it is the reminder that
the revision is half done.

---

### V-013 — Normative exclusivity by topic and scope

**Stage** resolve · **Code** `AKR-R002`

**Statement.** No two live normative records share a `topic` while their scopes overlap
(D-010 overlap algorithm).

**Why.** The governance half of D-004. Two live policies claiming the same topic over the
same code is a contradiction the compiler *can* detect mechanically, because the author
declared the topic.

```
# fails — same topic, and `all` overlaps everything
record sys.policy.tandem-work/1  : policy { state active  topic tandem-work  scope [ all ] ... }
record sys.policy.tandem-strict/1: policy { state active  topic tandem-work  scope [ path "sim/**" ] ... }
```

```
# passes — supersede one, or give them distinct topics, or make scopes disjoint
record sys.policy.tandem-work/1  : policy { state superseded topic tandem-work ... }
```

**Fix.** Usually one should supersede the other. Occasionally the topics really are
different and one should be renamed. Removing `topic` also silences the rule, which is
legitimate when the records genuinely coexist.

---

### V-014 — The supersession graph is acyclic

**Stage** resolve · **Code** `AKR-R011`

**Statement.** `supersedes` edges form a DAG.

**Why.** A supersession cycle has no head, and every automatic tiebreak picks a winner
nobody chose. Rejected rather than broken.

```
# fails
record x/1 : policy { supersedes [ @x/2 ] ... }
record x/2 : policy { supersedes [ @x/1 ] ... }
```

```
# passes
record x/1 : policy { state superseded ... }
record x/2 : policy { state active  supersedes [ @x/1 ] ... }
```

**Fix.** Decide which record is current and make the chain point one way. The diagnostic
prints the full cycle.

---

### V-015 — Structural relation graphs are acyclic

**Stage** resolve · **Code** `AKR-R012`

**Statement.** The `depends_on`, `derived_from`, `part_of`, `implements`, and `blocks`
graphs are each acyclic.

**Why.** Each of these means "this rests on that". A cycle means a set of records that
justify each other and nothing else, which is either a modelling error or a genuinely
circular argument. Cycles also make staleness propagation non-terminating without a visit
set, and it is better to reject them than to silently tolerate them.

```
# fails
record a/1 : work { depends_on [ @b ] ... }
record b/1 : work { depends_on [ @a ] ... }
```

```
# passes — one direction, or express the mutual constraint as `after`
record a/1 : work { depends_on [ @b ] ... }
record b/1 : work { ... }
```

**Fix.** Break the cycle. Mutual ordering constraints between work items are usually
`after`, and mutual *containment* is usually one item that should be split.

---

### V-016 — The `after` graph is acyclic

**Stage** resolve · **Code** `AKR-R013`

**Statement.** `after` edges among milestones and work items form a DAG.

**Why.** `after` is a hard ordering constraint, and a cycle in it is a plan that cannot be
executed in any order. Separate from V-015 because the diagnostic can be much better: it
prints the cycle as a milestone sequence.

```
# fails
record sys.milestone.m2-deterministic-sim/1 : milestone { after [ @sys.milestone.m3-playable-day ] ... }
record sys.milestone.m3-playable-day/1      : milestone { after [ @sys.milestone.m2-deterministic-sim ] ... }
```

```
# passes
record sys.milestone.m3-playable-day/1 : milestone { after [ @sys.milestone.m2-deterministic-sim ] ... }
```

**Fix.** Remove the back edge. If two milestones genuinely interleave, they are one
milestone, or the shared part is a track.

---

### V-017 — Supersession disposes of unfinished children

**Stage** resolve · **Code** `AKR-R014`

**Statement.** A planning record that supersedes another must carry a `disposition` block
for every record in a live planning state that is `part_of` the superseded record.

**Why.** The most valuable check in the system (D-017). Work silently vanishing across a
replan is what makes long-running agent-driven projects untrustworthy: nobody decided to
drop the audio pass, it just stopped being mentioned.

```
# fails — /1 had two live children, /2 mentions neither
record sys.work.m3-plan/2 : work {
    supersedes [ @sys.work.m3-plan/1 ]
```

```
# passes
    disposition @sys.work.m3-audio-pass {
        outcome intentionally_dropped
        note """ Ambient audio is not part of a playable day. """
    }
    disposition @sys.work.m3-lighting-pass {
        outcome carried_forward
        into @sys.track.lighting
    }
    supersedes [ @sys.work.m3-plan/1 ]
```

**Fix.** Add one block per unfinished child. The diagnostic lists every child needing
one, so the fix is mechanical. `into` is required for `carried_forward` and
`completed_elsewhere` and forbidden for `intentionally_dropped` (`AKR-R015`).

---

### V-018 — One plan of record

**Stage** resolve · **Code** `AKR-R018`

**Statement.** At most one live `work` record may be `plan_of_record` for a given
milestone or track.

**Why.** AKR has no `plan` kind precisely because `plan_of_record` carries that meaning
(D-001); the relation has to be exclusive or it carries nothing. Two live plans for one
milestone is the ambiguity the whole design is against.

```
# fails
record sys.work.m3-plan/2     : work { state active plan_of_record [ @sys.milestone.m3-playable-day ] ... }
record sys.work.m3-plan-alt/1 : work { state active plan_of_record [ @sys.milestone.m3-playable-day ] ... }
```

```
# passes — the alternative is `proposed` until it supersedes the incumbent
record sys.work.m3-plan-alt/1 : work { state proposed ... }
```

**Fix.** `proposed` is a live state, so a proposed alternative also trips this rule —
which is intended. Model an alternative as a revision of the plan, not a second plan.

---

### V-019 — Live records do not rely on invalid terminal records

**Stage** resolve · **Code** `AKR-R021`

**Statement.** A record in a live state must not have an `implements`, `plan_of_record`,
or `supported_by` edge to a record in a terminal state. A `depends_on` edge is also
invalid when its target was abandoned, superseded, rejected, withdrawn, or disproven,
but remains valid when a planning target was completed.

`after`, `part_of`, `blocks`, and `verified_by` are excluded, deliberately. A completed
predecessor is what `after` normally points at; work under a completed milestone is
history rather than an error; a terminal blocker means the blockage lifted (see
`AKR-R023`); and evidence is cited precisely because it was recorded once. `part_of`
pointing at a *superseded plan revision* is governed instead by the disposition
exemption in `docs/04` §5.1. Completed `depends_on` is the prerequisite equivalent: it
records that the required work finished successfully.

**Why.** V-006 catches the syntactic case at link time by looking at the referring slot.
This is the resolved case: a floating `@key` that was fine when written and now points at
something superseded. Without it, a plan can quietly rest on a withdrawn decision.

```
# fails — the head of that decision is now superseded and nothing floats to a live head
implements [ @lege.decision.renderer-boundary/1 ]
```

```
# passes
implements [ @lege.decision.renderer-boundary ]
```

**Fix.** For an invalid terminal target, float the reference to a live head or revise the
dependent record to point at the replacement. Do not remove an edge merely because its
target completed. This rule firing after someone else's supersession is the system working:
it is exactly the "current record at risk" signal the planning notes asked for, made
mandatory rather than advisory for hard dependencies.

---

### V-020 — Completion requires satisfied acceptance

**Stage** resolve · **Code** `AKR-R022`

**Statement.** A `milestone` or `work` record in state `completed` has every `check` in
its `acceptance` block satisfied: at least one referenced evidence record with
`result pass` and an `observed_at` commit descended from the record's last *definitional*
change, or evidence authored in the same commit as that change (D-016, refined by D-029).

**Why.** This is what makes `completed` mean something. Without it the state is a
self-report, and a milestone marked done with a failing check is worse than no milestone
at all.

```
# fails — no-placeholder-assets has no passing evidence
record sys.milestone.m3-playable-day/1 : milestone {
    state completed
```

```
# passes — either the evidence exists, or the milestone is honest about being active
record sys.milestone.m3-playable-day/1 : milestone {
    state active
```

**Fix.** Run the check and record the evidence, or leave the record `active`. `akr
complete` refuses up front; the rule catches a hand-edited state.

The descendant-commit condition is what stops a green run from 200 commits ago closing a
milestone whose acceptance changed yesterday. Editing acceptance invalidates its evidence
— correct, occasionally annoying, worth it. The commit compared against is the last one to
change the record's **definition**, not its lifecycle: per D-029 the `state` slot, each
check's `verified_by`, and the `note` slot are excluded, so `akr complete` writing the
completion is not itself the change that strands the evidence.

The co-commit case is the staged-tree workflow: tests run before the future commit hash
exists, then the evidence and verified record land together. Equal last-change commits
identify that case without allowing evidence from an older commit to verify a later
redefinition.

**A branch is not an age.** Three different facts can leave a check unsatisfied on its
commit, and the message says which. Evidence *predates* the last definitional change only
when that change genuinely descends from it. When the evidence's commit is one the
resolved head does not reach — a rebase, a squashed re-upload, any rewritten history — the
evidence is not old, it is on a branch this one does not contain, and the message says so
and points at the `AKR-G012` warning that reports the same fact directly. Evidence naming
a commit the repository does not have at all points at `AKR-G011`. Lege-ecosystem carries
228 `AKR-G012`s from one re-upload, and every one of its eight `AKR-R022`s was reported as
an age until this separation existed.

The reachability fact is carried separately from the ancestry because the ancestry cannot
answer it. `LedgerFacts::ancestry` is a topological *order* over the commits a ledger
mentions — one history walk rather than a process per comparison — and an order always
puts one of two commits first, so it reads two branches as an age. `LedgerFacts::off_branch`
is filled from one `rev-list --not <head>`, free when every commit is an ancestor. It
changes the wording and never the verdict: a workspace does not newly fail because its
fault can now be named accurately.

**D-028 exemption.** When the record carries at least one `source { kind legacy ... }`
block, the descendant-commit comparison is waived — a historical port's own introduction
commit says nothing about when the work happened. Everything else about the check is
still enforced: the evidence must resolve, it must record `result pass`, and its
`observed_at` commit must still be one the repository has, whenever git facts are
available at all.

---

### V-021 — Active decisions cite something

**Stage** resolve · **Code** `AKR-R031`

**Statement.** A `decision` in state `active` has at least one live `implements`,
`depends_on`, or `supported_by` edge to a `requirement`, `policy`, `constraint`, or
evidence-bearing record.

**Why.** A decision resting on nothing is a preference. The rule exists to make you notice
which one you have — not to forbid preferences, but to make them visible as such.

```
# fails
record sim.decision.timestep-4ms/1 : decision {
    state active
    decision """ The simulator steps at 4 ms. """
```

```
# passes
    implements [ @sim.req.fixed-timestep ]
    depends_on [ @sys.constraint.frame-budget-16ms ]
```

**Fix.** Cite the requirement, policy, or constraint that motivated it, or the evidence
that informed it. If there is genuinely nothing, the record may belong in `proposed`
until there is.

---

### V-022 — Live observations have provenance

**Stage** resolve · **Code** `AKR-R032`

**Statement.** An `observation` in state `verified` has `observed_at` (V-009) and at least
one of: `method`, a `source` block, or a `supported_by` edge to evidence.

**Why.** "Verified" should mean somebody looked, in a way another person could repeat.
The commit says when; this rule says how.

```
# fails — a commit, but no account of how it was determined
record sim.obs.projection-gaps/1 : observation {
    state verified
    observed_at git:7c41d0ba92e6f37518a3cd406b5e2f91d8074a63
```

```
# passes
    method command
```

**Fix.** Record the method, cite the tool output as a `source`, or link the evidence.

---

### V-023 — Contradictions are dispositioned

**Stage** resolve · **Code** `AKR-R041`

**Statement.** Every `contradicts` edge between two live records must be resolved (one
side reaches a terminal state) or explicitly acknowledged (`acknowledged true` on the
declaring record).

**Why.** The compiler cannot detect semantic contradiction in prose and does not try
(D-023). What it can guarantee is that a contradiction somebody noticed is never quietly
lost — which is the part that actually goes wrong.

```
# fails — declared, both live, not acknowledged
record sim.obs.timestep-drift/1 : observation {
    state verified
    contradicts [ @sim.evidence.determinism-suite-pass/1 ]
```

```
# passes
    contradicts [ @sim.evidence.determinism-suite-pass/1 ]
    acknowledged true
```

**Fix.** Resolve it — usually by superseding one side with a newer observation — or
acknowledge it and say why in prose. Acknowledging is not a defeat; "these two disagree
and we know it" is a legitimate and honest ledger state.

---

### V-024 — Sealed revisions match their recorded hash

**Stage** resolve · **Code** `AKR-R051` (with `AKR-R052` for a stale or incomplete lock)

**Statement.** Every revision in a state other than `proposed` has a `seal` entry in
`akr.lock` whose hash matches the SHA-256 of its canonically formatted text, comments
excluded.

**Why.** This is what makes "accepted bodies are immutable — changes need a new revision"
enforceable without a server (D-015). It is enforced by the same commit-and-review
machinery the project already has: editing a sealed record shows up as a lock diff, and a
lock diff is what a reviewer should be reading.

```
# fails — the record is active, and its text changed since it was sealed
error[AKR-R051]: sealed revision modified
note: recorded sha256:9c1f..., computed sha256:4ab0...
```

```
# passes — the change went into a new revision
akr revise sys.policy.tandem-work
```

**Fix.** `akr revise`, move the change into the new revision, and supersede the old one.
Legitimate resealing — after a grammar upgrade changes canonical output — is
`akr lock --reseal`, which produces a diff touching every record and is therefore hard to
do by accident.

Comments are excluded from the hash on purpose: adding a clarifying comment to a sealed
record must not trip this rule, or people stop writing comments.

A legal lifecycle transition can change the canonical text of an existing sealed
revision. When re-rendering the current record with the state stored in `akr.lock`
reproduces the recorded hash, validation reports `AKR-R052`: the record body is intact
and the lock needs rebuilding. A simultaneous body edit still reports `AKR-R051`.

---

### V-025 — An evidence artifact is cited from a durable path

**Stage** type · **Code** `AKR-T023`

**Statement.** No **live** `evidence` revision has an `artifact` under an unkept
`.agent/scratch` entry.

**Why.** Scratch is the one directory the protocol documents as disposable, and
`akr scratch prune` deletes from it on an ordinary handoff (D-036). An artefact cited from
there is a verified claim whose backing a later session removes without knowing it was
cited — and nothing notices, because a path is not a reference the ledger resolves. The
audit that prompted this found 38 records across five files citing scratch paths, most of
them already dangling.

```
# fails — the benchmark output is in the directory `akr scratch prune` empties
record sys.evidence.ocr-benchmark/1 : evidence {
    result pass
    method command
    observed_at git:5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285
    artifact ".agent/scratch/ocr-benchmark/out.txt"
```

```
# passes
    artifact "docs/benchmarks/2026-08-22.txt"
```

An artifact under `.agent/scratch/<entry>/...` also passes when `<entry>` is named in
`.agent/scratch/KEEP`, as written by `akr scratch keep <entry> --reason <why>`.

**Fix.** Move the artefact somewhere durable and cite that path, or `akr scratch keep
<entry>` first and cite it where it is. Once the entry has already been pruned neither is
possible, and the repair is to drop the `artifact` slot: `command`, `summary` and
`observed_at` carry the claim without it. The help line names all three, because a help
line offering only the impossible ones is what sent one reader looking for an escape that
was not there.

This is a rule rather than a flag check on `akr evidence add` because evidence reaches the
ledger by four routes — `akr evidence add`, `akr evidence add-many --from`, `akr propose
--kind evidence --from`, and `knowledge.propose` over MCP — and a guard on the first leaves
the other three open. Every write validates the resulting ledger before writing anything
(`docs/07` §4), so the rule still refuses at write time, which is the point: a warning from
a later build arrives after the record is in the ledger, which is when nobody goes back and
moves the file. What the rule does *not* do is re-judge records nobody is touching: a write
answers for the diagnostics it introduces, not for the ones it inherits (D-039). Evidence
is born `verified` at revision 1, so a citation made today is always a new subject and is
always refused.

D-036 says scratch is never a ledger diagnostic. That is about the directory's contents —
how much is sitting there and how old it is, which `akr check` reports as a build fact.
This is about a record, and the record is wrong whether or not the file behind it still
exists.

The comparison is textual and normalises separators, a leading `./`, and an absolute path
that passes through a scratch directory, because those all name the same place. It reads
the `artifact` slot only: a papercut whose statement quotes a scratch path is describing
the problem, not committing it.

**Live revisions only**, as V-023 judges contradictions. A sealed revision is a fact of
history rather than a live claim, and judging one made this the single rule the sanctioned
write path could not satisfy: superseding the offending record leaves its old revision in
the file, so the repair reproduced the diagnostic it was repairing. The repair is
therefore what it should be — revise the evidence to cite a durable path, and the previous
revision seals as `superseded` and stops being read. That also matters for a citation
whose scratch entry has already been pruned, which `akr scratch keep` cannot rescue
because there is nothing left to keep.

---

## 3. Rule-to-stage matrix

| Stage | Rules | Codes | What it can see |
| --- | --- | --- | --- |
| **A — parse** | — | `AKR-P*`, `AKR-F*` | One file's bytes |
| **B — type** | V-007, V-008, V-009, V-010, V-011, V-025 | `AKR-T001`, `T002`, `T011`, `T021`, `T022`, `T023`, `T031` | One record, plus the vocabulary |
| **C — link** | V-001, V-002, V-003, V-004, V-005, V-006 | `AKR-L001`, `L002`, `L004`, `L006`, `L012`, `L021`, `L031` | All records, unresolved |
| **D — resolve** | V-012 … V-024 | `AKR-R001`, `R002`, `R011`–`R015`, `R018`, `R021`, `R022`, `R031`, `R032`, `R041`, `R051`, `R052` | The whole graph, heads resolved, git history, the lock |

The grouping is an execution boundary: a stage runs only if the previous one produced no
errors, and within a stage all diagnostics are collected. This is why V-011 (a
single-record property) is at type-check while V-011's companion requirement — that a
live `resolves` edge exists — is evaluated at resolve and reported under the same code.

---

## 4. Deliberately not validated

What the compiler does not check, and why. Every item here was considered.

**Prose quality.** No length limits, no readability scoring, no required sections. A rule
that rejects a short `statement` produces padding, not clarity.

**Semantic duplication.** Two records saying the same thing in different words is not
detected. Detecting it requires judgement (D-020), and the mechanical approximation —
declare a shared `topic` — is available to anyone who wants it (V-013).

**Whether a claim is true.** AKR records who claimed what, when, on what basis, and
whether anything has changed since. It never asserts that a claim is correct. The
strongest thing the system says is "this is current, sourced, and consistent with
everything else recorded", which is a real guarantee and not that one.

**Taste in modelling.** Nothing stops you from writing a requirement that should be a
decision, or a milestone with one meaningless check. `docs/02` §3.1 and §4 give the
distinctions; the compiler does not enforce them because the failure mode of a wrong
guess is worse than the failure mode of a wrong record.

**Scope correctness.** That a record's `scope` actually covers the code it talks about is
unverifiable — the paths may not exist yet. Scope is checked for *form* (V-005 on `ref`
terms), never for accuracy.

**Estimates and dates.** `target` is never enforced. A missed date is information for a
human, not a build failure.

**Cross-project consistency.** There is no cross-project reference in 0.1 (`docs/02`
§12).

**Whether evidence was honestly recorded.** A person or agent can write `result pass` for
a check that never ran. The `command` and `artifact` slots make that reproducible enough
to catch in review, which is the correct place to catch it.

---

## 5. See also

- `spec/diagnostics/codes-lang.md` — the full `P`/`F`/`T`/`L`/`R` registry.
- `spec/diagnostics/README.md` — the code scheme and severity model.
- `fixtures/validate/` — one failing fixture per rule, with its expected diagnostic.
- `docs/02-data-model.md` — what the rules are protecting.
