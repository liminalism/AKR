# References, Revisions, and Versioning

How a record is identified, how it changes, which version a reference means, and what
`akr.lock` records so that a build is reproducible. This document is the semantics; the
lock file's concrete format is `spec/schema/akr-lock.md`.

---

## 1. Identity

A record is identified by its **key**: a dotted path of two to eight lowercase segments,
whose first segment is a declared namespace.

```
sys.policy.tandem-work
lege.decision.renderer-boundary
```

Four properties, all load-bearing:

**A key never changes.** Renaming a key is not an operation. If the name was wrong, write
a new key and supersede the old one — that leaves a trail, which is the point. A rename
would silently invalidate every reference in every historical record.

**A key is never reused.** Once a key has been used, it is used forever, including after
its head is withdrawn. Reuse would make `@key` in an old record resolve to something
unrelated, which is worse than a dangling reference.

**A key is never derived from a filename.** Files are containers (D-018). Moving
`sys/policies.akr` to `sys/governance.akr` changes nothing, and neither does splitting
it. The one file-related rule — all revisions of a key live in one file (V-003) — is for
review ergonomics, not identity.

**A key is not a line number.** The planning notes list line-number citations as an
anti-goal, and this is the reason: a citation that breaks when someone adds a paragraph
above it is not a citation.

### 1.1 Naming keys

The convention across all three namespaces of the worked example:

```
<namespace>.<kind-hint>.<subject>
sys.policy.tandem-work
sim.obs.projection-gaps
lege.decision.renderer-boundary
```

The middle segment is a hint, not a declaration — the kind is on the record, and nothing
checks that `sim.obs.x` is an `observation`. It exists because keys appear in prose,
transcripts, and CI logs far from their records, and a reader benefits from knowing
whether they are looking at a policy or an observation before they look it up.

Deeper keys are for genuinely nested subjects: `lege.viewer.renderer-boundary` when
`lege.viewer` is a real component. Depth as decoration is how a key becomes a directory
path, which is what keys exist to avoid.

---

## 2. Revisions

A **revision** is a numbered version of a key, written `key/N`, starting at 1 and
incrementing by one. The pair (key, revision) is a **revision identifier** and is what
every reference ultimately resolves to.

```
record sys.work.m3-plan/1 : work { state superseded  ... }
record sys.work.m3-plan/2 : work { state active      ... }
```

Both live in the same file (V-003), sorted by revision (`docs/03` §6.2), so a key's
history reads top to bottom in one diff.

### 2.1 Creating a revision

`akr revise <key>` copies the current head, increments the number, sets the copy's state
to `proposed`, adds `supersedes [ @key/N-1 ]`, **and moves revision N-1 to
`superseded` in the same write**.

An explicit `--state` is applied to the successor instead of that `proposed` default.
Omitting it during a content revision is deliberate re-acceptance: changed settled
knowledge is proposed again rather than silently remaining binding.

Retiring the old revision is not optional and cannot be deferred. Leaving both live is two
live heads, which V-012 rejects, and `docs/07` §4 refuses to write a ledger that does not
validate — so a `revise` that left the old head live could never write at all. The two
halves are one act.

It follows that revising a *sealed* head is superseding it, and the tool treats it as
such: for planning records it demands a disposition for every unfinished child before it
will write, exactly as `akr supersede` does. Otherwise `akr revise` would be a way to
replace a plan without accounting for its children, which is the hole D-017 exists to
close.

A `proposed` head is different: it is unsealed (D-015), so `akr revise` edits it **in
place** rather than creating revision N+1. Creating revision 2 of a proposal nobody has
accepted would be noise.

The normal sequence:

```
akr revise sys.work.m3-plan \
    --disposition sys.work.m3-audio-pass=intentionally_dropped
                                     # /2 appears proposed, /1 becomes superseded
$EDITOR .akr/records/sys/work.akr    # refine the new plan
akr check                            # one live head throughout
akr build                            # the lock catches up (§8.3)
```

### 2.2 Sealing

A revision in any state other than `proposed` is **sealed** (D-015). Its content hash —
SHA-256 over its canonically formatted text — is recorded in `akr.lock`. `akr check`
recomputes every sealed revision's hash and fails with `AKR-R051` on mismatch:

```
error[AKR-R051]: sealed revision modified
  --> .akr/records/sys/policies.akr:12:1
   |
12 | record sys.policy.tandem-work/1 : policy {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this revision is active and sealed
   |
note: recorded hash sha256:9c1f..., computed sha256:4ab0...
help: create a new revision with `akr revise sys.policy.tandem-work`
```

This is what turns "accepted bodies are immutable" from a convention into a rule. It
needs no server, no signatures, and no central authority: changing a sealed record shows
up as a lock diff, and a lock diff is exactly what a reviewer should be looking at.

`proposed` revisions are unsealed and freely editable. That is what makes `proposed`
worth having — a proposal you cannot iterate on is not a proposal.

Two escape hatches, both deliberate and both noisy:

- `akr fmt` may rewrite a sealed record's *formatting*. Since the hash is over the
  canonical form, reformatting cannot change it. If it does, the file was not canonical
  before, and the mismatch is real information.
- `akr lock --reseal` recomputes sealed hashes. It exists for legitimate cases — a
  grammar upgrade that changes canonical output — and it produces a lock diff touching
  every record, which no reviewer will wave through by accident.

---

## 3. Head resolution

At any commit, a key has at most one **head**: its single live revision (D-004a).

```
resolve_head(key):
    revisions = all revisions of key, across all source files
    live      = [ r for r in revisions if r.state in live_states(class_of(r.kind)) ]

    if len(live) >  1: raise AKR-R001(key, live)
    if len(live) == 1: return live[0]

    # No live revision: the key was completed, withdrawn, rejected, or abandoned.
    # The head is the end of the supersession chain — the revision nothing replaced.
    chain_ends = [ r for r in revisions if no revision supersedes r ]
    if len(chain_ends) != 1: raise AKR-R001(key, chain_ends)
    return chain_ends[0]
```

The second tier matters more than it looks. A completed milestone has no live
revision, and `after [ @sys.milestone.m2-deterministic-sim ]` must still resolve —
finishing M2 cannot break every reference to it. So `@key` always resolves as long as
the key exists; whether the *result* is live is a separate question, asked by V-019 of
the relations for which it matters (§5).

`resolve_head` is a pure function of the record set. Its result does not depend on file
order, directory traversal, or filesystem case sensitivity.

Three things this deliberately is not:

**Not newest-wins.** Two live revisions is an error, not a tiebreak. A project where the
highest revision number silently wins is a project where an abandoned draft can quietly
become policy. Rejected as an anti-goal in the planning notes, and enforced here.

**Not per-file.** Heads are computed across the whole ledger. A key split across two
files is caught by V-003 first, but the head rule does not depend on that.

**Not liveness-gated.** A key with no live revision still has a head (the second tier
above). Resolution answers "which revision does this name mean"; liveness is a separate
property that the rules consult where it matters.

### 3.1 Following a supersession chain

Two walks, both used by the CLI:

```
current(revision):                      # forward: what replaced this?
    seen = {}
    while true:
        successor = the revision whose `supersedes` includes this one
        if none: return revision
        if successor in seen: raise AKR-R011   # cycle
        seen.add(successor); revision = successor

history(revision):                      # backward: why is this the way it is?
    chain = [revision]
    while revision.supersedes is non-empty:
        revision = target of revision.supersedes
        chain.append(revision)
    return chain
```

`akr why-current <key>` prints the backward walk with each revision's title, state, and
the reason recorded in the superseding record. It is the answer to "why does this say
what it says", and it is why superseded records are kept rather than deleted.

---

## 4. Reference modes

Four forms, no others (D-009):

| Form | Mode | Resolves to |
| --- | --- | --- |
| `@key` | **current head** | Whichever revision is live at build time |
| `@key/2` | **pinned** | Revision 2, always |
| `@key#anchor` | current head, anchor | The named claim or check in the head |
| `@key/2#anchor` | pinned, anchor | The named claim or check in revision 2 |

There is no `@key/latest`, no revision range, no wildcard, and no cross-project
reference. Anything more expressive turns reference resolution into a query language and
the lock into a query cache.

Every current-head resolution a build performs is written to `akr.lock` (§7), so a build
is reproducible from (sources, lock) alone, and a reviewer can see when a floating
reference started pointing somewhere new.

### 4.1 When to pin and when to float

Not enforced by any rule — this is judgement, and the guidance is short enough to
remember.

**Float (`@key`) when you mean "whatever the current rule is".** A work item
`implements @sys.policy.tandem-work` should follow that policy as it is revised; pinning
it to `/1` would mean the work item implements a policy nobody follows any more.

**Pin (`@key/2`) when you mean "this specific thing".** Four cases:

1. **Evidence and provenance.** `derived_from [ @lege.obs.viewer-imports-engine/1 ]` —
   the decision was derived from that observation, not from whatever replaces it.
2. **Supersession.** `supersedes [ @sys.work.m3-plan/1 ]` is always pinned; superseding
   "the current head" is meaningless. **This one is enforced (D-044):** an unversioned
   `supersedes` is `AKR-L021`. It is the one case where the guidance below was not merely
   ignored but silently undone by the reference following its target.
3. **Narrating history.** Anything in a `context` or `consequences` slot that describes
   what was true at the time.
4. **Contradiction.** `contradicts` names a specific claim that conflicts, and the
   conflict may be resolved by a later revision — which you want to see as a change,
   not have silently applied.

The general shape: **normative references float, historical references pin.** If the
sentence is "we follow X", float. If it is "we did this because X said Y", pin.

**"Not enforced" governs the choice, not the use.** D-009 defines what a reference
*means* — `@key` is the head at build time, `@key/2` is that revision for good — and
leaves the choice between them to judgement, which is this section. V-006 governs where a
reference may be *used*: a live record may not pin a terminal target, because that is
building on something that has been retired. The two do not conflict, and the reason is
worth stating because reading them together suggests they do. Nothing forces you to pin a
`supersedes` edge; if you pin one, the target being terminal is then expected rather than
a fault, which is why `supersedes` and the other historical relations are exempt from
V-006 outright.

**Case 2 was guidance and case 2 was also a trap, which is why it is now enforced.**
`supersedes` naming a key with no revision meant "the current head", so it re-aimed as its
target gained revisions. On 2026-09-08 a LegeOS decision was found carrying `supersedes
[ @legeos.decision.toolchain-licence-namespace ]`, written when that key was at an earlier
revision and by then naming revision 3, which was `active` — the ledger asserting that one
live decision had replaced another live decision, with nobody having edited either. Worse,
a cross-key `supersedes` edge does not retire anything in *either* form: within one key
supersession is the revision chain and head resolution computes it, but across keys the
edge is documentation and `akr supersede` is what moves the target's state. V-006 now
refuses the unversioned form outright (D-044) and separately reports a `supersedes` edge
whose cross-key target is still live. The two are distinct: the refusal stops an edge
re-aiming later, the warning catches one that was inert from the day it was written.

The same argument applies to `contradicts` and `derived_from`, which are historical
relations by the same classification, and the migration does not: fifteen workspaces held
nine unversioned `supersedes` edges and **510 unversioned `derived_from`** ones. That is
its own piece of work with its own plan, and case 1 above already states the rule those
510 break.

### 4.2 Anchors

An anchor names a `claim` block in the target record, or a `check` block inside its
`acceptance` block:

```
supported_by [ @sys.assessment.projection-gaps#coverage-gap ]
verified_by  [ @sys.milestone.m3-playable-day#full-day-demo ]
```

Anchors make citation precise: "supported by that assessment" is weaker than "supported
by exactly this sentence of that assessment", and the difference matters when the
assessment is revised and only one of its claims changes.

Claims are versioned with their record and are not independently versioned (D-011). A
revision that drops an anchor its predecessor had must list it in `retired_claims`:

```
record sys.policy.tandem-work/2 : policy {
    ...
    retired_claims [ no-exceptions ]
}
```

A reference to a retired anchor then produces a specific diagnostic rather than a generic
one:

```
error[AKR-L012]: claim anchor retired
  --> .akr/records/sim/assessments.akr:31:19
   |
31 |     supported_by [ @sys.policy.tandem-work#no-exceptions ]
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ retired at revision 2
   |
help: pin to @sys.policy.tandem-work/1#no-exceptions to cite the historical claim,
      or cite @sys.policy.tandem-work#lag-bound, which replaced it
```

That message is the whole reason `retired_claims` exists. Without it the compiler could
only say "no such anchor", and the reader would have to reconstruct what happened.

---

## 5. Historical access

Live records are for building on; terminal records are for citing, except that a
completed planning record remains a satisfied prerequisite. The rule (V-006,
`AKR-L021`) is therefore:

> A reference whose target is in a terminal state is an error, **unless** the referring
> slot is historical or it is `depends_on` a completed planning record.

The historical relations exist precisely to point backwards. Other relations mean "this
is part of how things currently work", and pointing one at a withdrawn policy or an
abandoned work item is a mistake worth catching. Completion differs from abandonment:
it means the prerequisite was satisfied, and removing the edge would discard useful
provenance.

| Situation | Legal? |
| --- | --- |
| `supersedes [ @key/1 ]` where `/1` is superseded | yes — that is what supersession is |
| `derived_from [ @lege.obs.viewer-imports-engine/1 ]` where the observation is disproven | yes — the derivation is a historical fact |
| `contradicts [ @sim.evidence.x/1 ]` where the evidence is withdrawn | yes — the contradiction happened |
| `depends_on [ @sys.policy.weekly-demo ]` where the policy is withdrawn | **no** — `AKR-L021` |
| `depends_on [ @sys.milestone.m2-deterministic-sim ]` where M2 is completed | **yes** — the prerequisite was satisfied |
| `implements [ @lege.decision.renderer-boundary/1 ]` where `/1` is superseded | **no** — implement the head, or pin deliberately and explain in prose |
| `after [ @sys.milestone.m2-deterministic-sim ]` where M2 is completed | **yes** — a finished predecessor is the normal case |
| `part_of [ @sys.milestone.m3-playable-day ]` where the milestone is completed | **yes** — work under a finished milestone is history, not an error |
| `part_of [ @sys.work.m3-plan/1 ]` where `/1` is superseded | **yes, if dispositioned** — see below |

### 5.1 The `part_of` exemption

`part_of` is the one structural relation that may point at a superseded planning record,
and only under a condition: the referring record must be dispositioned by whatever
superseded the target.

```
record sys.work.m3-lighting-pass/1 : work {
    state ready
    part_of [ @sys.work.m3-plan/1 ]      # /1 is superseded
}
```

is legal precisely because `sys.work.m3-plan/2` carries
`disposition @sys.work.m3-lighting-pass { outcome carried_forward ... }`. Without that
block it is `AKR-L021`.

This is what makes V-017 bite rather than being vacuous. `part_of` **pins** to a plan
revision — the one structural exception to the float-normative/pin-historical guidance in
§4.1 — so that "the children of plan revision 1" is a well-defined set at the moment the
plan is superseded. A child retained by the new plan is repointed at it (or was created
under it); a child that is not retained keeps pointing at the old revision and must be
accounted for. There is no way to drop a child silently: it is either under the new plan,
or it is named in a disposition.

Children of a *milestone* rather than a plan float normally: `part_of [ @sys.milestone.m3-playable-day ]`
means "this belongs to M3", which does not change when a plan is revised.

Archived records (files under `.akr/archive/`) still resolve. Archiving is a filesystem
convention with exactly one semantic effect: archived records are excluded from ordinary
context assembly and from every generated view except `DECISION-HISTORY.md`. State, not
location, is what makes a record terminal.

---

## 6. Supersession and disposition

### 6.1 Chains

`supersedes` forms a directed acyclic graph, checked at resolve (V-014, `AKR-R011`).
Usually it is a simple chain within one key. It may cross keys when one record genuinely
replaces another under a different name; the target must be the same kind.

Cycles are rejected rather than broken, because every automatic tiebreak would pick a
winner nobody chose.

### 6.2 Disposition

A planning record that supersedes another with unfinished children must say what happened
to each one (D-017, V-017, `AKR-R014`). An **unfinished child** is any record in a live
planning state related to the superseded record by `part_of`.

Worked example. Revision 1 of the M3 plan had four children:

| Child | State at supersession | Disposition in `/2` |
| --- | --- | --- |
| `sys.work.m3-renderer-boundary` | `completed` | none required — finished |
| `sys.work.m3-sim-step` | `active` | carried forward into the new plan implicitly (still `part_of` it) |
| `sys.work.m3-lighting-pass` | `ready` | `carried_forward` into `@sys.track.lighting` |
| `sys.work.m3-audio-pass` | `ready` | `intentionally_dropped` |

and revision 2 records the two that left:

```
record sys.work.m3-plan/2 : work {
    title "M3 plan of record"
    state active
    intent """
        Land the renderer boundary first, then the sim step rewrite, then the asset
        audit. Lighting moves to the standing track; ambient audio is dropped.
        """
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
    plan_of_record [ @sys.milestone.m3-playable-day ]
    supersedes [ @sys.work.m3-plan/1 ]
}
```

| `outcome` | `into` | Meaning |
| --- | --- | --- |
| `carried_forward` | required | Still to be done, under the target named by `into`. |
| `completed_elsewhere` | required | Already done, by the work named by `into`. |
| `intentionally_dropped` | forbidden | Decided against. `note` should say why. |
| `still_required_separately` | optional | Still needed, outside any current plan. The honest answer when there is no home for it yet. |

What the rule does not do: it does not change the child's state. `sys.work.m3-audio-pass`
stays `ready` until someone abandons it. The disposition records the decision; the state
change is a separate write, and the worked example deliberately sits between the two,
because that is the state a reviewer meets in practice.

Why this rule earns its cost: work silently vanishing across a replan is the failure that
makes long-running agent-driven projects untrustworthy. Nobody decided to drop the audio
pass — it just stopped being mentioned, and six weeks later nobody can say whether that
was a decision or an accident. One block, written at the moment the author knows the
answer, converts "we must have decided that" into a sentence with a name on it.

---

## 7. Retiring a record: which state?

Four ways for a record to stop being live, and choosing between them is the most common
modelling question in practice.

| State | Use when | Replacement exists? | Was it ever in force? |
| --- | --- | --- | --- |
| `superseded` | Something newer replaces it | **yes**, and it must declare `supersedes` | yes |
| `withdrawn` | It no longer applies and nothing replaces it | no | yes |
| `rejected` | It was proposed and declined | no | **no** |
| `abandoned` (planning only) | The work is not happening | no | it was planned, not delivered |

Worked distinctions:

- The tandem-work policy gets an exceptions list → **superseded** by `/2`.
- The weekly demo stops being a practice and nothing replaces it → **withdrawn**.
- A proposed 4 ms timestep is considered and declined → **rejected**. Kept, because "we
  said no to that, for these reasons" is knowledge, and the alternative is relitigating
  it every quarter.
- The audio pass is dropped from the plan → **abandoned**, once someone acts on the
  disposition.

The one to resist is deleting the record. Deletion is not an operation in AKR: the
planning notes list auto-deletion as an anti-goal, and manual deletion has the same
effect with a human to blame. A terminal record costs a few lines and answers a question
somebody will otherwise ask twice.

---

## 8. `akr.lock`

### 8.1 What it is for

The lock file exists so that **the same sources plus the same lock produce the same
build**, and so that a floating reference changing its target is visible in review rather
than invisible in behaviour.

It records four things (D-014):

1. **Build inputs** — tool version, grammar version, resolved commit, source-graph hash.
2. **Source files** — path and content hash of every `.akr` file.
3. **Resolutions** — every current-head reference the build resolved, and what it
   resolved to.
4. **Seals** — the content hash of every sealed revision (§2.2).

It is written in AKR syntax with the header `akr-lock 0.1` (D-014). One grammar, one
formatter, one determinism story, and a generated file a reviewer must read during
supersession review is not in a second syntax. The format is specified in
`spec/schema/akr-lock.md`.

### 8.2 It is committed

The lock is checked into the repository. Not committing it would mean CI could not
detect a floating reference silently repointing, which is the main thing it is for.

### 8.3 When it changes, and what a diff means

| Lock diff | What happened | Reviewer question |
| --- | --- | --- |
| A `source` hash changed | A file was edited | Normal; read the record diff. |
| A `resolution` target changed | A floating `@key` now points at a new head | **Look at this.** Did the referring record intend to follow that change? |
| A `seal` appeared | A revision left `proposed` | Was it actually reviewed? |
| A `seal` hash changed, and only the `state` slot differs | A lifecycle transition: `supersede`, `complete`, `abandon` | **Expected.** A record's state is part of its canonical text, so advancing it changes the hash. The tool rebuilds the seal on the next `akr build`; until then `akr check` reports `AKR-R052`. |
| A `seal` hash changed, and the body differs | A sealed record was edited | Almost always wrong — should have been a new revision (`AKR-R051`). |
| Every seal changed at once | `akr lock --reseal`, usually a grammar upgrade | Check the tool version line changed too. |

The two seal rows are worth telling apart, because they look identical in a bare hash
diff and mean opposite things. A lifecycle transition is the tool doing its job: D-015
seals a record's *body*, and moving `active` to `superseded` does not change what the
record says. An edit to the body is the failure D-015 exists to catch. The state slot is
the whole difference, and a reviewer should read it before anything else in the diff.

The resolution row is the one that pays for the file. A policy revision that quietly changes
what fifteen work items implement should be a fifteen-line lock diff, not a silent
behaviour change.

### 8.4 Merge conflicts

The lock conflicts whenever two branches both add records or both change resolutions —
which is often. The resolution procedure is mechanical and should be in the project's
`AGENTS.md`:

```
git checkout --ours .akr/akr.lock     # or --theirs; the content does not matter
akr build                              # regenerates the lock from sources
git add .akr/akr.lock
```

The lock is derived, so any conflict is resolved by regenerating it. What must **not** be
done is hand-merging the two versions: a hand-merged lock can contain a resolution that
no build ever produced, which defeats the point. `akr check` fails on a lock that does
not match the sources (`AKR-R052`), so a bad merge is caught rather than shipped.

---

## 9. See also

- `spec/schema/akr-lock.md` — the lock file format, field by field, with a worked
  example.
- `docs/02-data-model.md` §8 — supersession and disposition in the model.
- `docs/05-validation-rules.md` — V-001 through V-006 (linking), V-012 through V-019
  (resolution), V-024 (sealing).
- `docs/07-cli.md` — `akr revise`, `supersede`, `why-current`, `lock`.
