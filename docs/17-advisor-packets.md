# 17 — Advisor packets

How a task is handed to a second model without handing over the judgement that model is
being brought in to make: the two layers, the blind read, the workspace fingerprint, and
the CLI and MCP surfaces.

Normative for the packet shape, the layer separation, the reveal contract and the drift
statuses. Advisory for rendering and for how a harness dispatches to a particular model.
Decided in D-040.

---

## 1. The problem

An agent doing the work — call it the *worker* — reaches a point where a second, differently
capable model would help. The obvious handoff is a summary: "I looked at the code, I think
the problem is in `chroma.rs`, here is what I found."

That handoff destroys the reason for making it.

If the advisor's value is finding what the worker did not notice, then a summary of what
the worker noticed is precisely the wrong input. It turns the worker into a perceptual
bottleneck: the advisor inherits the worker's framing, searches where the worker searched,
and confirms or refines a conclusion instead of forming one. The worker's blind spot
becomes the advisor's boundary.

The opposite extreme is no handoff at all. The advisor then spends its first — and often
most expensive — thousands of tokens on questions that have deterministic answers: what is
AKR, what work is current, what does `base_rev` mean, is there a `benches/` directory, what
is the build command, which document is authoritative, and a cold dependency compile.

## 2. The rule

> **Compress state, not search.**

Everything administrative may be prepared, compressed and handed over, because compressing
it loses nothing an advisor would have wanted. Everything interpretive may be recorded but
not substituted for the problem, because the problem is what the advisor was hired to see
for itself.

An advisor packet is that rule made into an object.

## 3. The two layers

**Layer A — administrative fact.**

| | |
| --- | --- |
| The user's request | verbatim, never a restatement |
| `HEAD`, its subject, dirty paths | with content digests |
| The ledger revision | the source-graph hash |
| The session head | `AKR SESSION HEAD`, embedded whole |
| Declared namespaces | |
| Governing context | the goal, and records that constrain the work |
| The search envelope | where the advisor may look, independently |
| Commands | build, test, benchmark, with what they produced |
| Baselines | measurements already established |
| Constraints | what the answer must respect |
| Evidence and artefacts | what already exists |

**Layer B — worker interpretation.**

| | |
| --- | --- |
| Hypotheses | where the worker suspects the answer lies |
| Examined | what the worker read |
| Not examined | what it did not — the half of coverage usually lost |
| Approaches | what the worker would propose |
| Searches | queries already run, including the ones that found nothing |

The layers are separate fields, not a convention. `handoff open` renders Layer A and
reports only that Layer B exists; nothing short of `handoff reveal` produces it.

## 4. The blind read

```text
handoff create   worker prepares the packet; its notes go in Layer B
handoff open     advisor reads Layer A and reviews independently
handoff reveal   advisor sees Layer B, and the packet records that it did
```

Recording the reveal is the point of separating the two acts. Once you know an advisor
read blind and then saw the worker's notes, "what did either side miss?" is a question
with an answer, and repeated across tasks it produces something better than handoff
machinery: evidence about where each model actually sees.

`akr handoff open --reveal` exists for the case where the advisor wants the notes at once.
It takes the same recorded path, so the packet is never unable to say whether the review
that followed was independent. The MCP surface has no such shorthand: `knowledge.handoff_open`
is declared read-only, and a flag that quietly wrote the reveal marker would make that
declaration false.

## 5. The search envelope

```text
search_envelope: ["**"]
```

The default is the whole project, and the default is load-bearing. A packet whose envelope
defaulted to the paths the worker touched would hand the advisor the worker's blind spot
as a boundary — the one thing the packet exists to prevent. Narrow it only when the user
narrowed the task.

The envelope is a permission, not an instruction. Layer A tells the advisor how the kitchen
is organised; it never says which drawer the answer is in.

## 6. The verbatim task

`task` is the user's request as the user wrote it.

A worker that has decided the work is about guided chroma denoising must not be able to
replace *"review this project and optimise for performance, both memory and CPU"* with
*"optimise guided chroma denoising"*. That substitution is the information loss the whole
design is against, and it is invisible afterwards: the advisor cannot tell a narrowed task
from a narrow one.

Interpretation has two legitimate homes. `question` is what the advisor is specifically
asked, when that is narrower than the task and the *user* narrowed it. `worker_notes` is
everything the worker thinks. Neither is `task`.

## 7. The workspace fingerprint

A packet describes a tree. Workers keep working. So a packet records:

```text
head            8c6370b5...            (full 40-hex)
head_subject    perf(ledger): cut git spawns per call
ledger_revision sha256:0b6aa...
dirty           <path> <sha256 of its bytes>   for every path git reports changed
```

Not every source byte: `HEAD` plus the source-graph hash plus a digest per dirty path is
enough to detect drift, costs one status call and a read of the files git already named,
and leaves the repository itself as the thing the advisor reads.

`handoff open` and `handoff verify` recompute it and report one of two statuses:

| Status | Meaning |
| --- | --- |
| `exact` | the workspace is as the packet describes it |
| `drifted` | `HEAD` moved, the ledger moved, or named paths changed |

A drifted packet is still usable — often the changes are irrelevant to the review — but the
advisor is told, and told *what* moved. Silently describing one tree while a second model
examines another is the failure this exists to make impossible.

Drift never changes the exit status. It is a fact about the working tree, not a
contradiction in the ledger, exactly as staleness is under D-024.

The packet store is excluded from the fingerprint. Writing a packet must not make that
packet drift against itself.

## 8. Where packets live

```text
.agent/handoffs/ap-7f29d3a10b44.json
```

Gitignored, alongside `.agent/scratch/` (D-036). A packet is disposable coordination
state: it describes a workspace at a moment, it is worthless once that tree has moved on,
and committing one would also commit the worker's private notes. Hundreds of them in
`.akr/records/` would drown the ledger the way D-032 says a `commit` kind would.

Packets are invisible to `akr search`, `akr context` and the compiler. Nothing about a
packet is knowledge. What survives one is whatever the advisor's review made durable — a
record, with evidence, written through the ordinary write pipeline.

The id is `ap-` plus twelve hex characters of a digest over the task, the ledger revision,
`HEAD`, the date and an ordinal, so a packet is addressable and two prepared the same day
for the same task do not collide.

## 9. The commands

```text
akr handoff create --task <text> | --task-file <path>
                   [--question <text>] [--by <name>] [--goal <ref>]
                   [--governing <ref>]... [--envelope <glob>]...
                   [--command <cmd>[=<result>]]... [--baseline <text>]...
                   [--constraint <text>]... [--evidence <ref>]...
                   [--artifact <path>]... [--budget <tokens>]
                   [--note-hypothesis <text>]... [--note-examined <path>]...
                   [--note-unexamined <path>]... [--note-approach <text>]...
                   [--note-search <text>]...
akr handoff list
akr handoff open <id> [--reveal]
akr handoff expand <id> <section>
akr handoff reveal <id>
akr handoff verify <id>
akr handoff discard <id>
```

`--task-file` exists because a verbatim request is often a paragraph with newlines and
quotes in it, and a shell mangles those. It is the only way to keep "verbatim" literally
true for a long request.

Every `--note-*` flag lands in Layer B, and nothing else can.

Sections for `expand`: `task`, `workspace`, `project`, `governing`, `envelope`,
`execution`, and `notes` — which is `reveal`, so that expanding the notes cannot become a
second door into Layer B that leaves no record.

## 10. The MCP surface

| Tool | Writes | |
| --- | --- | --- |
| `knowledge.handoff_create` | yes | prepare a packet |
| `knowledge.handoff_list` | no | what this workspace holds |
| `knowledge.handoff_open` | no | Layer A, plus drift |
| `knowledge.handoff_expand` | no | one section |
| `knowledge.handoff_reveal` | yes | Layer B, and the record of it |
| `knowledge.handoff_verify` | no | drift alone |
| `knowledge.handoff_discard` | yes | delete a packet |

`writes` here means "changes the workspace", which for these tools is `.agent/handoffs/`
rather than `.akr/records/`. `readOnlyHint` and `--surface read` both read that field, and
both would be lying if a tool that created a packet reported itself read-only.

`knowledge.handoff_open` carries a deliberately large budget (2,000 target, 3,500 hard):
it is the one read on this surface whose whole purpose is that the second model arrives
knowing the workspace. Because a packet is addressable, the overflow path has somewhere
real to send the caller — `knowledge.handoff_expand`, one section at a time, rather than a
truncated everything.

## 11. What AKR does not own

AKR owns state, context, packet construction and retrieval, provenance and snapshot
verification. It does not launch models, choose between them, price them, or know what a
particular harness's spawn mechanism looks like this month.

```text
              AKR
               │
       advisor packet ap-X
               │
     ┌─────────┴─────────┐
     │                   │
 one harness        another harness
```

A packet is provider-neutral by construction. Coupling the data model to today's agent
harness would date it within a release.

## 12. The preparation boundary

The worker's job before calling an advisor is:

> Make the task executable without making the judgement the advisor is being hired to make.

Concretely: initialise AKR and read the session head, resolve project state, discover
repository structure, verify the build, compile dependencies, find and run the existing
tests and benchmarks, collect profiling data that already exists, capture git state,
identify recent related work, collect acceptance criteria.

It may investigate beyond that, and record what it found. What it must not do is wait until
it believes it understands the problem before calling the advisor — that just moves the
bottleneck rather than removing it.
