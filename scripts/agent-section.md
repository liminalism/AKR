## AKR (project knowledge)

A `.akr/` directory is a typed ledger of what the project decided, observed, and planned. Use the `knowledge.*` MCP tools, or the `akr` CLI (`knowledge.context` → `akr context`, `knowledge.validate` → `akr check` / `akr validate`). No `.akr/`: skip this, and do not run `akr init` uninvited.

### Before a task

- Known planning key: `knowledge.context` with that key and the `paths` you will touch.
- Otherwise: `knowledge.start` with the task and those paths. Read the bundle, including contradiction and staleness warnings.
- Do not browse `.akr/records/` or `docs/generated/`. If the tools miss, log a papercut.

Consult at task and state-transition boundaries, not after every edit.

- Mechanical (fmt, a comment, a lock refresh): no planning read.
- Known work: one summary, implement, one batched update.
- Ambiguous: `knowledge.start`, one targeted read, one update.
- Planning or reconciliation: the full bundle.

### While working

- `knowledge.get` reads a record; `knowledge.search` finds one. Ranking is not authority.
- Outside advice is `sources/`. `knowledge.source_search` / `knowledge.source_get` are **non-authoritative** until a record adopts them. Never edit a registered source.
- Never hand-edit `.akr/` or `docs/generated/`.

### When knowledge changes

- New: `knowledge.propose`. Observations need `observed_at`, and `watches` if they can go stale.
- Changed: `knowledge.revise`. Never edit a record that is not `proposed`.
- Replacing a plan: `knowledge.supersede`, with a disposition for every unfinished child.
- Finished work: `knowledge.evidence_add`, then `knowledge.complete`. An evidence `artifact` must not be under `.agent/scratch`; move it or `akr scratch keep` it first.
- Friction: `knowledge.papercut`.
- Handoff: `knowledge.validate`.

### Calling an advisor

When you are asked to bring in a second model — "ask an advisor", "get a second opinion", "hand this to <model>", "use the AKR handoff workflow" — prepare an **advisor packet**. Do not write it a summary of what you found: an advisor is there to see what you did not, and a summary hands it your blind spot as a boundary. **Compress state, not search.**

1. Do the administrative preparation, and stop there. Session head, project state, repository map, verify the build, run the existing tests and benchmarks, collect what already exists. Do **not** wait until you think you understand the problem — that just moves the bottleneck.
2. `knowledge.handoff_create` (CLI: `akr handoff create`).
   - `task` is the user's request **verbatim**, never your reading of it.
   - `search_envelope` stays project-wide unless the *user* narrowed it.
   - `commands`, `baselines`, `constraints`, `evidence`, `artifacts`: what you established.
   - `worker_notes`: what you *think* — hypotheses, what you examined, what you did **not** examine, approaches, searches already run. The advisor cannot see these until it asks.
3. Hand the advisor the packet id and nothing else.
4. As the advisor: `knowledge.handoff_open` (`akr handoff open <id>`), review independently anywhere the envelope reaches, form your own view, and only then `knowledge.handoff_reveal`. Compare: what did either side miss?
5. Whatever the review made durable goes in the ledger — `knowledge.propose`, evidence, completion. The packet is disposable: `akr handoff discard <id>` when done.

`knowledge.handoff_open` and `handoff_verify` report `exact` or `drifted` and name what moved, so an advisor is never told about one tree while reading another. `handoff_expand <id> <section>` reads one part without re-opening the whole packet.

### Scratch

`.agent/scratch/` is gitignored and never auto-deleted. Before handoff: `akr scratch prune`; `akr scratch keep <name> --reason "..."` to retain; `akr scratch list`. `akr check --scratch-clean` fails on prunable leftovers and deletes nothing. Same for any persistent scratch directory if the workspace has no AKR.

### Cost

The first `knowledge.*` call derives git freshness (~1–2s). Later calls are cheap until `HEAD` or the working tree changes. Batch reads; do not poll a slow call.
