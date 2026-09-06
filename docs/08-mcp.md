# 08 — The MCP Tool Surface

How an agent reaches the ledger: the tool catalogue, its input and output schemas, how
diagnostics become tool errors, why reads and writes are separated, what idempotency
means here, and the `AGENTS.md` text that makes agents use any of it.

Normative for tool names, schemas, and the error mapping. The semantics of each
operation are normative in [`07-cli.md`](07-cli.md); the MCP server is a transport, not a
second implementation.

---

## 1. Shape

The MCP server is `akr-mcp`, a thin adapter over the same in-process `akr-core` model the
CLI uses. It runs in the workspace, over stdio, with no network and no state:

```
   agent ──MCP──> akr-mcp ──> akr-core ──> .akr/**.akr  (source of truth)
                                    └────> .akr/cache/index.sqlite (private)
```

Two invariants make the surface trustworthy:

- **One implementation.** `knowledge.context` and `akr context` call the same function
  with the same arguments and produce the same bundle. There is no MCP-specific
  assembly, ranking, or filtering. A behaviour that cannot be reproduced from the command
  line is a bug.
- **No privileged access.** The server has exactly the capabilities the CLI has. It
  cannot skip validation, cannot write an unformatted record, and cannot read anything an
  operator could not read by running a command.

The framing is the stdio transport's. A newline-delimited JSON-RPC document is still
accepted — and is what the differential harness speaks — so a pretty-printed request is
several malformed ones, and the server answers each fragment with its own parse error
rather than guessing where the message was meant to end. Official MCP clients send
`Content-Length` headers instead; the server accepts that framing too and answers in
kind, so a host that cannot parse NDJSON is not left attached with an empty tool list.

## 1a. Envelope and negotiation

Normative for the shape of every response, as §2–§4 are normative for what is inside one.
The distinction matters because the two fail differently: a wrong payload is a wrong
answer, and a wrong envelope is *no* answer — the client cannot deserialise the response
at all, drops the connection, and the host attaches with an empty tool list while the
server reports nothing wrong. Every MCP defect this project has shipped has been of the
second kind.

**`resultType`.** Every `tools/call` result and the `server/discover` result carries
`resultType`. Its vocabulary is closed — `complete`, `input_required`, `task` — and a
ledger read or write only ever produces `complete`. The field is how a client decides
which result type to parse, so a value outside the set is not an extension a client
ignores: it leaves the client with no branch to take and the entire response is refused.
A result from a server speaking an earlier revision omits the field, and a client must
read that absence as `complete`.

**Protocol versions.** The server offers exactly these, newest first:

| Version | Note |
|---|---|
| `2026-07-28` | current |
| `2025-11-25` | |
| `2025-06-18` | |
| `2025-03-26` | |
| `2024-11-05` | legacy |

`initialize` naming any of them is answered with **that** version, never a fallback. This
rule is load-bearing and its violation is silent: a version the server does not offer is
not refused, it is answered with the oldest one, and the session runs a generation behind
what both ends could have spoken without anyone being told. A version dropped by a future
MCP revision is removed here in the same change that removes it from the code.

**`server/discover`.** A client that speaks the discovery handshake sends this *before*
`initialize`, and retreats to the legacy handshake on one signal only: a `-32601`. The
result is a discovery result and carries `resultType`, `supportedVersions`, `capabilities`,
`ttlMs` and `cacheScope`, optionally `instructions` and `_meta`. `protocolVersion` and a
top-level `serverInfo` belong to an `initialize` result and must not appear; the server
identity travels in `_meta` under `io.modelcontextprotocol/serverInfo`. Answering this
method with an `initialize` result is worse than not implementing it, because a malformed
success gives the client nothing to fall back from.

**The tool-result envelope.** A `tools/call` result carries `content`, `structuredContent`,
`isError`, `resultType` and `_meta`, and nothing else. A refusal is a *successful*
JSON-RPC response with `isError: true` (§5) and travels through the same constructor as an
answer, so an envelope that is correct only on the happy path breaks the first time a tool
says no.

**How this section is enforced.** Two ways, deliberately, because neither subsumes the
other. `crates/akr-mcp/tests/conformance.rs` checks the envelope against closed sets
written out by hand, with no dependency on this server's code and none on any client — it
is the only layer that can catch an *omission*, such as a supported version quietly
missing. `tools/mcp-conformance/`, run by `scripts/verify-mcp-conformance.sh`, drives the
server with several major versions of a real MCP client library at once — it is the only
layer that can catch a client generation becoming stricter than the one before it, which
is how every defect here was actually found. The published JSON Schema catches neither
reliably: it types `resultType` as an unconstrained string, so the closed set above lives
in prose and in these two checks, not in a schema anyone can validate against.

## 2. Tool catalogue

| Tool | Kind | CLI equivalent | Idempotent |
| --- | --- | --- | --- |
| `knowledge.search` | read | `akr search` | yes |
| `knowledge.start` | read | `akr start` | yes |
| `knowledge.explain` | read | `akr explain` | yes |
| `knowledge.get` | read | `akr get` | yes |
| `knowledge.context` | read | `akr context` | yes |
| `knowledge.source_list` | read | `akr source list` | yes |
| `knowledge.source_add` | write | `akr source add` | by id |
| `knowledge.source_search` | read | `akr source search` | yes |
| `knowledge.source_get` | read | `akr source get` | yes |
| `knowledge.source_verify` | read | `akr source verify` | yes |
| `knowledge.source_supersede` | write | `akr source supersede` | by state |
| `knowledge.source_status` | read | `akr source status` | yes |
| `knowledge.source_dependents` | read | `akr source dependents` | yes |
| `knowledge.source_finalize` | write | `akr source finalize` | by state |
| `knowledge.impact` | read | `akr impact` | yes |
| `knowledge.validate` | read | `akr check` (alias `akr validate`) | yes |
| `knowledge.propose` | write | `akr propose` | by key |
| `knowledge.revise` | write | `akr revise` | no |
| `knowledge.supersede` | write | `akr supersede` | no |
| `knowledge.complete` | write | `akr complete` | by state |
| `knowledge.evidence_add` | write | `akr evidence add` | by key |
| `knowledge.evidence_add_many` | write | — | by all keys |
| `knowledge.papercut` | write | `akr papercut` | no |
| `knowledge.handoff_capsule` | read | `akr handoff capsule` | yes |
| `knowledge.handoff_session_begin` | write | `akr handoff session begin` | no |
| `knowledge.handoff_session_show` | read | `akr handoff session show` | yes |
| `knowledge.handoff_session_end` | write | `akr handoff session end` | by state |
| `knowledge.handoff_create` | write | `akr handoff worker` (and `scout`, `reviewer`, `advisor`) | no |
| `knowledge.handoff_list` | read | `akr handoff list` | yes |
| `knowledge.handoff_open` | read | `akr handoff open` | yes |
| `knowledge.handoff_expand` | read | `akr handoff expand` | yes |
| `knowledge.handoff_reveal` | write | `akr handoff reveal` | by state |
| `knowledge.handoff_verify` | read | `akr handoff verify` | yes |
| `knowledge.handoff_result` | write | `akr handoff result` | no |
| `knowledge.handoff_results` | read | `akr handoff results` | yes |
| `knowledge.handoff_coverage` | read | `akr handoff coverage` | yes |
| `knowledge.handoff_discard` | write | `akr handoff discard` | by state |

Thirty-seven tools. Notably absent:

- **No `knowledge.query`.** No arbitrary query language, and above all no SQL. Agents
  never see the SQLite cache (§6).
- **No `knowledge.build`.** Emitting views is a repository-maintenance act performed by
  a human or by CI, not by an agent mid-task.
- **No `knowledge.delete`.** Nothing deletes knowledge (`01-architecture.md` §9). The
  terminal-state transitions are reached through `revise` and `supersede`.

`knowledge.evidence_add` earns its place for the same reason `akr evidence add` does:
an evidence record has required slots a blank template cannot invent, and the first
agent to close out a milestone over MCP had to shell out to the CLI for exactly this
step. Like the command, the tool deliberately has **no field for what the evidence
verifies** (D-016) — the link is authored on the check (`verified_by`) or supplied to
`knowledge.complete`.

`knowledge.evidence_add_many` accepts the same payloads as an `evidence` array and
validates and commits them in one atomic write. Use it when one verification run closes
several checks; duplicate or existing keys reject the whole batch without a partial
write.

The fourteen `handoff_*` tools are the agent-to-agent context transport of
`docs/17-handoff.md` (D-040, D-041): a project capsule, a session capsule, packets in four
modes, results, and the coverage roll-up. Fourteen is a large addition to a catalogue this
document calls closed, and it earns the size for one reason — a subsystem a child agent
must use *instead of* `knowledge.start` has to be fully reachable over MCP, or children
fall back to re-orienting, which is the whole thing being prevented.

Their `write` kind means they change `.agent/handoffs/`, not `.akr/records/`: all of it is
disposable coordination state, and nothing about it is knowledge. `readOnlyHint` and
`--surface read` read the same flag, so a tool that creates a packet may not report itself
read-only.

`knowledge.handoff_open` resolves the inheritance chain and renders it, withholding the
parent's notes unless the mode discloses them; `knowledge.handoff_reveal` releases them and
records that it happened, so whether a pass was independent stays knowable. There is
deliberately no reveal-on-open argument — `akr handoff open --reveal` is one process and
one recorded act, while an MCP flag would make a tool declared read-only write. The
one-implementation invariant holds either way: `akr handoff open <id>` reproduces
`knowledge.handoff_open` exactly.

`knowledge.handoff_open` carries the largest budget on this surface (2,000 target, 3,500
hard). It is the one read whose whole point is that a second agent arrives knowing the
workspace, and because a packet is addressable the overflow path names a real continuation
— `knowledge.handoff_expand`, one section at a time — rather than truncating everything.

## 3. Read tools

### `knowledge.search`

```jsonc
// input
{
  "query": "frame budget",            // required
  "kinds": ["constraint","observation"],  // optional filter
  "states": ["active","verified"],        // optional filter
  "limit": 20,                            // optional, default 20, max 100
  "offset": 0                             // optional; continue from next_offset
}
// output
{
  "index_stale": false,
  "results": [
    { "key": "sys.constraint.frame-budget-16ms", "rev": 1, "kind": "constraint",
      "state": "active", "title": "16 ms frame budget at p99",
      "score": 0.91, "stale": false, "at_risk": false }
  ],
  "count": 2, "has_more": false, "next_offset": null
}
```

`score` is advisory and comparable only within one result set. **Search ranks; it never
authorises.** A record appearing here has no standing it did not already have, and
nothing enters a context bundle because it matched a query
([`09-context-assembly.md`](09-context-assembly.md) §1).

If the MCP output budget cannot carry the requested number of ranked records, the
response still contains the largest useful first page that fits. It sets `truncated`,
`has_more`, and `next_offset`, and its `continuation` repeats the original query and
filters with the advancing offset. A budget limit must never replace every result with a
notice. `knowledge.source_search` uses the same `limit`/`offset` paging contract while
retaining its NON-AUTHORITATIVE standing on every page.

Before returning results, search refreshes a missing or stale disposable index from the
currently loaded ledger. Successful responses therefore carry `index_stale: false`.
Under `--no-rebuild`, a needed refresh is refused with `AKR-I031` and the cache remains
untouched.

### `knowledge.get`

```jsonc
// input
{ "ref": "@sys.policy.tandem-work",   // any of the four forms of D-009
  "history": false, "relations": true }
// output
{
  "key": "sys.policy.tandem-work", "rev": 1, "kind": "policy",
  "class": "normative", "state": "active", "is_head": true,
  "title": "Engine and simulator advance in tandem",
  "scope": [ { "form": "all" } ],
  "topic": "tandem-work",
  "slots": { "rule": "No engine change lands without …" },
  "claims": [ { "anchor": "lag-bound", "text": "…", "retired": false } ],
  "relations": {
    "outbound": [ { "relation": "exceptions", "ref": "@sys.track.lighting/1" } ],
    "inbound":  [ { "relation": "implements", "ref": "@sys.decision.view-generation/1" } ]
  },
  "freshness": { "stale": false, "at_risk": true, "depth": 2,
                 "path": ["@sys.assessment.projection-gaps",
                          "@sim.obs.projection-gaps"] },
  "source_text": "record sys.policy.tandem-work/1 : policy {\n …"
}
```

`source_text` is the canonically formatted record. An agent that wants to reason about
the ledger's own syntax reads that, not a file.

### `knowledge.context`

```jsonc
// input
{ "goal": "sys.milestone.m3-playable-day",   // required; milestone|work|track
  "paths": ["sim/src/project/**"],           // optional
  "budget_tokens": 8000,                     // optional
  "format": "json" }                         // "json" | "text"
// output
{
  "goal": { "key": "…", "rev": 1, "title": "…" },
  "commit": "e806b3f54a2d7091c5e13b8a26f490dc7b135e64",
  "sections": [
    { "id": "goal",            "records": [ … ] },
    { "id": "milestone",       "records": [ … ] },
    { "id": "work-items",      "records": [ … ] },
    { "id": "plan-of-record",  "records": [ … ] },
    { "id": "normative",       "records": [ … ] },
    { "id": "dependencies",    "records": [ … ] },
    { "id": "acceptance",      "checks":  [ … ] },
    { "id": "observations",    "records": [ … ] },
    { "id": "questions",       "records": [ … ] },
    { "id": "contradictions",  "pairs":   [ … ] },
    { "id": "staleness",       "warnings":[ … ] }
  ],
  "excluded": { "superseded": 2, "archived": 1, "terminal": 4,
                "out_of_scope": 12 },
  "truncated_prose": [], "estimated_tokens": 5120
}
```

`budget_tokens` controls context assembly and disables the MCP adapter’s separate fixed
ceiling for this call. The same rule applies to `knowledge.start` and its handoff
assembly. The server does not assemble an 8,000-token bundle and then discard it behind
a smaller transport limit. When the argument is omitted, the normal per-tool ceiling
applies; an oversized result retains a compact preview and an actionable continuation
instead of withholding all content.

The budget is a promise about the **delivered** result — both halves, the readable text
and the structured metadata that repeats it — and not about the records the assembler
selected, which is a much smaller number. Assembly measures what it rendered and, when the
result overshoots, re-assembles to proportionally less, up to three passes. It used to
measure only the content it had chosen, and a caller who asked for 4,500 tokens received
close to three times that and a transport-truncated bundle. Because the promise belongs to
the bundle rather than to this transport, `akr context --budget` keeps it identically
(§1).

Every budgeted bundle reports what happened, as `budget`:

```json
"budget": { "requested_tokens": 4500, "delivered_tokens": 4887,
            "note": "the bundle's mandatory content … is never truncated (V-123) …" }
```

The `note` appears only when the request could not be met. Mandatory content — keys,
states, relations, acceptance verdicts, contradictions, staleness — is never truncated, so
a goal with a large normative scope costs what its structure costs however small the
budget. Narrowing `paths` reduces it; lowering `budget_tokens` again does not.

Sections appear in the fixed order above, always, whether or not they are empty. The
membership of each is computed by the algorithm of `09-context-assembly.md` §4 — a pure
function of (ledger, commit, request).

### `knowledge.impact`

```jsonc
// input — exactly one of `ref` or `git_diff`
{ "ref": "@sim.obs.projection-gaps", "depth": null }
{ "git_diff": "5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285..e806b3f54a2d7091c5e13b8a26f490dc7b135e64" }
// output
{
  "mode": "ref",
  "dependents": [
    { "key": "sim.work.rewrite-projection", "rev": 1, "depth": 1,
      "via": "depends_on", "path": ["@sim.obs.projection-gaps"] },
    { "key": "sys.assessment.projection-gaps", "rev": 1, "depth": 1,
      "via": "supported_by", "path": ["@sim.obs.projection-gaps"] }
  ],
  "newly_stale": [], "newly_at_risk": []
}
```

The tool an agent calls **before** proposing a supersession, to see what it is about to
disturb.

### `knowledge.validate`

```jsonc
// input
{ "review_clean": false }
// output
{ "ok": true, "diagnostics": [],
  "counts": { "records": 40, "revisions": 42, "stale": 2, "at_risk": 4 } }
```

Runs stages A–D over the ledger as it stands on disk. An agent calls it after a batch of
writes to confirm the ledger is still coherent, and before handing work back to a human.

## 4. Write tools

All four writes go through the pipeline of [`07-cli.md`](07-cli.md) §4 — parse, apply,
validate the *result*, canonically format, write atomically — and all four fail
completely rather than partially. A rejected write leaves the working tree byte-identical.

### `knowledge.propose`

```jsonc
// input
{ "key": "sim.obs.projection-rewrite", "kind": "observation",
  "title": "Projection pass rewritten; coverage measured again",
  "slots": { "statement": "…", "observed_at": "git:e806b3f5…",
             "watches": ["sim/src/project/**"] },
  "relations": { "derived_from": ["@sim.obs.projection-gaps/1"] },
  "claims": [ { "anchor": "coverage-restored", "text": "…" } ],
  "sources": [ { "kind": "internal", "path": "sim/src/project/mod.rs" } ] }
// output
{ "key": "…", "rev": 1, "state": "verified", "path": ".akr/records/sim/observations.akr",
  "written": true, "lock_stale": true, "content_hash": "sha256:…" }
```

For registered outside material, `sources` also accepts the exact locator returned by
`knowledge.source_search`: `document`, `start_byte`, `end_byte`, `start_line`, and
`end_line`, plus optional `role`, `excerpt_hash`, and `use`. This is the reliable adoption
path: retrieval remains non-authoritative, while `knowledge.propose` or
`knowledge.revise` records the project’s explicit interpretation and stable provenance.

A citation may also name `document`, `start_line` and `end_line` **alone**. The stored
record still carries all four coordinates — bytes are what resolves, lines are what a
reader opens the file at, and a half-written range would point somewhere nobody chose —
but the byte offsets are read off the registered bytes rather than demanded of the author,
who reads a document by line and would otherwise count bytes by hand. The range covers
whole lines, including the newline that ends the last, and carries the `excerpt_hash` of
exactly the bytes it selected, so a located citation verifies itself. Lines without a
`document` are refused: there is nothing to read the offsets from. A document that retains
only cited ranges or metadata cannot be located in either, and says so.

If the requested `end_line` is blank after nonblank cited text, authoring normalizes both
the stored `end_line` and `end_byte` to the preceding covered line. This is the same
trailing-newline convention `AKR-S022` validates, so a successful proposal cannot become
invalid merely because the caller ended its line selection on a blank separator.

The command-line equivalent is `akr source get <id> --lines a:b`, which reports the same
locator alongside the text.

Creates revision 1 of a **new** key, in its class's initial state. An existing key is an
error — the tool will not silently turn a proposal into a revision.

The callable schema derives every kind's required and optional slots from the vocabulary
table. In particular, work uses required `intent`, decision uses required `decision`, and
`topic` is a separate normative-only field. `scope` accepts compact strings (`"all"`, a
bare path glob, or `"@key"`) and exposes typed object alternatives as a `oneOf`:
`{form:"all"}`, `{form:"path",glob:"src/**"}`, or `{form:"ref",ref:"@key"}`.

`acknowledged: true` marks a declared `contradicts` edge as knowingly tolerated (D-023).
Two live records that contradict each other fail V-023 (`AKR-R041`) unless one of them
carries it, and acknowledging is a legitimate ledger state rather than a workaround — the
alternative an agent reaches for when the marker is unreachable is dropping the relation,
which is the contradiction going unrecorded that the rule exists to prevent.

A milestone requires a non-empty `acceptance` block to exist at all (V-008), so
`knowledge.propose` accepts one directly — an array of `{id, statement, method, command?,
verified_by?}`, one entry per check:

```jsonc
{ "key": "product.milestone.m2", "kind": "milestone", "title": "M2: …",
  "acceptance": [
    { "id": "full-day-demo", "statement": "…", "method": "manual" } ] }
```

`knowledge.revise` accepts the same field to replace a record's acceptance block; omit it
to keep the head's checks.

All four writes return this payload, and three of its fields describe the write rather
than the request:

- `rev` is the revision the write **produced**, not the one it started from. A revision
  and a supersession each touch two revisions of the key — the retired head and its
  successor — and it is the successor an agent's next `base_rev` has to name.
- `state` and `content_hash` describe the record as it landed on disk, read back after
  the write rather than predicted from the request. For a lifecycle move they are the
  only way the agent learns the move took.
- `lock_stale` is `true` after every write, because no write operation may invent a build
  (D-014). Saying so here is the difference between an expected `AKR-R052` on the next
  `knowledge.validate` and a confusing one.
- `next` names `akr build` and explains that it refreshes `akr.lock` and generated views
  before validation. This makes the required post-write maintenance executable rather
  than leaving the agent to infer it from `lock_stale`.
- `notes` is present only when the write leaves something for the caller to look at, and
  is advisory: never a diagnostic, never blocking, never a reason a strict write fails.
  Today it carries one thing. Revising a record that stays `completed` puts *every* one of
  its acceptance references back in question at once, because V-020 compares each one's
  `observed_at` against the commit that last changed the record's content — so refreshing
  three checks of four leaves the fourth to surface as `AKR-R022` on a later build, long
  after the session that could have refreshed it. The pipeline cannot settle the question,
  since the commit this revision will land in does not exist yet; it can name the
  references in play at the one moment somebody is looking. `akr revise` prints the same
  lines.

### `knowledge.revise`

```jsonc
{ "key": "sim.obs.projection-gaps",
  "slots": { "observed_at": "git:e806b3f5…" },
  "state": null,                       // optional lifecycle move
  "retired_claims": ["old-anchor"],
  "base_rev": 1 }                      // required: optimistic concurrency
```

`base_rev` must equal the current head revision. If another writer has revised the key
since the agent read it, the tool fails with a conflict rather than clobbering
(`AKR-C033`). This is the only concurrency control the surface has, and it is enough
because the underlying store is a git working tree that a human is also watching.
An explicit `state` lands on the successor even when the old head is sealed. Without an
explicit state, a content revision of settled knowledge starts `proposed` for review.

Omitted fields carry forward from the head — slots, relations, scope, claims, acceptance,
`topic`, `acknowledged` and `sources` alike. The last three are the ones worth naming,
because each was for a while dropped by the merge rather than carried: provenance is the
part of a record the ledger cannot reconstruct from anything else, `topic` is a normative
record's exclusivity handle (D-004b), and `acknowledged` is what keeps a tolerated
contradiction from failing V-023 on the next build. An edit that renames a policy must not
silently retire a marker it never mentioned. Supplying any of them replaces the head's
value outright.

### `knowledge.supersede`

```jsonc
// First propose the complete replacement under its accurate key, then:
{ "old_key": "sys.work.m3-plan", "new_key": "sys.work.day-loop-plan",
  "dispositions": [
    { "child": "@sys.work.m3-lighting-pass", "outcome": "carried_forward",
      "into": "@sys.track.lighting", "note": "Lighting is standing work." },
    { "child": "@sys.work.m3-audio-pass", "outcome": "intentionally_dropped" }
  ] }
```

For different keys, `new_key` must resolve to a proposed record of the same kind and
`slots` is omitted: its content belongs in the preceding `knowledge.propose`. The
supersession call atomically adds the pinned edge and retires the old head.

If any unfinished `part_of` child lacks a disposition, the tool fails with `AKR-R014` and
**lists the children in the error payload**, so the agent's next message can name them.
That is the moment the design cares most about (D-017), and the API is shaped to make
answering easy and skipping impossible.

### `knowledge.complete`

```jsonc
{ "key": "sys.milestone.m3-playable-day",
  "checks": { "no-placeholder-assets": "@sys.evidence.asset-audit/1" } }
```

Fails with `AKR-R022` naming each unsatisfied check, including whether the failure was
"no passing evidence" or "evidence predates the last content change" (D-016).

### `knowledge.evidence_add`

```jsonc
{ "key": "sys.evidence.asset-audit",
  "result": "pass",                       // pass | fail | inconclusive
  "method": "command",                    // manual | command | observation
  "command": "cargo run -p tools -- audit-assets",
  "summary": "Zero placeholder assets on the day-loop path",
  "observed_at": "e806b3f54a2d7091c5e13b8a26f490dc7b135e64" }  // defaults to HEAD
```

Creates an `evidence` record, exactly as `akr evidence add` does. There is no field for
what the evidence verifies, and that absence is the tool doing its job (D-016): the
check names its evidence in `verified_by`, or `knowledge.complete` supplies the link —
one direction, one source of truth. The typical closing sequence is `evidence_add`,
then `complete` with `checks` citing the returned revision.

### `knowledge.evidence_add_many`

```jsonc
{ "evidence": [
    { "key": "sys.evidence.native-build", "result": "pass", "method": "command",
      "command": "cargo check --workspace", "summary": "Workspace compiled" },
    { "key": "sys.evidence.tick-contract", "result": "pass", "method": "command",
      "command": "cargo test tick_contract", "summary": "Tick contract passed" }
] }
```

The array contains 1–100 ordinary evidence-add payloads. AKR resolves and checks their
commits together, validates the resulting ledger once, and commits every record or none.

### `knowledge.papercut`

```jsonc
{ "agent": "claude",
  "message": "Ran knowledge.search right after a write and got stale results;               akr build in between fixed it.",
  "namespace": "sys" }   // optional; defaults to where this project's papercuts go
```

Logs a small friction as a `papercut` record (D-027): what you were doing, what got in
the way, and — as a bonus — a guess at the cause or fix. The message is the whole
ceremony: the key, the commit, the author and the date are filled in by the tool. Not
idempotent, deliberately: a log never refuses an entry, so the same message twice is two
records with distinct keys. The aggregate renders to `PAPERCUTS.md` on the next build.

## 5. Error mapping

Every failure is an MCP tool error whose payload is the JSON diagnostic array of
[`07-cli.md`](07-cli.md) §5, plus a coarse class the agent can branch on without knowing
the code table:

```jsonc
{ "error": {
    "class": "invariant",
    "summary": "superseding plan does not dispose of an unfinished child",
    "diagnostics": [ { "code": "AKR-R014", "severity": "error", "rule": "V-017",
                       "message": "…", "path": "…", "line": 61, "column": 1,
                       "help": "add a disposition block" } ],
    "retryable": false,
    "wrote": false } }
```

| Class | Codes | What the agent should do |
| --- | --- | --- |
| `usage` | `AKR-C001`–`AKR-C005`, `AKR-C041`, `AKR-X041` | Fix the call. Never retry unchanged. |
| `not_found` | `AKR-L001`, `AKR-L004`, `AKR-X001`, `AKR-E003` | The reference is wrong. Search or re-read. |
| `schema` | `AKR-P***`, `AKR-T***` | The proposed content is malformed. Fix and resubmit. |
| `invariant` | `AKR-R***`, `AKR-L006`, `AKR-L012`, `AKR-L021`, `AKR-L031` | The ledger would become incoherent. This usually needs a *design* decision, not another attempt — surface it to the human. |
| `conflict` | `AKR-C032`, `AKR-C033` | Re-read the head and rebase the edit. Retryable once. |
| `environment` | `AKR-C011`, `AKR-C012`, `AKR-G001`, `AKR-G003`, `AKR-I003`, `AKR-I031`, `AKR-I032` | Not the agent's fault and not fixable by it. Stop and report. |
| `degraded` | `AKR-X033`, `AKR-G004`, `AKR-X012`, `AKR-X022` | Warnings under `--lenient`; the call succeeded with a caveat that belongs in the agent's report. |
| `internal` | `AKR-X099` | The server contained an unexpected tool panic. Retry once; if it repeats, report the bug. |

`wrote` is always present on a write tool's error and is always `false`. An agent never
has to guess whether a failed write left something behind.

An internal panic is contained to the request that triggered it and returned as
`AKR-X099`; it does not terminate the stdio server or prevent the next request from being
handled.

## 6. Why agents never see SQLite

D-019 in one paragraph, because it is the boundary most likely to be eroded by
convenience.

`.akr/cache/index.sqlite` is a private implementation detail of pipeline stage E. It is
gitignored, rebuilt whenever the schema version or the source-graph hash changes, and
safe to delete at any instant. Exposing it — even read-only, even "just for search" —
would convert its schema into a public interface with compatibility obligations, and the
ledger would acquire a second source of truth that is sometimes newer and sometimes older
than the first. Every `AKR-I` diagnostic in
[`../spec/diagnostics/codes-runtime.md`](../spec/diagnostics/codes-runtime.md) exists
because the cache is allowed to fail loudly and be rebuilt; none of that is true of an
interface someone depends on.

The practical consequence for tool design: whenever an agent wants something the
tools cannot express, the answer is a new tool with a defined contract, never a query
hole. `knowledge.search` is the deliberate, narrow escape valve, and it returns records,
not rows.

## 7. Read/write separation and idempotency

**Separation.** Read tools never touch `.akr/records/`. They may rebuild the index cache
as a side effect (that is what a cache is), and under `--no-rebuild` they will not even
do that. A read tool's effect on the repository's committed content is always nil.

**Idempotency.** The read tools are idempotent in the strong sense: called twice against
the same (sources, commit, tool version), they return byte-identical results. That
follows directly from the determinism contract (`01-architecture.md` §4) and is what lets
an agent cache a bundle for a session.

The write tools are idempotent to the extent the operation allows, and the schema is
shaped to make the difference explicit:

- `knowledge.propose` is idempotent **by key**: a second call with the same key fails
  rather than creating a second record.
- `knowledge.revise` is not idempotent — that is what `base_rev` is for. A retry with a
  stale `base_rev` fails with `conflict`; a retry with the new one applies the edit
  again, which is usually not what was wanted.
- `knowledge.supersede` and `knowledge.complete` are idempotent **by state**: superseding
  an already-superseded record, or completing an already-completed one, fails with an
  invariant error rather than doing it twice.

There is no transaction spanning arbitrary tool calls. Evidence is the deliberate bulk
exception: `knowledge.evidence_add_many` lands several evidence records in one validated
transaction. Other records are proposed one at a time, and every write still validates
the resulting ledger.

## 8. The `AGENTS.md` protocol text

This is the recommended minimal `AGENTS.md` section. It is deliberately protocol only —
no philosophy, no data model, no examples — because an agent reads it every session and
every extra line competes with the task. Everything it needs to know beyond this is
reachable through the tools themselves.

The installed text is `scripts/agent-section.md`. Keep that file and this
section in sync; the setup scripts copy it between `<!-- AKR_START -->` and
`<!-- AKR_END -->`.

```markdown
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

### Handoff

`akr handoff worker` invokes a subagent. `akr handoff scout` invokes an independent agent. `akr handoff advisor` invokes a second opinion. `akr handoff reviewer` invokes an adversarial check.

Because inherited facts are cheap and inherited conclusions are not.

Open a session first (`akr handoff session begin --request "<the user's words, verbatim>"`); each packet inherits it, so no child re-derives the project. `akr handoff --help` has the rest.

### Scratch

`.agent/scratch/` is gitignored and never auto-deleted. Before handoff: `akr scratch prune`; `akr scratch keep <name> --reason "..."` to retain; `akr scratch list`. `akr check --scratch-clean` fails on prunable leftovers and deletes nothing. Same for any persistent scratch directory if the workspace has no AKR.

### Cost

The first `knowledge.*` call derives git freshness (~1–2s). Later calls are cheap until `HEAD` or the working tree changes. Batch reads; do not poll a slow call.
```

That is the whole protocol. One collated first read replaces chronological record
searching; focused context and detail remain explicit follow-ups.

## 9. Walkthrough: an agent working on M3

Against [`../examples/save-your-skin/`](../examples/save-your-skin/), whose inventory is
frozen in its `MANIFEST.md`. The agent has been asked to rewrite the projection pass.

**1. Get context.**

```jsonc
→ knowledge.context { "goal": "sys.milestone.m3-playable-day",
                      "paths": ["sim/src/project/**"] }
```

The bundle returns M3, its plan of record `sys.work.m3-plan/2`, the live in-scope
policies and constraints, `sim.work.rewrite-projection` in `blocked` state with the
question that blocks it, both M3 acceptance checks with `full-day-demo` marked satisfied,
`sim.obs.projection-gaps` with a staleness warning, and the acknowledged contradiction
between `sim.obs.timestep-drift` and `sim.evidence.determinism-suite-pass`. It does not
return `sys.work.m3-plan/1`, `lege.decision.renderer-boundary/1`, or
`sys.policy.weekly-demo` — superseded, superseded, and archived respectively.

The agent now knows three things it could not have learned from any Markdown file: the
work item is blocked and by what; the observation it would naturally rely on is stale and
why; and one of the two acceptance checks is already met.

**2. Check what the blocker is.**

```jsonc
→ knowledge.get { "ref": "@sim.question.timestep-vs-budget", "relations": true }
```

An `open` question — "does a 4 ms timestep fit the frame budget?" — with `blocks` edges
to `sim.decision.timestep-4ms` and to `sim.work.rewrite-projection`. The agent cannot
proceed past it without an answer, and it now knows to say so rather than guessing.

**3. See what the rewrite would disturb.**

```jsonc
→ knowledge.impact { "ref": "@sim.obs.projection-gaps" }
```

Three dependents: at depth 1, `sim.work.rewrite-projection` itself (via `depends_on` —
the agent's own work item rests on the stale observation) and
`sys.assessment.projection-gaps` (via `supported_by`); at depth 2,
`sys.policy.tandem-work`. Rewriting the projection pass will require re-observing, and
three records downstream will need review when it does.

**4. Record what it found.**

```jsonc
→ knowledge.propose {
    "key": "sim.obs.projection-rewrite-scope", "kind": "observation",
    "title": "The projection pass has three callers, all inside sim",
    "slots": { "statement": "…", "observed_at": "git:e806b3f5…",
               "watches": ["sim/src/project/**"] },
    "relations": { "derived_from": ["@sim.obs.projection-gaps/1"] } }
```

Note the pinned `derived_from`: the new observation records what it was derived from at
the revision it actually read, which is what makes the provenance auditable later. The
`watches` glob means this observation will itself go stale when the code moves again —
the agent is writing knowledge that knows how to expire.

**5. Hand back.**

```jsonc
→ knowledge.validate { }
← { "ok": true, "diagnostics": [],
    "counts": { "records": 41, "revisions": 43, "stale": 2, "at_risk": 4 } }
```

Still 2 stale and 4 at risk: the new observation is current, and nothing it touched
became stale. The agent reports that the rewrite is blocked on
`@sim.question.timestep-vs-budget` and stops — which is the outcome the whole design
exists to produce, in place of a confident rewrite built on a stale observation.

---

Next: [`09-context-assembly.md`](09-context-assembly.md) for exactly what step 1
computed, or [`07-cli.md`](07-cli.md) for the same operations from a shell.
