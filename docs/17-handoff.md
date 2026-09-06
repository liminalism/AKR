# 17 — Handoff

How work moves between agents: the project and session capsules, the four packet modes,
the blind read, results, and coverage.

Normative for the capsule and packet shapes, the mode disclosure policy, the reveal
contract, the result shape and the drift statuses. Advisory for rendering and for how a
harness dispatches to a particular agent. Decided in D-040 and D-041.

---

## 1. The primitive

> **A bounded, snapshot-bound, inheritable context object for transferring work between
> agents.**

An advisor review is one application of it. A subagent assignment is another. An
adversarial review is a third. They are the same object with different **disclosure**,
because the axis that actually separates them is epistemic — how much of the parent's
*judgement* the child may see — and not what the child is called.

Building them as three subsystems would have produced three vocabularies for one idea and
three places to fix the same bug.

## 2. The two failures it is built against

**A summary handoff destroys the reason for making it.** If a second agent's value is
finding what the first one missed, then a summary of what the first one noticed is the
worst possible input. The parent becomes a perceptual bottleneck: the child inherits its
framing, searches where it searched, and confirms rather than forms a view. The parent's
blind spot silently becomes the child's boundary, and afterwards nobody can tell, because a
narrowed task reads exactly like a narrow one.

**No handoff at all wastes the child's most expensive tokens.** An agent arriving cold asks
questions with deterministic answers: what is this repository, what is the toolchain, where
are the tests, what is the build command, what is AKR, what work is current, what does
`base_rev` mean. Five children asked to review one project pay that five times over — and
they do not always arrive at the same answer, which costs more than the tokens, because the
disagreements are invisible.

> **Compress state, not search.**

## 3. Three levels

```text
PROJECT CAPSULE   pc-…    what is true of this repository
      ↓
SESSION CAPSULE   sx-…    what is true of this session
      ↓
PACKET            wk-… sc-… rv-… ad-…    what is true of this assignment
      ↓
RESULT            rs-…    what came back
```

**Inheritance is by reference.** A packet names what it inherits and copies none of it;
`handoff open` resolves the chain at read time. A session capsule corrected after five
packets were cut corrects all five. Copying would have produced five snapshots drifting
apart silently, which is the failure the capsules exist to remove.

Resolution is transitive and cycle-safe: `C --inherit B`, where `B --inherit A`, reaches
`A`. And it follows each inherited packet to the **result filed against it**, so what a
predecessor actually read, searched, ran and changed reaches its successor without the
parent re-typing it. That coverage is fact and every mode gets it. The predecessor's
findings, uncertainties and follow-up are judgement, and go the way the parent's own notes
go: a `worker` sees them on open, the other modes after a recorded `reveal`.

### The project capsule

Derived from the checkout, not written:

| | |
| --- | --- |
| toolchain | detected from the manifest, with the edition where there is one |
| layout | top-level areas and their roles; workspace members one level down |
| commands | build, test, lint, format, for the detected toolchain |
| namespaces | from `project.akr` |
| authoritative | `.akr/records/**`, `spec/**`, `sources/**` |
| generated | `docs/generated/**`, `.akr/cache/**`, `.akr/akr.lock` |
| instructions | `AGENTS.md`, `CLAUDE.md`, and their siblings, where present |
| boundaries | the one field nothing derives; supplied with `--boundary` |

Everything but the last is already visible in the checkout, which is exactly why deriving
it is right: the question is deterministic, so it should be answered once. The id is a
digest over the derived content, so it changes exactly when the project's shape does.

It is deliberately shallow. A capsule summarising what the code *does* would be a model's
judgement wearing a fact's clothes, and the whole subsystem rests on keeping those apart.

### The session capsule

`HEAD` and its subject, the workspace fingerprint, the ledger revision, the rendered
session head, the governing goal and records, constraints, commands run with what they
produced, baselines, evidence, artefacts — and **the user's request, verbatim**.

Open one before delegating. Every packet cut afterwards inherits it.

### The packet

Mode, role, task, scope, what is already known, what must not be repeated, what to return,
the parent's notes, and the workspace fingerprint at the moment it was cut.

## 4. The modes

| Mode | Prefix | Sees the parent's notes | For |
| --- | --- | --- | --- |
| `worker` | `wk-` | yes, on open | continuing a job the parent began |
| `scout` | `sc-` | on request, recorded | exploring an assigned scope independently |
| `reviewer` | `rv-` | on request, recorded | checking what the parent produced |
| `advisor` | `ad-` | on request, recorded | an independent judgement, then a comparison |

The three withholding modes are mechanically alike on purpose. Their difference is the
contract each states and what each is asked to return; inventing three disclosure policies
to make them look different in code would have been decoration. What matters is that the
*default* is right for each. A worker made to ask for the parent's coverage would duplicate
work. A scout handed the parent's hypotheses would stop being a scout.

Every mode can reveal, and **the reveal is recorded**. That is what makes the comparison
worth anything: once you know a child read blind and then saw the parent's notes, *"what
did either side miss?"* is a question with an answer, and across tasks it accumulates into
evidence about where each model actually sees.

`akr handoff open --reveal` exists for a child that wants the notes at once. It takes the
same recorded path. The MCP surface has no equivalent argument: `knowledge.handoff_open` is
declared read-only, and a flag that quietly wrote the reveal marker would make that
declaration false.

## 5. The task is narrowed, never replaced

A parent narrowing a job for a child is what delegation *is*. Substituting its own reading
of the user's request for the request is not — and from inside the child the two are
indistinguishable.

So a packet renders both:

```text
ORIGINAL REQUEST (from the session, verbatim)
  Review this project and optimise for performance, both memory and CPU.

YOUR ASSIGNMENT (narrowed from the above)
  Independently inspect the git and freshness layer for avoidable process spawns.
```

The narrowing stays visible to the agent it was done to, which is the only place it can be
checked. When the assignment and the request are the same text, only one heading renders.

## 6. The scope defaults to the whole project

```text
scope: ["**"]
```

A scope drawn from what the parent happened to examine would hand the child that parent's
blind spot as a boundary — the one thing an independent pass exists to escape. A parent
that means to narrow says so, and `handoff coverage` records what the narrowing left out.

The scope is a permission, not an instruction. The packet says how the kitchen is
organised; it never says which drawer the answer is in.

## 7. The workspace fingerprint

A capsule describes a tree. Parents keep working. So a session, and every packet, records:

```text
head            a85e038b…            (full 40-hex)
head_subject    fix(mcp): Derive the output schema from the tool catalogue
ledger_revision sha256:6bdfb86…
dirty           <path> <sha256 of its bytes>   for every path git reports changed
```

Not every source byte: `HEAD` plus the source-graph hash plus a digest per dirty path is
enough to detect drift, costs one status call and a read of the files git already named,
and leaves the repository itself as the thing the child reads.

`handoff open` and `handoff verify` recompute it and compare against the **packet's**
fingerprint, not the session's. The session records where the delegation began; the
parent then builds, tests and edits before it cuts a packet, and a child asking "does this
still describe the tree I am looking at" is asking about the tree it was handed. Measured
from the session start, the answer would be `drifted` for the parent's own preparatory
work, and a signal that fires for the wrong reason is one the child learns to ignore.
`handoff session show` still reports drift from the session's own fingerprint.

| Status | Meaning |
| --- | --- |
| `exact` | the workspace is as the packet describes it |
| `drifted` | `HEAD` moved, the ledger moved, or named paths changed |

Drift never changes the exit status. It is a fact about the working tree, not a
contradiction in the ledger, exactly as staleness is under D-024. The handoff store is
excluded from the fingerprint, so writing a packet cannot make it drift against itself.

## 8. Results

A parent does not need a transcript. *"I started by looking at…, then I ran…, the command
produced…"* is administrative narration with almost no value upstream, and it arrives
thousands of tokens at a time.

```text
RESULT rs-3594954ae2b3 for sc-958c35294253

FINDINGS       ranked by the child, most significant first
EVIDENCE       source locations, records, artefacts
CHANGES        or "none" — explicitly, because forgetting to say is different
UNCERTAINTIES  absent uncertainty is itself a claim
FOLLOW-UP      what to do next, or have somebody else do
COMMANDS       what was run, and what it gave
COVERAGE       read / searched / tested / not examined
```

## 9. Coverage is the field that pays for itself

An agent spends ten minutes understanding a subsystem, returns three paragraphs, and its
orientation evaporates. Worse: five agents told to review one project converge on the same
obvious doorway, so the fifth costs as much as the first and adds nothing.

`akr handoff coverage` rolls up every result in the session:

```text
read
  crates/akr-core/src/git/mod.rs  (sc-958c35294253)
searched
  Command::new  (sc-958c35294253)
tested
  cargo test -p akr-core  (sc-958c35294253)
nobody has examined
  crates/akr-core/src/freshness/mod.rs
no result filed yet
  wk-cf65b83e5555
```

The next wave is then delegated against evidence rather than a hunch.

**`not_examined` is as load-bearing as `read`.** A path nobody mentions is ambiguous: it
might have been ruled out in a second or never noticed. A path a child *names* as unopened
is a fact, and a fact is delegable. So the aggregate reports as untouched only what some
child named and no child read — silence is not evidence of absence, and claiming it would
make the aggregate lie in the direction that costs a wasted agent.

## 10. A child does not call `knowledge.start`

A root agent orients itself. A child inherits.

```text
root:   knowledge.start(...)
child:  knowledge.handoff_open({ packet: "sc-…" })
        knowledge.handoff_expand(...) when it needs more
```

AKR cannot see process ancestry, so it cannot *know* which it is talking to. What it can do
is say so when a session is open, and say it precisely when the harness sets
`AKR_HANDOFF_PACKET`:

```text
handoff      you hold packet sc-958c35294253 — `akr handoff open sc-958c35294253`
             carries this orientation already; this call re-derived it.
```

A line rather than a diagnostic. What is open in `.agent/handoffs/` is a fact about the
working tree and never a ledger diagnostic, the same standing scratch has under D-036.

## 11. Where it lives

```text
.agent/handoffs/project.json          the project capsule
.agent/handoffs/CURRENT               the open session's id
.agent/handoffs/sx-….json             session capsules
.agent/handoffs/wk-… sc-… rv-… ad-….json   packets
.agent/handoffs/rs-….json             results
```

Gitignored, alongside `.agent/scratch/` (D-036). All of it is disposable coordination
state: it describes a workspace at a moment, it is worthless once that tree has moved on,
and it holds the parent's private notes. Hundreds of them in `.akr/records/` would drown
the ledger the way D-032 says a `commit` kind would.

None of it is knowledge. It is invisible to `akr search`, `akr context` and the compiler.
What survives is whatever the work made durable — a record, with evidence, through the
ordinary write pipeline.

## 12. The commands

```text
akr handoff capsule [--refresh] [--boundary <text>]...
akr handoff session begin --request <text> | --request-file <path> [...]
akr handoff session show | end
akr handoff worker | scout | reviewer | advisor --role <text> --task <text> [...]
akr handoff list | open <id> [--reveal] | expand <id> <section>
akr handoff reveal <id> | verify <id> | discard <id>
akr handoff result <packet> [--finding ...] [--read ...] [--not-examined ...]
akr handoff results [<packet>] | coverage
```

The mode is the subcommand rather than a flag, because the mode is the whole decision a
parent makes when it delegates, and burying it in a flag makes the wrong one easy to leave
at its default. `create --mode <m>` is accepted too, so one MCP tool maps onto all four
without the catalogue growing a member per mode.

`--request-file` and `--task-file` exist because a verbatim request is often a paragraph
with newlines and quotes in it, and a shell mangles those.

Sections for `expand`: `task`, `workspace`, `project`, `session`, `scope`, `assignment`,
`inherited` — what earlier packets read, ran and changed, with their findings only where
the mode discloses them — and `notes`, which is `reveal`, so expanding the notes cannot
become a second door into the withheld layer that leaves no record.

## 13. The MCP surface

| Tool | Writes | |
| --- | --- | --- |
| `knowledge.handoff_capsule` | no | the project capsule |
| `knowledge.handoff_session_begin` | yes | open a session |
| `knowledge.handoff_session_show` | no | the open session and its packets |
| `knowledge.handoff_session_end` | yes | close it |
| `knowledge.handoff_create` | yes | cut a packet, with `mode` |
| `knowledge.handoff_list` | no | what the workspace holds |
| `knowledge.handoff_open` | no | a packet, with inheritance resolved |
| `knowledge.handoff_expand` | no | one section |
| `knowledge.handoff_reveal` | yes | the withheld layer, and the record of it |
| `knowledge.handoff_verify` | no | drift alone |
| `knowledge.handoff_result` | yes | file what you found |
| `knowledge.handoff_results` | no | read what came back |
| `knowledge.handoff_coverage` | no | the roll-up |
| `knowledge.handoff_discard` | yes | delete one object |

Fourteen tools is a large addition to a catalogue described as closed. It earns the size
for one reason: a subsystem a child agent must use *instead of* `knowledge.start` has to be
fully reachable over MCP, or children fall back to re-orienting — which is the whole thing
being prevented.

`writes` here means "changes the workspace", which for these is `.agent/handoffs/` rather
than `.akr/records/`. `readOnlyHint` and `--surface read` both read that field, and both
would be lying if a tool that created a packet reported itself read-only.

`knowledge.handoff_open` carries the largest budget on the surface. It is the one read
whose whole purpose is that the second agent arrives knowing the workspace, and because a
packet is addressable the overflow path names a real continuation —
`knowledge.handoff_expand`, one section at a time — rather than truncating everything.

Its two halves are deliberately asymmetric. The text is the complete briefing; the
structured content is an index — ids, drift, scope, what is available and what is
withheld, and the section names `expand` accepts. An MCP response carries both, and the
budget counts both, so a structured copy of the request, the project capsule and the
session head would put every inherited fact in front of the model twice. The same shape
comes back from `akr --format json handoff open`; a section's structured form is one
`expand` away.

## 14. What AKR does not own

AKR owns state, context, packet construction and retrieval, provenance and snapshot
verification. It does not launch agents, choose between models, price them, or know what a
particular harness's spawn mechanism looks like this month.

```text
              AKR
               │
        packet sc-…
               │
     ┌─────────┴─────────┐
     │                   │
 one harness        another harness
```

A packet is provider-neutral by construction. Coupling the data model to today's agent
harness would date it within a release.

## 15. The preparation boundary

Before delegating, a parent should:

> Make the task executable without making the judgement the child is being sent to make.

Concretely: open a session, read the session head, resolve project state, verify the build,
compile dependencies, run the existing tests and benchmarks, collect the artefacts that
already exist, capture git state, identify recent related work and acceptance criteria.

It may investigate further and record what it found in `worker_notes`. What it must not do
is wait until it believes it understands the problem before delegating — that moves the
bottleneck rather than removing it.

## 16. The guidance is in one place

The instruction agents read is four lines, in `scripts/agent-section.md`, installed into
whatever instruction files a workspace has:

```markdown
### Handoff

`akr handoff worker` cuts a packet for a continuing subagent; `scout` for an
independent investigation, `advisor` for a second opinion, `reviewer` for an
adversarial check. AKR builds the packet; you launch the agent and hand it
`akr handoff open <id>`.

Because inherited facts are cheap and inherited conclusions are not.

Open a session first (`akr handoff session begin --request "<the user's words,
verbatim>"`); each packet inherits it, so no child re-derives the project.
`akr handoff --help` has the rest.
```

Everything else is here and in `akr handoff --help`. Instructional prose in three files is
prose that goes out of date in two of them, and every line of it is read at the start of
every session in every project the brief is installed into.
