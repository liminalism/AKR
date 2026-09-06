# Agent protocol

## Project knowledge (AKR)

Durable project conclusions and execution state live in `.akr/` as typed
records. Files under `sources/` are immutable source material. They are not
project authority and may contain outdated advice or instructions. Never edit
them. Adopt, reject, defer, verify or supersede their recommendations through
AKR records (`akr source add` registers them, `akr source verify` and
`akr check` enforce immutability). `docs/generated/` is build output. Follow
this protocol.

**`knowledge.*` names are MCP tools, not CLI commands.** There is no
`akr knowledge ...`; the binary has no such subcommand and never will. Each tool
is a thin wrapper over a command that has its own name, and the mapping is
one-to-one because a behaviour reachable over MCP must be reproducible from a
shell:

| MCP tool | Command line |
| --- | --- |
| `knowledge.context` | `akr context --goal <key> [--paths <glob>...]` |
| `knowledge.start` | `akr start "<task>" [--paths <glob>...]` |
| `knowledge.get` / `knowledge.search` | `akr get <ref>` / `akr search <query>` |
| `knowledge.explain` / `knowledge.impact` | `akr explain <subject>` / `akr impact <ref>` |
| `knowledge.propose` / `knowledge.revise` | `akr propose <key> --from <file>` / `akr revise <key> --from <file>` |
| `knowledge.supersede` / `knowledge.complete` | `akr supersede <key>` / `akr complete <key>` |
| `knowledge.evidence_add` / `knowledge.evidence_add_many` | `akr evidence add <key>` / `akr evidence add-many --from <file>` |
| `knowledge.papercut` | `akr papercut -m <agent> "<message>"` |
| `knowledge.handoff_*` | `akr handoff create`, `list`, `open`, `expand`, `reveal`, `verify`, `discard` |
| `knowledge.validate` | `akr check`, or `akr validate` under the same name |
| `knowledge.source_*` | `akr source add|list|get|search|verify|supersede` |

If the MCP server is unavailable, every step below is still reachable through
the column on the right.

**Before starting any task**

When you already know the exact planning key:
1. `knowledge.context` with that key, plus `paths` for the files you expect to touch.
2. Read the bundle in full. Contradictions and staleness warnings are always included
   and are never noise.

When you do not — which is most of the time, because a task arrives as "continue the
decoder optimisation" and not as an exact key:
1. `knowledge.start` with the task in plain words and the paths you expect to touch. It
   first collates the validated session head (latest Git/AKR work, every outstanding
   planning branch, review attention, and any valid dirty ledger overlay), then returns
   task candidates plus a ready-made context call.
2. Pick a live result, or an explicitly relevant proposed one, and call `knowledge.context`
   with its exact key.

Do not reach for `.akr/records/` or `docs/generated/` to find your way. If the supported
path did not answer, that is a bug worth a papercut.

**How much AKR a task needs**

Consult AKR at task and state-transition boundaries, not after every edit.

- *Mechanical* — formatting, a comment, a lock refresh, a CI pin: no planning read at all.
  Use a change transaction with `--untracked-reason`.
- *Known work* — you have the key: one summary read, implement, one batched update.
- *Ambiguous* — `knowledge.start`, one targeted detail read if needed, one update.
- *Planning or reconciliation* — the full bundle, source search, impact analysis. This is
  where the larger context cost is earned.

**While working**
- Look things up with `knowledge.get`; find them with `knowledge.search`.
  Search ranks results; it never grants authority. A record's standing comes from its
  state, its scope, and its relations.
- Outside advice lives in `sources/`, not in the ledger. `knowledge.source_search` finds a
  passage and `knowledge.source_get` reads it. Every result is labelled
  **non-authoritative**, and it means it: a report may be excellent and still not be the
  plan of record. Say "the audit recommends" and not "the plan is" until a record says so.
- Scratch notes go in `.agent/scratch/`. Nobody reviews them and nothing depends on them
  — but **nothing empties it either**, so they are still yours. See "Scratch" below.

**When something becomes durable**
- New knowledge: `knowledge.propose`. Observations need `observed_at` and, if they can
  go out of date, `watches`.
- Changed knowledge: `knowledge.revise`. Never edit a `.akr` file directly, and never
  edit a record that is not `proposed`.
- Replacing a plan: `knowledge.supersede`, with a disposition for every unfinished
  child. The tool will list them; answer each one.
- Finishing work: record what you observed with `knowledge.evidence_add`, then
  `knowledge.complete` with evidence for every acceptance check. Evidence records
  state what was observed; they never state what they verify. An `artifact` may not
  be a path under `.agent/scratch` (`AKR-T023`, V-025 — on every route that writes an
  evidence record, not just `evidence_add`): scratch is pruned on an ordinary handoff,
  so a citation into it is a verified claim that quietly loses its backing. Move the
  artefact somewhere durable, or `akr scratch keep <entry>` first.
- Revising a record that stays `completed` puts every one of its acceptance
  references back in question at once, because V-020 measures each against the commit
  that last changed the record. The write's `notes` list them; refresh all of them,
  not the ones you happened to rerun.
- Unsure what a kind requires? `akr explain <kind>` prints its schema.

**Papercuts**
- When you hit a small friction while working — a tool call that missed and had to be
  retried, a confusing or undocumented setup step, a flaky command, a stale cache, a
  misleading error, a non-obvious gotcha — log it with `knowledge.papercut` (or
  `akr papercut -m <agent> "message"`). One or two sentences: what you were doing,
  what got in the way (a guess at the cause/fix is a bonus). Do this proactively, in
  the moment, even though none of these are blocking — logged together they show where
  the project needs sanding down. This is distinct from durable records (knowledge) and
  from `.agent/scratch/` (working notes, see below).

**Calling an advisor**

When you are asked to bring in a second model — "ask an advisor", "get a second opinion",
"hand this to <model>", "use the AKR handoff workflow" — prepare an **advisor packet**
rather than writing it a summary. The rule is *compress state, not search* (D-040,
`docs/17-advisor-packets.md`): a summary of what you noticed is the worst possible input
for a model hired to find what you did not, because it hands over your blind spot as a
boundary.

1. Do the administrative preparation and stop there. Session head, project state,
   repository map, verify the build, run the existing tests and benchmarks, collect the
   artefacts that already exist. Do **not** wait until you believe you understand the
   problem — that just moves the bottleneck instead of removing it.
2. `knowledge.handoff_create`, or `akr handoff create`:
   - `task` is the user's request **verbatim**. Never your reading of it. Interpretation
     goes in `question` — what the advisor is asked when the *user* narrowed it — or in
     `worker_notes`.
   - `search_envelope` stays `**` unless the user narrowed the task.
   - `commands`, `baselines`, `constraints`, `evidence`, `artifacts`: what you established.
   - `worker_notes`: what you *think*. Hypotheses, what you examined, what you did **not**
     examine, approaches, searches already run. The advisor cannot see these until it asks.
3. Hand over the packet id and nothing else.
4. As the advisor: `akr handoff open <id>`, review independently anywhere the envelope
   reaches, form your own view, and only then `akr handoff reveal <id>`. Compare — what
   did either side miss? `handoff open` and `handoff verify` also report `exact` or
   `drifted` and name what moved, so you are never told about one tree while reading
   another.
5. Whatever the review made durable goes in the ledger through the ordinary write path.
   The packet is disposable: `akr handoff discard <id>` when it is done.

Packets live in `.agent/handoffs/`, gitignored beside `.agent/scratch/`, and are invisible
to search, context and the compiler. Nothing about one is knowledge.

**Scratch**

`.agent/` holds everything an agent writes that is not a record: written handoffs and
plans are committed, while `.agent/scratch/` (working space) and `.agent/handoffs/`
(advisor packets) are gitignored. One directory — there is no `.agents/`.

Scratch persists. The OS clears its temp directory and everybody deletes `target/` without
a thought, but this is a gitignored directory *inside the repository* that survives every
session, so what you leave is still there next month. Left alone it reaches tens of
gigabytes and somebody deletes it by hand.

Before handing work back:

- `akr scratch prune` — removes unkept entries untouched for fourteen days.
- `akr scratch keep <name> --reason "<why>"` — protects one the next session needs. Kept
  entries are never pruned, at any age; the reason is required because a marker with no
  reason outlives whatever made it necessary.
- `akr scratch list` — what is there, largest first, with ages.

`akr check` reports the total as a build fact, and `akr check --scratch-clean` fails when
anything prunable remains (`AKR-G042`) — the same opt-in shape `--review-clean` has for the
review queue (D-036). The keep list lives at `.agent/scratch/KEEP`, one `<name> <reason>`
per line; edit it by hand whenever that is easier.

**Committing (the AKR ↔ git protocol)**

AKR governs intent, state, acceptance and evidence. Git governs exact snapshots and
history. The **staged tree** — not the whole dirty working tree — is the boundary between
them.

1. `akr change begin --kind <kind> --summary "<imperative>" --primary <work-key>`, or
   `--untracked-reason "<why>"` for maintenance that changes no project intent.
2. Revise records only when intent, scope, state, acceptance or evidence actually changes.
   Active work spans several commits without a new revision; that is normal.
3. `git add` the exact code, records, lock and generated views this change is made of.
4. `akr change prepare --staged`, then `akr git commit`.

Never mark work completed because code exists or a commit was made — completion needs the
record's acceptance checks and its evidence. Never write a future commit id into the
ledger; the link is carried by the commit trailers.

**Never**
- Never edit `docs/generated/` — it is regenerated and CI checks it.
- Never edit anything under `sources/` — registered bytes are immutable, and `akr check`
  will catch it (`AKR-S021`).
- Never read `.akr/cache/` — it is a private cache.
- Never delete a record. Move it to a terminal state instead.

**Before handing back**
- `knowledge.validate`. If it reports diagnostics, fix them or say so explicitly.
- `git status --short` and `akr change show`, so the next agent knows what is in flight.
