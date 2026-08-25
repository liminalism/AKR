# 10 — Freshness and Git

How AKR knows that something it was told six weeks ago may no longer be true, without
ever deciding that it is false.

Normative for `observed_at` semantics, watch-glob matching, `review_after`, the staleness
derivation, reverse propagation, `akr impact --git-diff`, review-queue ordering, and
rules V-101–V-104.

---

## 1. The problem freshness solves

The Markdown pile's worst property is that a sentence written four hundred commits ago
looks exactly like one written this morning. Everything else AKR does — types, states,
relations, acceptance — is about *what* is claimed. Freshness is about *when*, and it is
the mechanism that turns a ledger from an archive into something worth reading.

The design has one hard constraint and one hard prohibition.

**The constraint:** freshness must be *derived*, not authored. If a record could be
marked stale by hand, the mark would itself go stale, and the build would have to write
its own inputs to keep it current (D-003).

**The prohibition:** the compiler never declares a record false. Staleness is a question
raised — *this observation was made before the code it watches changed; someone should
look* — and never an answer given. A stale record keeps its state, its content, its
claims, and its standing until a human or an agent changes them with an explicit write.

## 2. The three inputs

### `observed_at`

A `commit` value, required on `observation` and `evidence`, optional on `assessment`
(where the slot is `as_of`). It states the commit at which the observation was made.

```
observed_at git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64
```

Exactly 40 lowercase hex digits after `git:`; abbreviations are rejected at parse time
(D-008). A commit that is not in the repository is `AKR-G011` (V-101) — usually a rebase
or a force-push, and always something a reader needs to know before trusting the record.
A commit that exists but is not an ancestor of HEAD is `AKR-G012`, a warning: the
observation was made on a branch this one does not contain, so its freshness is not
computable and it is reported rather than guessed at.

`observed_at` is also what the acceptance rule of D-016 tests against: a check is
satisfied only by evidence whose `observed_at` **descends from** the last commit that
changed the verified record's **definition** (D-029: the canonical record minus the
`state` slot, each check's `verified_by`, and the `note` slot, so completing the record is
not itself that change; D-038 also minus the revision number and a `supersedes` naming an
earlier revision of the same key, so *revising* the record is not itself that change
either, and the history walk continues past the commit that introduced a revision into the
history of the revision it was revised from) — unless the verified record carries a
`legacy` source, in which case D-028 waives the comparison entirely. The evidence's `observed_at` commit still has
to exist in the repository; only the descendancy comparison is skipped.

### `watches`

An array of repo-root-relative globs, on `observation`. It answers the question no
Markdown document can: *what change to the code would make this wrong?*

```
watches [ "sim/src/project/**", "sim/src/step.rs" ]
```

The glob subset is fixed by D-008: `/` separators, `*` and `?` matching within one path
segment, `**` matching any run of segments, `[a-z0-9]` character classes. No brace
expansion, no `!` negation, no backslashes. A glob outside the subset is `AKR-G021`
(V-102).

Matching is literal path matching against the paths a commit touched — added, modified,
deleted, or renamed at either end. A glob that matches no path at HEAD is `AKR-G022`, a
warning, because a watch that can never fire is silent rot: the record looks guarded and
is not.

Path scope uses the same glob subset and tracked-path check. An unmatched scope glob is
`AKR-G023`, also a warning: it usually means a copied rendered `path ` prefix, a typo, or
moved code. Unlike an invalid glob it remains non-fatal under strict validation, so the
diagnostic guides repair without preventing active work from being recorded. A path or
glob intentionally covered by `.gitignore` is exempt: scope may legitimately govern a
disposable cache or generated target that cannot appear in the committed tree.

For efficiency the compiler precomputes each glob's **literal prefix** — the portion
before the first wildcard, `sim/src/project/` above — which is stored in the index and
used to reject non-matching paths without running the matcher
([`../spec/schema/index.sql`](../spec/schema/index.sql), `watches.prefix`).

### `review_after`

A bare date, on `observation`. It answers the other question: *when should someone look
at this again regardless of whether the code moved?*

```
review_after 2026-12-01
```

It exists because not everything that ages is tracked by a file. A benchmark on hardware
that will be replaced, an assumption about a third-party service, an observation about
team practice — none of them has a watchable path, and all of them decay.

`.akr/project.akr` may set `defaults { review_after_days 90 }`, which the write commands
use to fill the slot when the author does not. A `review_after` earlier than the record's
`created_at` is `AKR-G031` (V-103), a warning, because it means the record was stale the
moment it was written.

## 3. The staleness computation

Run in pipeline stage D, after every validation rule has passed
([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §6, check 11). Inputs: the resolved
model, the repository, `HEAD` (or `--at`), and today's date (or `--today`).

**A record is stale if it is empirical, live, and either condition holds:**

> **(a) Watched path moved.** Some commit reachable from `HEAD` but not from the record's
> `observed_at` touched a path matching one of its `watches` globs.
>
> **(b) Review date passed.** Its `review_after` date is earlier than today.

Nothing else makes a record stale. Normative and planning records are never stale in
their own right — they have no `observed_at` and no claim to describe reality at a point
in history. They become `at_risk` by propagation (§4), which is a different flag with a
different meaning.

Terminal records are not evaluated at all. A `disproven` observation has already been
answered; asking whether it is current is meaningless.

**Procedure.**

```
1.  R  := live empirical revisions, in key order
2.  W  := union of all watch globs across R
3.  P  := paths touched by commits in (oldest observed_at in R, HEAD]  [ONE git query]
4.  for each r in R:
5.      cause := none
6.      for each glob g in r.watches, in authored order:
7.          for each (commit c, path p) in P, in commit-then-path order,
                     where c is not reachable from r.observed_at:
8.              if match(g, p):
9.                  cause := watch, detail = (g, c, p)
10.                 break out of 6
11.     if cause is none and r.review_after is not null
                        and r.review_after < today:
12.         cause := review_after, detail = the date
13.     if cause is not none: mark r stale with cause
```

**When both conditions hold, the watch cause is reported.** A moved path names the change
to go and look at; a passed date only says the record is old. Reporting the date instead
would replace the actionable answer with the vaguer one. The record is stale either way —
this decides only what the queue tells you about it.

**A record whose `observed_at` is not in the repository is not evaluated.** Its freshness
is not computable, and guessing in either direction would be worse than silence.
`AKR-G011` (V-101) reports the stranded commit, and the record appears in neither the
stale set nor the fresh one. One unanswerable record never aborts the rest of the queue.

Step 3 is why the stage is fast. The commit-range path query is issued **once**, for the
union of every watch glob, rather than once per record; per-record work is then a set
intersection. That turns O(records × history) into O(history + records × globs)
([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §12).

Determinism: the traversal order is key order, the git queries are by commit id, and the
only clock reading is `today`, which is an explicit parameter. Two runs at the same
commit on the same date produce the same flags.

**Staleness is a build fact, not a diagnostic.** It carries no `AKR-*` code, never enters
the diagnostic stream, and never changes an exit status (D-024). A project with stale
knowledge still builds — that is the point, since building is how you find out.

## 4. Reverse propagation

A stale record casts doubt on whatever rests on it. Propagation is how far that doubt
travels.

**From a stale record, doubt propagates to dependents along exactly three relations:**

| Relation | Reading |
| --- | --- |
| `supported_by` | "my standing rests on this" |
| `depends_on` | "my correctness rests on this" |
| `derived_from` | "I was produced from this" |

And along no others. Not `part_of` — a milestone is not endangered by one stale
observation under it. Not `after` — sequence is not dependence. Not `implements`,
`resolves`, `blocks`, `contradicts`, `supersedes`, `verified_by` as a general
propagator, or `plan_of_record`. Propagating along containment or ordering would flag
half the project every time a file changed, and a warning that always fires is not a
warning.

Dependents are marked **`at_risk`**, which is a distinct flag from `stale`:

| | `stale` | `at_risk` |
| --- | --- | --- |
| Applies to | empirical records only | any kind |
| Cause | a watched path moved, or a date passed | something it rests on is stale |
| Carries | the glob and commit, or the date | the propagation path and depth |
| Means | "re-observe this" | "re-check this once the source is re-observed" |

Propagation is transitive, unbounded in depth, and cycle-safe: the traversal is a
breadth-first walk from the set of stale records over the reversed three-relation graph,
with a visited set, recording for each dependent the **shortest** path and its length.
A record that is itself stale is not additionally marked at risk.

**Only live records are flagged, and doubt does not travel through a terminal one.**
`docs/02-data-model.md` §6 defines `at_risk` over live records, and the walk enforces it
in both roles. A superseded, withdrawn or disproven record rests on whatever it rested on
when it was settled; flagging it asks somebody to review a decision the project has
already moved past, and a warning nobody can act on trains people to ignore the rest. The
walk therefore stops at a terminal record rather than passing through it. `derived_from`
may point at a terminal record as provenance, and `depends_on` may point at a completed
planning record because completion satisfies the prerequisite. Neither passes later
staleness through that settled boundary.

Neither flag ever changes a record's state, its content, or the truth value of any claim
(D-003, D-024).

## 5. Worked example

Using the frozen synthetic history of
[`../examples/save-your-skin/MANIFEST.md`](../examples/save-your-skin/MANIFEST.md) §4.
`HEAD` is C5; today is 2026-08-03.

| Id | Commit | Touched |
| --- | --- | --- |
| C1 | `3f0a1c9d…` | `sim/src/**`, `lege/src/**` |
| C2 | `7c41d0ba…` | `sim/src/project/**`, `sim/src/step.rs` |
| C3 | `b2e58f14…` | `lege/src/**`, `sim/src/step.rs` |
| C4 | `5d9c2a70…` | `sim/src/project/**`, `sim/tests/determinism.rs` |
| C5 | `e806b3f5…` | `lege/src/render/**`, `docs/generated/**` |

### Stale by watched path

`sim.obs.projection-gaps/1` has `observed_at git:7c41d0ba…` (C2) and
`watches [ "sim/src/project/**" ]`. Commits reachable from C5 and not from C2 are
{C3, C4, C5}. C4 touched `sim/src/project/**`. **Stale**, cause `watch`, detail
`("sim/src/project/**", 5d9c2a70…)`.

### Stale by review date

`sim.obs.timestep-drift/1` has `review_after 2026-07-15`, which is before 2026-08-03.
**Stale**, cause `review_after`. Its `watches` are not even consulted — either condition
suffices.

### Fresh

`lege.obs.frame-budget-headroom/1` has `observed_at git:e806b3f5…` (C5) and watches
`lege/src/render/**`. No commit is reachable from C5 but not from C5. **Fresh**, even
though C5 itself touched a watched path: the observation was made *at* that commit, so
the change is already accounted for.

`lege.obs.viewer-imports-engine/1` is `disproven` — terminal — and is not evaluated at
all.

### Propagation

```
        sim.obs.projection-gaps/1                     sim.obs.timestep-drift/1
        STALE (watch: sim/src/project/**,             STALE (review_after
               matched by 5d9c2a70)                          2026-07-15 passed)
          │                      │                              │
          │ depends_on           │ supported_by                 │ supported_by
          ▼                      ▼                              ▼
 sim.work.rewrite-      sys.assessment.projection-     sys.assessment.m3-
 projection/1           gaps/1                         readiness/1
 AT RISK depth 1        AT RISK depth 1                AT RISK depth 1
                                 │
                                 │ supported_by
                                 ▼
                        sys.policy.tandem-work/1
                        AT RISK depth 2
```

**2 stale, 4 at risk, maximum propagation depth 2.** Nothing else is flagged, which is
the property worth checking: a forty-record project with two aged observations produces
six entries in the review queue, not forty.

Two edges in that picture are worth reading slowly, because between them they are the
whole argument for propagation.

`sys.policy.tandem-work` is a live, active governance rule that nothing has touched. Its
support rests on an assessment, which rests on an observation, which was made before the
projection code changed. Nobody looking at the policy would have known. The graph knew.

`sim.work.rewrite-projection` is the other direction of the same point, and the one an
agent meets first. It is the work item somebody is about to pick up, and it declares
`depends_on [ @sim.obs.projection-gaps ]` — the coverage measurement that motivated it.
That measurement is now stale, so the *premise of the work* is stale, and the flag says so
before anyone starts. Note that this is a `work` record: propagation is not restricted to
records that make claims about the world. Anything that declares its correctness rests on
something stale is flagged, whatever its kind.

## 6. `akr impact --git-diff`

Staleness answers "what is questionable now?". Impact answers "what would this change
make questionable?" — the same computation, run against a hypothetical or a proposed
range instead of against `(observed_at, HEAD]`.

```
akr impact --git-diff <A>..<B> [--depth <n>] [--format text|json]
```

**Algorithm.**

1. Resolve `A` and `B`. Either unknown is `AKR-G013`.
2. Collect the paths touched by commits in `(A, B]`.
3. For each live empirical record whose `observed_at` is reachable from `A`, test its
   watch globs against those paths.
4. A match that the record was not already stale for **at `A`** is **newly stale**.
5. Propagate from the newly stale set along the three relations of §4. New dependents
   are **newly at risk**.
6. Report both sets, with cause and path.

Note step 3's condition. A record observed *after* `A` has already accounted for part of
the range, so it is tested only against the commits it does not contain. Without that
condition, `akr impact` would report every observation in the repository as endangered by
any large range, and nobody would run it twice.

Note step 4's baseline as well. "Already stale" means stale **as of `A`**, computed by
running §3's derivation with `A` in place of HEAD — not stale as of HEAD. The distinction
matters for exactly the interesting case: a range that has already been merged made some
record stale, and asking `akr impact` what that range did must answer "this one", not
"nothing, it was already stale" — which it is, because of the range being asked about.

**Against the frozen example**, `akr impact --git-diff C4..C5` reports **no newly stale
records**. C5 touches `lege/src/render/**` and `docs/generated/**`. The only record
watching `lege/src/render/**` is `lege.obs.frame-budget-headroom/1`, whose `observed_at`
*is* C5 — it already accounts for the change. Nothing watches `docs/generated/**`, which
is generated output and should not be watched by anything.

That negative result is the useful one to have documented: the command reports nothing
when nothing is invalidated, so a report of nothing is informative rather than a sign it
did not run.

**Where to run it.** In a pre-commit or pre-push hook over the staged range, and in CI
over the pull request's range, as information rather than as a gate (§8). It tells the
author which observations their change is about to invalidate at the one moment they are
equipped to re-observe them.

## 7. `akr review-queue`

The human-facing half of the freshness model. `REVIEW-REQUIRED.md`
([`11-projections.md`](11-projections.md) §6) is the committed half, generated from the
same data.

**Ordering**, total and deterministic:

1. **Stale before at-risk.** Stale records are the source of the problem; fixing one may
   clear several at-risk entries at once.
2. Within stale: cause `watch` before cause `review_after`. A watched path that moved is
   a concrete, locatable change; a passed date is a prompt.
3. Within cause `watch`: by the matching commit in reverse history order — most recent
   invalidation first.
4. Within cause `review_after`: by date ascending — longest overdue first.
5. Within at-risk: by propagation depth ascending. Depth 1 is nearest the source.
6. Final tiebreak everywhere: record key.

```
$ akr review-queue
STALE (2)
  sim.obs.projection-gaps/1        observation  verified
      watches "sim/src/project/**" matched by 5d9c2a70 (C4)
      observed_at 7c41d0ba (C2)
  sim.obs.timestep-drift/1         observation  verified
      review_after 2026-07-15 passed (19 days ago)

AT RISK (4)
  depth 1  sim.work.rewrite-projection/1      work
             via depends_on   -> @sim.obs.projection-gaps
  depth 1  sys.assessment.m3-readiness/1      assessment
             via supported_by -> @sim.obs.timestep-drift
  depth 1  sys.assessment.projection-gaps/1   assessment
             via supported_by -> @sim.obs.projection-gaps
  depth 2  sys.policy.tandem-work/1           policy
             via supported_by -> @sys.assessment.projection-gaps
                              -> @sim.obs.projection-gaps

2 stale, 4 at risk
```

**Exit 0**, always, however long the queue is. A non-empty queue is normal and healthy: it
means the project is moving and the ledger noticed. Projects that want a gate opt in with
`akr check --review-clean`, which raises `AKR-G041` (V-104) and exits 1 — and §8 explains
why that should be a nightly notification rather than a merge gate.

**Acting on the queue** is always an explicit write, never a build side effect: `akr
revise` to re-observe with a new `observed_at`, `akr evidence add` to record a fresh
check, `akr supersede` to replace the record, or nothing at all if the reviewer decides
it still holds. Deciding it still holds is a legitimate outcome, and `akr revise` with an
updated `observed_at` and unchanged prose is how it is recorded.

## 8. Invariants

Four, and they are the reason the freshness model can be trusted.

**1. The compiler never declares a record false.** It flags `stale` and `at_risk`, both of
which mean "look at this". Neither means "this is wrong". The distinction is what keeps
the ledger honest: a system that auto-invalidated knowledge would train its users to
ignore the flags.

**2. `akr build` never writes a `.akr` file.** Not to mark staleness, not to update a
date, not ever. The build's only outputs are the index cache, the generated views, and
`akr.lock` (D-003). A build that mutated its inputs would be neither reproducible nor
cacheable, and the "pure function of (sources, commit, tool version)" contract would be
false.

**3. Staleness is a build fact, not a diagnostic.** No code, no diagnostic stream, no
effect on exit status. The single opt-in exception is `akr check --review-clean`, whose
`AKR-G041` reports an unmet command-line request rather than a defect in the ledger.

**4. Freshness is computed from committed history only.** Uncommitted working-tree changes
are invisible to it, and `AKR-G004` warns when watched paths have uncommitted edits so
that nobody is misled by a clean queue on a dirty tree.

## 9. Rules

| Rule | Statement | Codes |
| --- | --- | --- |
| **V-101** | Every `observed_at` and `as_of` commit exists in the repository, and is an ancestor of the resolved commit. | `AKR-G011` (error), `AKR-G012` (warning) |
| **V-102** | Every `watches` and path-scope glob is within the D-008 subset and matches at least one path at the resolved commit. | `AKR-G021` (error), `AKR-G022`, `AKR-G023` (warnings) |
| **V-103** | `review_after` is not earlier than `created_at`. | `AKR-G031` (warning) |
| **V-104** | Under `akr check --review-clean`, the review queue is empty. | `AKR-G041` (error) |

`AKR-G001`, `AKR-G002`, `AKR-G003`, `AKR-G004` and `AKR-G013` implement no rule: they
report that the repository or the invocation is unusable, not that an invariant was
broken.

## 10. Recipes

**Pre-commit — informational.**

```bash
#!/bin/sh
# .git/hooks/pre-commit
akr fmt --check || exit 1
akr check       || exit 1
akr impact --git-diff HEAD..   # what am I about to invalidate?
```

The `impact` line never fails the commit. Its job is to put "you are about to invalidate
`sim.obs.projection-gaps`" in front of the author while the change is still in their
hands.

**Pull request — gate on correctness, report on freshness.**

```yaml
- run: akr check --views-current           # gate
- run: akr impact --git-diff origin/main.. --format json > impact.json
  if: always()                             # report
- run: akr review-queue --format json > queue.json
  if: always()                             # report
```

**Nightly — the only place a review gate belongs.**

```bash
akr check --review-clean --format json > queue.json || notify "AKR review queue"
```

`review_after` dates pass with the calendar, not with commits, so nothing in the
pull-request path will ever notice them promptly. A nightly notification does.

**Why `--review-clean` is not the merge gate.** A gate that fails because knowledge aged
teaches contributors to delete the aged knowledge, or to stop writing `watches` globs at
all. Both make the ledger worse than having no freshness model. The flag exists for
projects that have decided otherwise, deliberately, with their eyes open.

---

Next: [`11-projections.md`](11-projections.md) for `REVIEW-REQUIRED.md`, the committed
form of the queue, or [`09-context-assembly.md`](09-context-assembly.md) §4 step 11 for
how these flags reach an agent.
