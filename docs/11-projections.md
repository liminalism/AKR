# 11 — Projections

The seven generated Markdown views: what each one selects, how it is ordered, how each kind
renders, what the banner contains, and how the never-hand-edit rule is enforced.

Normative for the view catalogue, source queries, section ordering, rendering rules, the
banner format, and rules V-111–V-115.

---

## 1. What a view is

A view is a **Markdown rendering of a query over the resolved model**, emitted by
pipeline stage F ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §8) and committed
to the repository (D-025).

Views exist for one reason: the ledger must be legible to readers and tools that will
never install AKR. Somebody browsing the repository on the web, a reviewer reading a pull
request, an agent with no MCP connection, a stakeholder who wants to know where the
project is — all of them get current knowledge from a Markdown file, because the build
put it there.

Three properties, none negotiable:

- **Derived.** Every byte comes from records. A view never contains information that is
  not in the ledger, and never omits a record its query selects.
- **Committed.** They live in git, in `docs/generated/` by default, so the web view of
  the repository is current.
- **Never hand-edited.** Enforced by CI, not by convention (§9).

## 2. The catalogue

Seven views, in the fixed order stage F renders them.

| File | Question it answers | Source query |
| --- | --- | --- |
| `ROADMAP.md` | Where is the project going? | Live `milestone` and `track` records, with their plans, acceptance and children |
| `CURRENT-STATE.md` | What does the project believe right now? | Live `term`, `requirement`, `policy`, `constraint` and live empirical records |
| `ACTIVE-WORK.md` | What is being worked on, and what is stuck? | Live `work` records, grouped by parent |
| `REVIEW-REQUIRED.md` | What should not be trusted without re-checking? | Records flagged `stale` or `at_risk` by stage D |
| `OPEN-QUESTIONS.md` | What is not yet known? | `question` records |
| `DECISION-HISTORY.md` | What was decided, and what was retired? | Every revision of every `decision`, plus terminal revisions of other normative kinds |
| `PAPERCUTS.md` | Where does the project need sanding down? | Live `papercut` records, newest first. Emitted only once one exists (D-027) |

The catalogue is closed for 0.1. A project needing an eighth view writes a template
against a declared section set (§10) rather than adding a case to the renderer.

`akr view <name>` renders one to stdout; an unknown name is `AKR-E003`.

## 3. Universal rendering rules

These hold in every view, and they are what make the output diffable.

**Headings come from `title`.** Every record carries a required `title` slot (D-012), and
that string, verbatim, is the heading. Nothing is derived from prose, truncated from a
statement, or synthesised. Two selected records with the same title in one view produce
the same heading anchor and are `AKR-E022` (V-115) — titles must be distinguishable
within a view, which is a cheap authoring constraint and a real navigation benefit.

**Every record is rendered with its key and revision.** Always as `@key/rev`, never as
just a title. The key is the citable identity; the title is the label.

**Every reference is a link.** A reference to a record rendered elsewhere in the same
view is an in-page anchor. A reference to a record in another view is a relative link to
that file's anchor. A reference to a record in no view — an archived record, say —
renders as plain `@key/rev` text with no link, because a dead link is worse than none.

**State is rendered as a bare word in backticks**, never as an emoji, a colour, or a
badge image. `` `active` ``, `` `blocked` ``, `` `superseded` ``. Views are read in diffs,
in terminals, and by tools.

**Freshness is rendered on the metadata line, never in the heading.** `**stale**` or
`**at risk**` appended to the `state · key · scope` line beneath the heading, with the
cause in a block quote after the body. It is deliberately kept out of the heading: a
heading anchor that changed when a record went stale would break every link into it from
every other view, on a build that changed no record at all.

**Prose is rendered verbatim**, dedented as the parser produced it, with no re-wrapping.
Re-wrapping would make the diff of a one-word edit span a paragraph.

**Lists are sorted by the view's declared sort key, ending in the record key**, so the
order is total ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §11).

**Empty sections are printed with `_(none)_`.** A missing heading would be ambiguous
between "nothing here" and "not generated".

**A terminal planning record renders its `note` slot.** `work`, `milestone` and `track`
carry an optional `note` (D-026): free-form operator commentary with no validation
consequence, which is where `akr abandon --reason` puts the reason. When such a record is
in a terminal state — `completed`, `abandoned`, `superseded` — the note is rendered as a
block quote after the body, prefixed `**Note:**`:

```markdown
`abandoned` · `@sys.work.m3-plan/2` · target 2026-09-15

> **Note:** The milestone was rescoped and this plan no longer describes it.
```

It appears wherever such a record appears — `ROADMAP.md`, `ACTIVE-WORK.md` and
`DECISION-HISTORY.md` — and **only** in a terminal state. On a live record the note is
working commentary that the record's own `intent` should be saying instead; on a terminal
one it is the last thing anybody wrote about it, and the only place a reader will find out
why the plan stopped. A comment could not do this job: comments are excluded from the seal
hash by D-015 and no view renders them.

## 4. The banner

Every generated file opens with exactly this, on line 1 (D-025):

```
<!-- GENERATED BY AKR — DO NOT EDIT
     source-graph: sha256:<64 hex>
     tool: akr <version>
-->
```

| Field | Meaning |
| --- | --- |
| `source-graph` | SHA-256 over the sorted `(path, file-hash)` pairs of every source file the build read ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §9). Identical across all views of one build. |
| `tool` | Semver of the binary that rendered it. |

Both are inputs to the build. **No timestamp appears in the banner**, deliberately:
a wall-clock field would make every rebuild produce a diff, which would make the
`--views-current` gate useless and train everyone to ignore view changes.

A commit hash is excluded for the same convergence reason: the generated bytes are part
of the commit whose hash they would need to contain. `source-graph` is stable before and
after Git advances, and `AKR-Graph` commit trailers provide the reverse lookup. The
current Git fact is shown directly by `akr start` instead of copied into every projection.

A banner that is absent, malformed, or not on line 1 is `AKR-E013` (V-113). Its two
fields are also what makes a view *self-describing*: a reader who finds a view
surprising can check whether it was built from the sources they are looking at.

## 5. `ROADMAP.md`

**Source query.** The head revision of every `milestone` record, then of every `track`
record, excluding archived ones (D-018). For each: its `plan_of_record`, its `part_of`
children, its `depends_on` targets, and its acceptance checks with verdicts.

**Completed milestones are included.** The query is over heads, not over live records: a
roadmap that hides finished work cannot show what was finished, and "M1 and M2 are done"
is the first thing a reader wants.

**Section order.** (1) Milestones, in `after` topological order with key as tiebreak.
(2) Tracks, by key. (3) Two summary tables — milestones, then tracks.

**Per milestone**, in this order: the `title` as a heading; a metadata line; the `intent`
prose; the plan of record as a link; the acceptance heading and table; a **Depends on**
line if it has `depends_on` targets; the live child work items; and, where it has a plan
of record, the live work items under that plan.

**Per track**, the same shape without acceptance: heading; metadata line; `intent`; a
**Depends on** line; work items; and a closing line naming any work carried into the
track by a `disposition` (D-017), which is not `part_of` the track and would otherwise
appear nowhere.

**The metadata line** carries, in this order and separated by ` · `: the state in
backticks; the record's `@key/revision` in backticks; for tracks, `scope` with each term
in its own backticks; `target` where present; `cadence` where present; `after` with each
resolved target in backticks; and the freshness marker where the record has one. Absent
slots are omitted rather than rendered empty.

**Work item lists.** `- ` then the state in backticks, the title as a link, the
`@key/revision`, and — under a plan of record — ` — part of `@key/rev`, dispositioned
`outcome`[ into `@key/rev`]`. A freshness marker joins with ` — ` where it is the only
qualifier and with `, ` where a disposition clause precedes it.

**Nothing editorial.** Every byte comes from records (§1). A sentence explaining *why* a
milestone is where it is belongs in the record's `intent` or in an `assessment`, not in
the view — the renderer cannot produce it, so a view containing one is a view that has
been hand-edited.

**Rendered sample** — full file in
[`../examples/save-your-skin/docs/generated/ROADMAP.md`](../examples/save-your-skin/docs/generated/ROADMAP.md):

```markdown
### M3 — playable day

`active` · `@sys.milestone.m3-playable-day/1` · target 2026-09-30 · after `@sys.milestone.m2-deterministic-sim/1`

A player can start, play and finish one in-game day without a crash, a
soft-lock, or a placeholder asset.

**Plan of record:** [M3 plan of record](ACTIVE-WORK.md#m3-plan-of-record) `@sys.work.m3-plan/2`

**Acceptance** — 1 of 2 satisfied

| Check | Method | Verdict |
| --- | --- | --- |
| `full-day-demo` | observation | **satisfied** by `@sys.evidence.playable-day-demo/1` (pass at `e806b3f5`, descends from `b2e58f14`) |
| `no-placeholder-assets` | command | not satisfied — no evidence |

**Work items**

- `active` [Extract the render graph behind the boundary](ACTIVE-WORK.md#extract-the-render-graph-behind-the-boundary) `@lege.work.extract-render-graph/1`
- `blocked` [Rewrite the projection pass](ACTIVE-WORK.md#rewrite-the-projection-pass) `@sim.work.rewrite-projection/1`

**Under the plan of record**

- `ready` [Ambient audio for the day loop](ACTIVE-WORK.md#ambient-audio-for-the-day-loop) `@sys.work.m3-audio-pass/1` — part of `@sys.work.m3-plan/1`, dispositioned `intentionally_dropped`
```

Two things this sample is doing that a hand-written roadmap does not.

The **acceptance table** is why the view is worth generating. "M3 is active because one of
two checks is unsatisfied, and here is which one, and here is the evidence that closed the
other" is a fact nobody maintains by hand for more than a fortnight.

The **"Under the plan of record"** section exists because `part_of` pins to a plan revision
([`04-references-and-versioning.md`](04-references-and-versioning.md) §5). A child of a
superseded plan keeps pointing at the revision that owned it, so a view that listed only
the head's children would silently drop exactly the items a replan is most likely to lose.
They are listed, each with the disposition that governs it.

## 6. `CURRENT-STATE.md`

**Source query.** Live normative records (`term`, `requirement`, `policy`, `constraint`)
and live empirical records (`observation`, `evidence`, `assessment`). Decisions are
excluded — they have their own view.

**Section order.** Terms, Constraints, Policies, Requirements, Observations, Assessments,
Evidence. Within each, by key.

The order runs from what words mean, through what the project cannot change, to what it
has chosen, to what it must deliver, to what has been found. Each section constrains the
next.

**Per record:** heading from `title`, then `` `state` `` · `@key/rev` · scope, then the
kind's required prose slot, then claims as a definition list, then relations as a compact
list, then the freshness marker if any.

```markdown
### Engine and simulator advance in tandem

`active` · `@sys.policy.tandem-work/1` · scope `all` · topic `tandem-work` · **at risk**

No engine change lands without the matching simulator change in the same
commit, except on the tracks listed under exceptions, where the simulator may
lag by at most one milestone.

- `#lag-bound` — Permitted simulator lag on an excepted track is at most one milestone,
  never two.
- `#same-commit` — Matching engine and simulator changes ship in one commit, not in a
  follow-up commit on the same day.

**exceptions** `@sys.track.lighting/1` · **supported_by** `@sys.assessment.projection-gaps/1`

> At risk at depth 2 via `supported_by` → `@sys.assessment.projection-gaps/1` →
> `@sim.obs.projection-gaps/1` (stale). See
> [REVIEW-REQUIRED.md](REVIEW-REQUIRED.md#engine-and-simulator-advance-in-tandem).
```

Evidence records carry an extra rendered line this view computes rather than reads: a
**"Verifies"** list, built by reversing the `verified_by` edges that point *at* the
evidence. The relation runs one way in the source (D-016); the view reverses it for the
reader, who is usually holding the evidence and wondering what it closed.

## 7. `ACTIVE-WORK.md`

**Source query.** Live `work` records — `proposed`, `ready`, `active`, `blocked`.

**Section order.** Grouped by `part_of` parent, parents in `ROADMAP.md` order,
unparented work last under "Unparented". Within a group: by state in the order `active`,
`blocked`, `ready`, `proposed`, then by key.

**Per record:** heading from `title`; state, key, parent; `intent`; the plan-of-record
designation if it has one; `disposition` blocks if it carries any; live `blocks` edges
holding it, each naming the blocker; acceptance checks if it has any.

Blocked work renders its blocker inline, because "blocked" without "by what" is the least
useful status in software.

```markdown
### Rewrite the projection pass

`blocked` · `@sim.work.rewrite-projection/1` · part of `@sys.milestone.m3-playable-day/1`

Rewrite the day-boundary projection so that pending state reconciliation is one
path rather than four, and bring coverage on that path up to the steady-state
level.

**Blocked by**

- `open` [Does a 4 ms timestep fit the frame budget?](OPEN-QUESTIONS.md#does-a-4-ms-timestep-fit-the-frame-budget)
  `@sim.question.timestep-vs-budget/1`

**depends_on** `@sim.obs.projection-gaps/1`
```

The plan of record renders its `disposition` blocks in full. They are the record of what
happened to the previous plan's unfinished children (D-017), and this view is the only
place a casual reader will ever see them.

## 8. `REVIEW-REQUIRED.md`

**Source query.** Every record flagged `stale` or `at_risk` by stage D
([`10-freshness-and-git.md`](10-freshness-and-git.md) §3, §4).

**Section order.** Stale, then At risk, using the review-queue ordering of
`10-freshness-and-git.md` §7 — cause `watch` before cause `review_after`, then propagation
depth, then key.

**Per record:** heading from `title`; state, key; the cause in full — the glob and the
commit that matched it, or the date that passed; for at-risk records, the propagation
path with every hop named.

```markdown
## Stale (2)

### Projection coverage is thinnest at day boundaries

`verified` · `@sim.obs.projection-gaps/1` · observation · **stale** ·
[CURRENT-STATE.md](CURRENT-STATE.md#projection-coverage-is-thinnest-at-day-boundaries)

**Cause** — `watches "sim/src/project/**"` was matched by `5d9c2a70`, which is not
reachable from `observed_at 7c41d0ba`.

**Matched path** — `sim/src/project/` (C4, 2 commits after the observation)

## At risk (4)

### Projection gaps put the M3 date at risk

`verified` · `@sys.assessment.projection-gaps/1` · assessment · **depth 1** ·
[CURRENT-STATE.md](CURRENT-STATE.md#projection-gaps-put-the-m3-date-at-risk)

**Via** `supported_by` → `@sim.obs.projection-gaps/1` (stale: watched path moved)
```

The view also carries a **"Not flagged"** table naming a few records a reader might expect
to see and the reason they are absent — an observation made *at* the commit that touched
its watched path, a terminal record that is never evaluated, a milestone that staleness
does not propagate to. "Why is this not here?" is asked as often as the reverse, and a
generated view can answer it for free.

This view has a property the others do not: **it is expected to be non-empty**, and a
long-empty `REVIEW-REQUIRED.md` on an active project means the `watches` globs are wrong,
not that the knowledge is perfect. It carries a header line saying so, because a reader
encountering an empty file will otherwise conclude the wrong thing.

Nothing in this view is a diagnostic. It is generated on every successful build, including
builds that exit 0 with a long queue (D-024).

## 9. `OPEN-QUESTIONS.md`

**Source query.** `question` records: live ones (`open`, `deferred`) in the main
sections, and terminal ones (`resolved`, `closed-without-resolution`) in a trailing
section, because "what did we decide about that?" is asked as often as "what is still
open?".

**Section order.** Open, Deferred, Recently closed. Within each, by key.

**Per question:** heading from `title`; state and key; the `question` prose; what it
`blocks`, as links; for resolved questions, the `resolution` prose and the live `resolves`
edge that closed it (V-011).

```markdown
### Does a 4 ms timestep fit the frame budget?

`open` · `@sim.question.timestep-vs-budget/1`

At 4 ms the simulator runs four steps per rendered frame. Does the resulting
per-frame cost fit inside the 16 ms budget on target hardware, with the
renderer's share unchanged? Nobody has measured four steps against the budget
on the target box.

**Blocks**

- `proposed` `@sim.decision.timestep-4ms/1` — Fix the simulator timestep at 4 ms
  ([DECISION-HISTORY.md](DECISION-HISTORY.md#simdecisiontimestep-4ms))
- `blocked` `@sim.work.rewrite-projection/1` — Rewrite the projection pass
  ([ACTIVE-WORK.md](ACTIVE-WORK.md#rewrite-the-projection-pass))
```

Archived questions are excluded, like everywhere except `DECISION-HISTORY.md` (D-018).

## 10. `DECISION-HISTORY.md`

**Source query.** Two sets, unioned:

- every revision of every `decision` record, live or terminal, including archived; and
- every **terminal** revision of any other normative kind — `superseded`, `rejected`,
  `withdrawn` — including archived.

This is the one view that includes archived records (D-018). It is the project's memory
of what it used to think.

**Section order.** By key, then by revision **descending**, so the current revision of a
key precedes the revisions it replaced. Grouped under one heading per key, so a
supersession chain reads as one narrative rather than as scattered entries.

Ordering is by key rather than by date deliberately: `created_at` is optional, and a view
whose order depends on an optional slot would reorder when someone fills one in.

**Per revision:** the revision number, state, the `decision` prose, `context` and
`consequences` if present, and the `supersedes` / superseded-by edges.

```markdown
### lege.decision.renderer-boundary

#### Revision 2 — The viewer consumes a frame snapshot

`active` · `@lege.decision.renderer-boundary/2` · scope `path "lege/**"` · topic `renderer-boundary`

The viewer reads an immutable frame snapshot produced by the simulator at each
tick boundary. It does not call into the simulator, and no engine type appears
in a viewer signature.

**Retired claims** — `direct-calls`, which revision 1 defined. A reference to
`@lege.decision.renderer-boundary#direct-calls` therefore reports "anchor retired at
revision 2" rather than "not found" (D-011).

**supersedes** `@lege.decision.renderer-boundary/1` ·
**resolves** `@lege.question.text-rendering-owner/1` ·
**implements** `@lege.req.no-engine-types-in-viewer/1` ·
**derived_from** `@lege.obs.viewer-imports-engine/1`

#### Revision 1 — The viewer calls the simulator directly

`superseded` · `@lege.decision.renderer-boundary/1` · scope `path "lege/**"`

The viewer calls simulator entry points directly at each tick boundary and
reads state through them.

- `#direct-calls` — The viewer calls simulator entry points directly; there is no
  intermediate representation of frame state. *(retired at revision 2)*

Superseded by `@lege.decision.renderer-boundary/2`.
```

Note the `derived_from` pointing at a `disproven` observation. That is legal and
intentional: the decision really was derived from the finding that the viewer imported
engine types, and that finding has since been disproven by the boundary lint. Recording
the provenance is more honest than hiding it, and V-019 restricts only `depends_on`,
`implements`, `plan_of_record` and `supported_by` to live targets — the historical
relations are exempt, because pointing at retired knowledge is what they are for.

## 11. Enforcement

If a view differs only in its tool-version banner, the check reports `AKR-E015` as a
non-fatal migration warning; rebuilding refreshes that stamp.

D-025 in mechanism form.

`akr check --views-current` runs stage F **in memory** and compares against the committed
files, byte for byte:

| Situation | Code | Rule |
| --- | --- | --- |
| A committed view differs from the rebuilt view | `AKR-E011` | V-112 |
| A catalogued view has no committed file | `AKR-E012` | V-112 |
| A banner is absent, malformed, or not on line 1 | `AKR-E013` | V-113 |
| A file in the output directory is not a generated view | `AKR-E014` | V-114 |
| A record the query selected is absent from the model | `AKR-E021` | V-111 |
| Two records in one view share a heading anchor | `AKR-E022` | V-115 |

Plus the output-location checks that apply to `akr build` itself: an unwritable directory
is `AKR-E001`, and a `view_output` resolving outside the repository root is `AKR-E002`.

`AKR-E011` reports the first differing line and the differing line count, because the
useful question is "was this a hand edit or a stale build?" and the answer is usually
visible in one line of context.

**This check is the CI gate**, and it is what gives the `sys.policy.no-hand-edited-views`
record actual force rather than good intentions:

```yaml
- run: akr check --views-current
```

A hand edit fails the build with the file and line named. A ledger change without a
rebuild fails the same way, with the same fix: run `akr build` and commit the result.

**Merge conflicts** in `docs/generated/` are the acknowledged cost of committing build
output. The remedy is mechanical and should be in the project's contributing guide: take
either side, run `akr build`, commit. Because the build is deterministic and the banner
carries no timestamp, the result is identical regardless of which side was taken.

## 12. View templates

The seven views are fixed (`PAPERCUTS.md`, the seventh, is emitted only once the ledger holds a papercut — D-027). Projects that need an eighth — a per-team roadmap, a
compliance extract — declare a template rather than patching the renderer.

```
defaults {
    view_output "docs/generated"
    view_template "docs/templates/team-roadmap.md.tmpl"
}
```

Rules, all of which exist to keep templates from becoming a way to lie:

1. **Templates compose declared sections; they do not query.** A template names sections
   from a view's declared section set and orders them. It has no query language, so it
   cannot select a record the underlying view's query did not.
2. **A template may omit and reorder; it may not add.** Nothing renders that the model
   does not hold.
3. **The banner is not templatable.** It is prepended by the renderer, always, in the
   form of §4.
4. **Templated output lands in the same directory and is checked the same way.** A
   template's output is a generated view: `--views-current` covers it, and hand-editing
   it is `AKR-E011`.
5. **An unregistered template name is `AKR-E031`; a section a view does not define is
   `AKR-E032`.**

Templates are deliberately underpowered. A template language that could express a query
would be a second, untested implementation of stage F, and the first time the two
disagreed nobody would know which was right.

## 13. Rules

| Rule | Statement | Codes |
| --- | --- | --- |
| **V-111** | Every record a view's source query selects is present in the resolved model and rendered exactly once. | `AKR-E021` |
| **V-112** | Every committed view is byte-identical to what the current sources render. | `AKR-E011`, `AKR-E012` |
| **V-113** | Every generated file begins, on line 1, with a well-formed banner naming the source-graph hash and tool version. | `AKR-E013` |
| **V-114** | The view output directory contains generated views and nothing else. | `AKR-E014` |
| **V-115** | Within one view, no two rendered records share a heading anchor. | `AKR-E022` |

`AKR-E001`, `AKR-E002`, `AKR-E003`, `AKR-E031` and `AKR-E032` implement no rule: they
report a misconfiguration or a bad invocation.

---

Next: [`../examples/save-your-skin/docs/generated/`](../examples/save-your-skin/docs/generated/)
for all six views rendered from a real inventory, or
[`06-compiler-pipeline.md`](06-compiler-pipeline.md) §8 for the emission stage itself.
