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

### Scratch

`.agent/scratch/` is gitignored and never auto-deleted. Before handoff: `akr scratch prune`; `akr scratch keep <name> --reason "..."` to retain; `akr scratch list`. `akr check --scratch-clean` fails on prunable leftovers and deletes nothing. Same for any persistent scratch directory if the workspace has no AKR.

### Cost

The first `knowledge.*` call derives git freshness (~1–2s). Later calls are cheap until `HEAD` or the working tree changes. Batch reads; do not poll a slow call.
