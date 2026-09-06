---
name: handoff
description: Prepare an AKR advisor packet and hand the task to a second model, or open a packet you were handed. Use whenever the user asks to call an advisor, get a second opinion, consult another model, or use the AKR handoff workflow — and when you are the advisor being handed a packet id.
---

# /handoff — hand a task to an advisor, or take one

The rule this exists for is **compress state, not search** (D-040,
`docs/17-advisor-packets.md`).

A second model is worth paying for when it finds what you did not. So a summary of what
*you* found is the worst possible handoff: it makes you a perceptual bottleneck, and the
advisor inherits your framing instead of forming one. But arriving with nothing is also
waste — an advisor's first and most expensive tokens should not go on "what is AKR", "what
is the build command", "does `benches/` exist".

An advisor packet separates the two. Administrative fact is compressed and handed over.
Your interpretation is recorded and withheld.

## If you are preparing the handoff

**1. Do the administrative preparation, and stop there.**

Session head, project state, repository map, verify the build, compile dependencies, find
and run the existing tests and benchmarks, collect profiling data and artefacts that
already exist, capture git state, identify recent related work and acceptance criteria.

You may investigate further and record what you find. **Do not** wait until you believe you
understand the problem before calling the advisor — that moves the bottleneck rather than
removing it.

**2. Create the packet.**

```
akr handoff create --task <verbatim> [--question <text>] [--by <your model name>]
                   [--goal <ref>] [--governing <ref>]...
                   [--command <cmd>[=<result>]]... [--baseline <text>]...
                   [--constraint <text>]... [--evidence <ref>]... [--artifact <path>]...
                   [--note-hypothesis <text>]... [--note-examined <path>]...
                   [--note-unexamined <path>]... [--note-approach <text>]...
                   [--note-search <text>]...
```

Or `knowledge.handoff_create` over MCP.

Three things matter more than the rest:

- **`--task` is the user's request verbatim.** Not your reading of it. If you have decided
  the work is about one subsystem, that conclusion is exactly what the advisor was brought
  in to form independently — it goes in `--question` (only if the *user* narrowed it) or a
  `--note-*` flag. Long or multi-line requests: write it to a file and use `--task-file`,
  because a shell mangles newlines and quotes.
- **Leave the envelope alone.** It defaults to `**`, the whole project. Narrowing it to
  what you examined hands the advisor your blind spot as a boundary.
- **Put everything you think in `--note-*`.** Hypotheses, what you examined, what you did
  **not** examine, approaches, searches you already ran. This is the layer the advisor
  cannot see until it asks, and recording what you did not examine is the half that usually
  gets lost.

**3. Hand over the packet id and nothing else.**

Not a summary, not "I think it's in X". Just the id, and that `akr handoff open <id>` reads
it.

## If you are the advisor

1. `akr handoff open <id>` — or `knowledge.handoff_open`. You get the task verbatim, the
   workspace and whether it has drifted, your independent search scope, the session head,
   governing records, commands, baselines and artefacts.
2. **Review independently.** Look wherever the envelope reaches. The packet tells you how
   the kitchen is organised; it does not tell you which drawer the answer is in.
3. `akr handoff reveal <id>` — only after you have formed a view. Then compare: what did
   either side miss? Where the two disagree, the code decides.
4. `akr handoff expand <id> <section>` reads one part without re-opening the whole packet:
   `task`, `workspace`, `project`, `governing`, `envelope`, `execution`.
5. `akr handoff verify <id>` if you have been working a while — `drifted` names what moved.

## Afterwards

Whatever the review made durable goes in the ledger through the ordinary write path:
`knowledge.propose`, evidence, completion. The packet itself is disposable coordination
state — `akr handoff discard <id>` when it is done.

`akr handoff list` shows what this workspace holds, and marks which packets have been
revealed.
