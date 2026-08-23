## AKR (project knowledge)

A workspace with a `.akr/` directory carries an AKR ledger: typed, versioned records of
what the project decided, observed and planned, reachable through the `akr` MCP server
(`knowledge.*` tools) and the `akr` CLI. It is a compiler for project knowledge, not a
retrieval store — it fails the build on contradictions rather than ranking them, and no
language model participates in any stage of it.

**Without a `.akr/` directory none of this applies.** Do not run `akr init` uninvited.

### Before starting a task

- You know the planning key: `knowledge.context` with that key and the `paths` you expect
  to touch.
- You do not know it, which is the usual case: `knowledge.start` with the task in plain
  words and those same paths. It returns the validated session head, the outstanding
  planning branches, and a ready-made context call.
- Read the bundle in full. The contradiction and staleness warnings are the point of the
  system, not noise to skim past.
- Do not go looking through `.akr/records/` or `docs/generated/` by hand. If the tools did
  not answer, that is a papercut worth logging, not a reason to read the files.

### How much to consult

Consult at task and state-transition boundaries, not after every edit.

- Mechanical work — formatting, a comment, a lock refresh: no planning read at all.
- Known work — one summary read, implement, one batched update.
- Ambiguous work — `knowledge.start`, one targeted read, one update.
- Planning or reconciliation — the full bundle. This is where the context cost is earned.

### While working

- `knowledge.get` reads a record; `knowledge.search` finds one. Ranking is not authority:
  a record's standing comes from its state, its scope and its relations.
- Outside advice lives in `sources/`, never in the ledger. `knowledge.source_search` finds
  a passage and `knowledge.source_get` reads it. Every result is labelled
  **non-authoritative** and means it — say "the audit recommends", not "the plan is",
  until a record says otherwise. Never edit a registered source.
- Never hand-edit a `.akr` file or anything under `docs/generated/`. Both are outputs.

### When something becomes durable

- New knowledge: `knowledge.propose`. Observations need `observed_at`, and `watches` if
  they can go out of date.
- Changed knowledge: `knowledge.revise` — never edit a record that is not `proposed`.
- A replaced plan: `knowledge.supersede`, with a disposition for every unfinished child.
- Finished work: `knowledge.evidence_add`, then `knowledge.complete`. An evidence
  `artifact` may not be a path under `.agent/scratch` — scratch is pruned on an
  ordinary handoff, and a citation into it is a verified claim that quietly loses its
  backing. Move the artefact somewhere durable first, or `akr scratch keep` it.
- Friction you hit on the way: `knowledge.papercut`.
- Before handing work back: `knowledge.validate`, or `akr validate` from a shell.

### Scratch persists — it is yours to clean up

Working files go in `.agent/scratch/`. **Nothing ever empties it.** This is not the
system temp directory, which the OS clears, and it is not `target/`, which everyone
deletes without thinking. It is a gitignored directory inside the repository that
survives every session, so whatever you leave behind is still there next month, and the
month after. Left alone across a set of projects it reaches tens of gigabytes, and a
person ends up deleting it by hand.

So, before you hand work back:

- `akr scratch prune` removes unkept entries untouched for a fortnight. Run it.
- `akr scratch keep <name> --reason "<why the next session needs it>"` protects one that
  should survive. A kept entry is never pruned, at any age.
- `akr scratch list` shows what is there, largest first, with ages.

`akr check` prints the total as a build fact, and `akr check --scratch-clean` fails when
anything prunable is left — the same shape as `--review-clean`. Neither deletes anything;
that is always your call, and always explicit.

If the workspace has no AKR, the point still stands: whatever scratch directory you were
told to use is persistent, and clearing it is part of finishing.

### What a call costs

The first `knowledge.*` call against a workspace derives git freshness for the whole
ledger and takes a second or two; later calls reuse that work for as long as `HEAD` and
the working tree are unchanged, and are near-instant. Any edit, stage or commit resets it.
So: batch related reads, prefer one `knowledge.context` over five `knowledge.get`s, and do
not poll. A slow call is the ledger being derived, not a hung server — wait for it rather
than firing a second one.
