## AKR (project knowledge)

A `.akr/` directory is a typed ledger of what the project decided, observed, and planned. Use the `knowledge.*` MCP tools, or the `akr` CLI (`knowledge.context` → `akr context`, `knowledge.validate` → `akr check` / `akr validate`). No `.akr/`: skip this, and do not run `akr init` uninvited.

If `knowledge.*` tools are missing from the MCP catalogue (Grok Build 1.0.25 drops dotted names and attaches with `tool_count: 0`; after akr-mcp 0.5 advertises `knowledge_search` etc. to Grok), use the CLI column — never hand-write `.akr` record bodies into scratch.

Session opened above the projects (a folder of checkouts)? `knowledge.*` tools then take `workspace` — the project directory, e.g. `"workspace": "Lege-ecosystem"` — on every call (D-045); the CLI equivalent is `--dir`.

### What the ledger is for

Part of your job this session is to help the next agent understand what happened in it: what the user asked, what you found in the code as it stood, and why that led to what you did. Explain in proportion to the expected impact of a change, not its line count. Never restate a diff — git has it.

- A step of work, a reason, a dead end: `knowledge.trace` on the work record (`akr trace <work> "<why>"`). One line each, about 25 words, as you go.
- A choice that binds the project: a `decision`. `context` says what forced it and what was rejected.
- A finding someone will re-read — a root cause, a measurement, an assumption that turned out false: an `observation`.
- A line of code: a comment beside it, never a record.

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
- Configuration changes: use `akr config --help` or `knowledge.config_get` /
  `knowledge.config_edit`, then `akr build`.

- `knowledge.get` reads a record; `knowledge.search` finds one. Ranking is not authority.
- Outside advice is `sources/`. `knowledge.source_search` / `knowledge.source_get` are **non-authoritative** until a record adopts them. Never edit a registered source.
- Never hand-edit `.akr/` or `docs/generated/`.
- Private paths, filenames and credentials: `knowledge.secret_put` with a private UTF-8
  input file (`akr secret put --from <file>`), then cite its opaque `{{secret:<id>}}` token
  in public text. Normal outputs show `[*]`; canonical retrieval retains opaque tokens.
  Only explicit `secret get` / `secret resolve --reveal` reads local values; absent data
  stays `[*]`. Do not reveal keys to remote agents or copy values into public records,
  evidence, sources or handoffs. `.akr/secrets/` and original input files stay private,
  outside Git and published archives. Share secrets only through a separate trusted channel.

### When knowledge changes

- New: `knowledge.propose`. Observations need `observed_at`, and `watches` if they can go stale. `policy` is the one kind for a standing rule, requirement or limit; `term`, `requirement`, `constraint`, `assessment` and `track` are retired — readable, never created.
- Changed: `knowledge.revise`. Never edit a record that is not `proposed`.
- Replacing a plan: `knowledge.supersede`, with a disposition for every unfinished child.
- Finished work: `knowledge.complete`. A `command` check you ran and saw pass records itself — `"checks": {"<id>": {"passed_at": "HEAD"}}` (`akr complete <key> --pass <id>`) — with no evidence record. `knowledge.evidence_add` is for a measurement, a failure, an artefact or a manual observation; its `artifact` must not be under `.agent/scratch` (move it, or `akr scratch keep` it first).
- Outside friction: `knowledge.papercut` with `about="<tool>"`.
- Handoff: `knowledge.validate`.

### Committing

`akr change begin --kind <kind> --summary "<imperative>" --primary <work-key>` (mechanical work: `--untracked-reason "<why>"` instead), `git add` the code, records, lock and views, then `akr change prepare --staged` and `akr git commit`. Trace entries the commit adds become its `AKR-Why:` trailers. Never write a commit id into the ledger.

### Handoff

`akr handoff worker` cuts a packet for a continuing subagent; `scout` for an independent investigation, `advisor` for a second opinion, `reviewer` for an adversarial check. AKR builds the packet; you launch the agent and hand it `akr handoff open <id>`.

Because inherited facts are cheap and inherited conclusions are not.

Open a session first (`akr handoff session begin --request-file <file>`, the file holding the user's words verbatim); each packet inherits it, so no child re-derives the project. Your shell runs backticks and `$(...)` inside double quotes before akr starts, so text that names a command goes in a file: `--task-file`, or every field as JSON via `--spec <file>`. `akr handoff --help` has the rest.

### Scratch

`.agent/scratch/` is gitignored and never auto-deleted. Before handoff: `akr scratch prune` (also drops live evidence citations of entries it removes, and of scratch paths already gone); `akr scratch keep <name> --reason "..."` to retain; `akr scratch list`. `akr check --scratch-clean` fails on prunable leftovers and deletes nothing. Same for any persistent scratch directory if the workspace has no AKR.

### Cost

The first `knowledge.*` call derives git freshness (~1–2s). Later calls are cheap until `HEAD` or the working tree changes. Batch reads; do not poll a slow call.
