# 13 — Implementation Roadmap

The order in which AKR gets built, what each phase must deliver before the next begins,
how the code is laid out, how each phase is tested, and the acceptance test that decides
whether the design worked.

Normative for phase boundaries and exit criteria. Advisory for module names and internal
structure.

---

## 1. Principles

**Semantics before syntax.** The model and its invariants are built and tested before a
line of the parser exists. A parser written first tends to define the model by accident,
and the model is the part that has to be right.

**Every phase ends in something runnable.** No phase delivers only a library that nothing
calls. Each ends with a command, a test suite, or a rendered artefact that a person can
look at.

**Fixtures before features.** `fixtures/` — Writer A's conformance corpus — is the
executable form of the specification. A rule that has no fixture is not implemented, and
a fixture that does not run is not a rule.

**Determinism is tested, not assumed.** Every phase that produces output adds a test that
runs the same input twice and compares bytes, and a test that runs it on shuffled input
order and compares bytes.

**Dogfood before publishing.** Nothing about the format is stable until AKR has managed
its own development and two more real projects (§8).

## 2. Phase overview

| Phase | Delivers | Implements |
| --- | --- | --- |
| **P1** | Semantic model and invariant tests | `02-data-model.md`, `05-validation-rules.md` |
| **P2** | Lexer, parser, AST, formatter, span diagnostics | `03-syntax.md`, `spec/grammar/akr.ebnf` |
| **P3** | Resolver and validator | `04-references-and-versioning.md`, `05-validation-rules.md`, `06-compiler-pipeline.md` A–D |
| **P4** | Fixed roadmap renderer | `11-projections.md`, `06-compiler-pipeline.md` F |
| **P5** | Git freshness and acceptance evidence | `10-freshness-and-git.md` |
| **P6** | CLI, then MCP | `07-cli.md`, `08-mcp.md`, `09-context-assembly.md` |
| **P7** | Full-text search, then embeddings | `09-context-assembly.md` §10, `spec/schema/index.sql` |
| **P8** | Migration tooling | `12-migration.md` |
| **P9** | Human review interface | `10-freshness-and-git.md`, `11-projections.md` |
| **P10** | Immutable source library, chunk index, citations | `15-external-sources.md`, D-031 |
| **P11** | The AKR ↔ git change protocol | `16-change-protocol.md`, D-032 |
| **P12** | Advisor packets: the worker-to-advisor handoff | `17-advisor-packets.md`, D-040 |

The order is a dependency order, not a priority order. P4 sits where it does because a
rendered roadmap is the first artefact a person can *judge*, and judging it early is
worth more than the two weeks it costs.

---

## 3. Phases

### P1 — Semantic model and invariant tests

**Goal.** Encode the twelve kinds, four classes, four lifecycles, twelve relations, scope
terms and claim structure as Rust types, with the validation rules expressed against
those types and tested — before any text can be parsed.

**Deliverables.**

- `akr-core::model`: `Kind`, `Class`, `State`, `Relation`, `Record`, `Revision`, `Claim`,
  `Check`, `Disposition`, `ScopeTerm`, `Ref`.
- Lifecycle state machines as data, generated from or checked against
  `spec/tables/vocabulary.json`.
- Relation domain/range/cardinality/acyclicity tables, likewise.
- `akr-core::validate`: V-001..V-025 over an in-memory model, each rule a named function
  returning diagnostics.
- Model builders for tests that construct records without text.
- The D-010 scope-overlap function, with its conservative bias tested in both directions.

**Exit criteria.**

1. Every kind, state, relation and rule in `vocabulary.json` has a corresponding
   construct in code, checked by a test that reads the JSON.
2. Every V-rule has at least one passing and one failing unit test built from model
   builders.
3. Scope overlap has property tests asserting reflexivity, symmetry, and that `all`
   overlaps everything.
4. No dependency on any text format. The crate compiles with no parser.

**Testing.** Unit tests per rule; property tests for overlap and for lifecycle
reachability: every terminal state reachable from some initial state, and every
initial state able to reach some terminal state (no live state you can never
leave). Not every terminal is reachable from every initial — `rejected` is
reachable only from `proposed`, because a record already in force is withdrawn
or superseded, never "considered and declined"; that asymmetry is pinned by its
own test.

---

### P2 — Lexer, parser, AST, formatter

**Goal.** Text in, model out, and back again, with spans good enough to render the
diagnostic form of `spec/diagnostics/README.md` §5.

**Deliverables.**

- `akr-core::syntax`: lexer, recursive-descent parser, CST with byte spans, trivia
  capture with the D-006 attachment rules.
- The canonical formatter of D-012: slot ordering, indentation, array layout, prose
  dedent and re-indent, comment re-emission.
- Diagnostic rendering: span to line/column, the caret form, notes, help.
- `AKR-P` and `AKR-F` codes wired to `spec/diagnostics/codes-lang.md`.

**Exit criteria.**

1. `spec/exemplar.akr` round-trips byte-identically through parse and format.
2. `fmt(fmt(x)) == fmt(x)` holds for every fixture in `fixtures/parse/ok/` — property
   tested, not spot-checked.
3. Every fixture in `fixtures/parse/err/` produces exactly the codes named in its
   `.expected` file, with the spans named.
4. Every `fixtures/format/` pair formats from input to expected output exactly.
5. Comments survive formatting with their attachment, tested on a fixture containing
   every attachment position of D-006.

**Testing.** Snapshot tests over `fixtures/`; a round-trip property test; a fuzz target on
the lexer asserting no panic and no infinite loop.

---

### P3 — Resolver and validator

**Goal.** Stages C and D: the reference graph, head resolution, the graph checks,
acceptance, sealing, and the lock file. After this phase `akr check` is real.

**Deliverables.**

- `akr-core::resolve`: linking, head resolution, supersession chains, the resolved model
  of `06-compiler-pipeline.md` §6.
- `akr-core::graph`: cycle detection with deterministic cycle reporting, reachability,
  the reverse-propagation walk (used in P5).
- `akr.lock` read and write, in AKR syntax, per `spec/schema/akr-lock.md`.
- Content hashing and source-graph hashing per `06-compiler-pipeline.md` §9.
- `AKR-L` and `AKR-R` codes.

**Exit criteria.**

1. Every fixture in `fixtures/validate/ok/` resolves clean; every fixture in
   `fixtures/validate/err/` produces exactly its expected codes.
2. The worked example `examples/save-your-skin/` passes `akr check` with exit 0
   (MANIFEST §9).
3. Cycle reports are deterministic: a graph with several cycles reports the same one on
   every run and after input shuffling.
4. A sealed revision whose text is edited produces `AKR-R051` naming the key, revision and
   expected hash.
5. A hash is stable across a reformat: formatting a file does not change any content hash.

**Testing.** Fixture-driven; a shuffled-input determinism test; a golden `akr.lock` for
the worked example.

---

### P4 — Fixed roadmap renderer

**Goal.** One view, rendered, snapshot-tested, and looked at by a human. The point of
doing this early is that a rendered roadmap is the first artefact whose *quality* can be
judged rather than merely verified.

**Deliverables.**

- `akr-core::render`: `ROADMAP.md` exactly as specified in `11-projections.md` §5,
  banner included.
- The banner writer, with the source-graph hash and commit threaded through.
- `--views-current` comparison logic and `AKR-E011`–`AKR-E014`.

**Exit criteria.**

1. `examples/save-your-skin/docs/generated/ROADMAP.md` is reproduced byte-identically from
   the example's `.akr` sources.
2. A hand edit to the committed file makes `akr check --views-current` fail with
   `AKR-E011` naming the file and the first differing line.
3. The renderer is deterministic under input shuffling.
4. A human reads the output and agrees it is the roadmap.

Exit criterion 4 is not a joke and is not automatable. If the generated roadmap is worse
than the Markdown roadmap it replaces, the data model is wrong, and finding that out at
phase 4 is much cheaper than finding it out at phase 9.

**Testing.** Snapshot tests against the committed example views.

---

### P5 — Git freshness and acceptance evidence

**Goal.** The parts that need a repository: staleness derivation, propagation, the
descendant-commit rule for acceptance, and `akr impact`.

**Deliverables.**

- `akr-core::git`: commit existence, ancestry, changed paths for a range, HEAD, dirty
  detection. One memoised query per distinct question (`06-compiler-pipeline.md` §12).
- Watch-glob matching over the D-008 subset, with literal-prefix precomputation.
- The staleness algorithm of `10-freshness-and-git.md` §3 and the propagation walk of §4.
- The D-016 acceptance check, including the "evidence too old" verdict.
- `akr impact` in both modes; `akr review-queue` ordering.
- `AKR-G` codes.

**Exit criteria.**

1. Against a synthetic repository reproducing MANIFEST §4, the derivation produces
   **exactly 2 stale and 4 at-risk** records, with the causes and depths of MANIFEST §7.
2. `akr impact --git-diff C4..C5` reports no newly stale records (MANIFEST §9).
3. Acceptance verdicts match MANIFEST §6, including M3's one satisfied and one
   unsatisfied check.
4. Staleness never changes an exit status; `--review-clean` does, with `AKR-G041`.
5. `akr build` writes no `.akr` file — asserted by a test that hashes every source file
   before and after a build.

**Testing.** A test fixture repository built by a script from the MANIFEST history table,
so the example and the tests cannot drift apart.

---

### P6 — CLI, then MCP

**Goal.** The whole surface of `07-cli.md`, then the nine tools of `08-mcp.md` over the
same core. In that order, because the CLI is what the MCP server wraps.

**Deliverables.**

- `akr-cli`: every command of `07-cli.md` §6, global flags, exit codes 0/1/2/3, the
  `--format json` envelope.
- The write pipeline of `07-cli.md` §4 with its atomicity guarantee.
- Context assembly: the eleven steps of `09-context-assembly.md` §4, exclusions,
  budgeting.
- `akr-mcp`: the nine tools, the error-class mapping, `base_rev` conflict detection.
- The `AGENTS.md` template of `08-mcp.md` §8, written by `akr init`.
- `AKR-C` and `AKR-X` codes.

**Exit criteria.**

1. Every transcript under `examples/save-your-skin/transcripts/` is reproduced exactly.
2. A context bundle is byte-identical across two runs and across machines.
3. A failed write leaves every source file byte-identical — tested by hashing before and
   after every failing write path.
4. `knowledge.context` and `akr context` produce the same bundle from the same request.
5. Exit codes are correct for one representative case each of 0, 1, 2 and 3.

**Testing.** CLI integration tests over the worked example; a differential test asserting
CLI and MCP agreement on every read tool.

---

### P7 — Search: FTS, then embeddings

**Goal.** Make the ledger navigable without weakening the rule that search only ranks.

**Deliverables.**

- Stage E in full: `spec/schema/index.sql`, populate, invalidate, `AKR-I` codes.
- `records_fts` and `akr search` with BM25 ranking and key tiebreak.
- Later, and optionally: a vector ranker behind the same interface, with the four
  contract points of `09-context-assembly.md` §10.

**Exit criteria.**

1. Search results are stable: the same query returns the same order twice.
2. Deleting `.akr/cache/` and rebuilding produces identical query results.
3. A test asserts that **no context bundle changes when the ranker is disabled** — the
   direct executable form of "search ranks, never authorises".
4. FTS5 absent degrades to `AKR-I022` on `akr search` and affects nothing else.

**Testing.** Query snapshot tests; the ranker-disabled bundle-equality test, which is the
important one.

---

### P8 — Migration tooling

**Goal.** `akr import` and the workflow of `12-migration.md`.

**Deliverables.**

- Markdown and plain-text readers; heading and paragraph segmentation.
- Durable-claim proposal, with the model-assisted path clearly outside the write pipeline
  (`12-migration.md` §6).
- `source { kind legacy … }` block generation with verbatim excerpts.
- Tracking-record creation and check generation.
- `--lenient`, `--dry-run`, and the `AKR-M` codes.

**Exit criteria.**

1. Importing a legacy document with no model available still works, proposing one record
   per heading — the deterministic floor.
2. Every imported record lands `proposed` with a `source { kind legacy }` block, asserted
   by a test (`AKR-M021`, `AKR-M042`).
3. `--lenient` changes exit status only; the warning list is identical with and without it.
4. `akr complete` on a tracking record with an unsatisfied check fails with `AKR-R022`.
5. Excerpts are byte-identical substrings of the source document — asserted, because a
   paraphrased excerpt defeats the audit.

**Testing.** A legacy-document corpus with expected proposals; a substring assertion over
every generated excerpt.

---

### P9 — Human review interface

**Goal.** Make the review queue something a person will actually work through.

**Deliverables.**

- A static HTML dashboard emitted by `akr build --dashboard`: the review queue, the
  roadmap, acceptance status, open questions. Static files, no server, no JavaScript
  framework, opens from the filesystem.
- A TUI (`akr review`) that walks the queue one record at a time, showing the record, the
  cause, the propagation path, and the four actions — re-observe, supersede, withdraw,
  leave — each of which shells out to the corresponding write command.
- Later: graph visualisation of the relation graph, scoped to a subtree, as a
  `graphviz`-style export rather than an embedded renderer.

**Exit criteria.**

1. The dashboard is deterministic and diffable, like every other generated artefact.
2. Every TUI action is exactly one existing write command, with no bypass of the write
   pipeline.
3. A reviewer clears a five-item queue without reading any documentation.

Graph visualisation is deliberately last. It is the most demonstrable feature and the
least useful one: a picture of the relation graph is impressive in a screenshot and
almost never what someone needs to answer a question. `akr why-current` answers more
questions per minute.

---

### P10 — Immutable source library, chunk index, citations

**Goal.** Hold outside advice as exact bytes, make it findable, and let a record cite a
passage of it — without turning somebody else's report into project state (D-031).

**Deliverables.**

- `sources/catalog.json`, `akr source add|list|get|verify|supersede`, `AKR-S021`.
- A deterministic Markdown chunker: heading paths, byte and line ranges, packed to
  450–700 estimated tokens, code and tables never split, fences recognised before
  headings.
- `.akr/cache/sources.sqlite` — `source_documents`, `source_chunks`, `source_chunks_fts` —
  on its own generation, so a record write does not rechunk and a registration does not
  re-resolve.
- `akr source search` with escaped-by-default queries, `--literal` and `--fts`;
  `akr source get --chunk --neighbors`.
- `knowledge.source_search` and `knowledge.source_get`.
- `source { document … start_byte … }` citations, checked by `akr check` (`AKR-S022`).

**Exit criteria.**

1. Registering a source creates no records, and an unreviewed report never reaches
   `ACTIVE-WORK.md`.
2. Editing a registered file fails `akr source verify` with a non-zero status.
3. Every chunk's byte range reproduces the registered bytes exactly.
4. Re-wrapping a paragraph does not change its search text.
5. A record write leaves every chunk id untouched; a registration leaves the record tables
   untouched.
6. Every source result carries the words `non-authoritative`.

**Testing.** `crates/akr-cli/tests/source_library.rs` for the workflow;
`source::chunk` and `store::sources` unit tests for the rules.

---

### P11 — The AKR ↔ git change protocol

**Goal.** Make the synchronised path easier than the unsynchronised one, without giving
git history a record kind (D-032).

**Deliverables.**

- `akr diff --staged`: a semantic delta computed by parsing two ledgers.
- `akr change begin|show|prepare|verify|abort`, stored per worktree under the git dir.
- `akr git message|commit|log|install-hooks`, and `akr git-hook`.
- The implementation digest, excluding `.akr/**` and `docs/generated/**`.
- The `AKR-*` commit trailers.

**Exit criteria.**

1. A whitespace-only ledger edit produces no semantic change.
2. A staged code change with neither a work reference nor an explicit exemption is refused.
3. A staged tree that moves after preparation invalidates it.
4. The same transaction and staged tree produce a byte-identical message.
5. A commit made through the bridge is findable from the record it advanced.
6. Installed hooks are wrappers; the checks live in the binary.

**Testing.** `crates/akr-cli/tests/change_protocol.rs`, whose assertions are mostly about
what the bridge *refuses* — which is the part worth having.

---

### P12 — Advisor packets

**Goal.** Make handing a task to a second model a first-class operation, without letting
the preparing agent make the judgement that model is being brought in to make (D-040).

**Deliverables.**

- `akr handoff create|list|open|expand|reveal|verify|discard`, stored in
  `.agent/handoffs/`, gitignored.
- The two-layer packet: administrative fact, compressible; worker interpretation,
  withheld until a recorded `reveal`.
- The workspace fingerprint — `HEAD`, the source-graph hash, a digest per dirty path —
  and the `exact` / `drifted` statuses derived from it.
- Seven `knowledge.handoff_*` MCP tools over the same commands, and a third runner
  (`run_coordination`) for a workspace change that is not a ledger write.
- The agent guidance that makes "call an advisor" mean "prepare a packet".

**Exit criteria.**

1. An ordinary `open` leaks no worker note, on either surface.
2. `reveal` is recorded, so whether a review was independent is knowable afterwards.
3. The search envelope defaults to the whole project, never to what the worker examined.
4. The task field survives the round trip byte-identically, newlines and quotes included.
5. A fresh packet verifies as `exact`; an edited tree verifies as `drifted` and names
   what moved, without changing the exit status.
6. `knowledge.handoff_open` and `akr handoff open` produce an identical result object.

**Testing.** `crates/akr-cli/tests/advisor_packet.rs`, whose assertions are mostly about
what a packet does *not* say — the blind read is only a property if both surfaces keep it.

---

## 4. Cargo workspace

```
crates/
    akr-core/
        src/
            syntax/      lexer, parser, CST, formatter, spans        [P2]
            model/       kinds, states, relations, records           [P1]
            validate/    V-001..V-025                                [P1, P3]
            resolve/     heads, supersession, resolved model, lock   [P3]
            graph/       cycles, reachability, propagation           [P3, P5]
            git/         commits, ancestry, changed paths            [P5]
            store/       SQLite index, FTS                           [P7]
            render/      views, banner, dashboard                    [P4, P9]
            context/     bundle assembly                             [P6]
            diagnostics/ codes, severity, rendering                  [P2]
    akr-cli/             the `akr` binary                            [P6]
    akr-mcp/             MCP server over akr-core                    [P6]
```

**Three crates, and the third arrives late.** `akr-core` holds everything; `akr-cli` is
argument parsing, output formatting and process exit; `akr-mcp` is a transport adapter.
Neither binary crate contains logic that the other would want.

**Do not over-split.** The temptation — a crate per module, `akr-syntax`, `akr-model`,
`akr-resolve`, `akr-git` — should be resisted until there is a concrete reason, and
"cleanliness" is not one. The costs are real and immediate: every cross-module change
becomes a multi-crate version bump; every shared type needs a home crate and a public
API; compile times get worse rather than better once the dependency graph is deep;
and refactoring across a crate boundary is materially harder than within one.

The signals that would justify splitting, none of which exist yet: a second binary needs
`syntax` without `git`; an external consumer wants the parser alone; compile time for the
full workspace exceeds a minute and profiling blames one module; or a module acquires a
dependency the rest of the workspace should not carry. Until one of those is true,
`akr-core` stays one crate with well-separated modules — which delivers the same
separation with none of the cost.

**Dependencies, kept short deliberately.** A parser needs no parser generator here; the
grammar is small and a hand-written recursive-descent parser gives better diagnostics.
SHA-256, SQLite with FTS5, a CLI argument parser, a serialisation library for the JSON
envelope, and a glob matcher (or a hand-written one for the D-008 subset, which is small
enough to be worth owning). Git is invoked as a subprocess or through a library; either
is fine, and the subprocess route is one fewer build-time dependency.

**What was actually taken, as of P7.** One: `rusqlite`, with the `bundled` feature, for
stage E. Everything else on that list was written in house — SHA-256, the JSON writer and
reader, the argument parser, the glob matcher — and git is the subprocess. SQLite is the
one entry that could not go the same way, because the cost is not a SQL engine but an FTS5
tokeniser and a BM25 ranker beside it, and owning those buys nothing the design wants.

**Verification tooling is exempt, and lives outside the workspace.** The rule above binds
`crates/`. `tools/mcp-conformance/` is excluded from `[workspace] members`, keeps its own
lock file, and depends on several major versions of a real MCP client library at once —
the exact opposite of owning it outright, and correct here, because the thing it verifies
*is* compatibility with code this project does not own and cannot predict. A conformance
harness that reimplemented the client would only ever confirm its own reading of the
protocol, which is the failure it exists to catch. Nothing in `crates/` may depend on it,
no shipped binary links it, and `cargo test` never builds it.

`bundled` compiles SQLite from vendored C rather than linking the machine's, which costs a
C toolchain at build time and buys a build that behaves the same everywhere and an FTS5
that is present by construction. The `fts5` cargo feature is therefore about what stage E
*builds*, not about what SQLite *has*: without it the cache carries no full-text table and
`akr search` fails with `AKR-I022`, which is how P7's fourth exit criterion is reachable
by a test rather than only by an unlucky operator.

## 5. Testing strategy

| Kind | Where | What it protects |
| --- | --- | --- |
| Unit | Every module | Rule-by-rule behaviour |
| Fixture | `fixtures/` | The specification, executably |
| Snapshot | `examples/` | Rendered output, byte for byte |
| Property | `syntax`, `graph`, `validate` | Formatter idempotence, overlap symmetry, cycle-report stability |
| Determinism | Every output-producing phase | Two runs agree; shuffled input agrees |
| Differential | P6 | CLI and MCP agree |
| Fuzz | Lexer, parser | No panic, no hang |
| Integration | P6 onward | Whole commands over the worked example |

Two tests are worth naming individually because they encode design decisions rather than
behaviour:

- **The no-write test** (P5): hash every `.akr` file, run `akr build`, hash again, assert
  equality. This is D-003 made executable, and it is the test that would catch a
  well-intentioned "just mark it stale in the file" change.
- **The ranker-disabled test** (P7): assemble a bundle with ranking on and off, assert the
  *set* of records is identical. This is "search ranks, never authorises" made executable.

`tools/check-design.py` runs in CI from now on and keeps the design set internally
coherent: link resolution, diagnostic-code closure, vocabulary agreement,
MANIFEST↔corpus agreement, reference resolution over the example, grammar lint, fixture
coverage, and terminology.

## 6. The dogfood acceptance test

AKR is judged on whether it manages its own development. Ten steps, each checkable, run
against `examples/save-your-skin/` and then against the AKR repository itself.

1. **`akr check` on `examples/save-your-skin/` exits 0.** The worked example is a valid
   ledger, and every V-rule passes over it (MANIFEST §9).
2. **`akr build` reproduces all six committed views byte-identically**, and
   `akr check --views-current` exits 0 immediately afterwards.
3. **`akr build` twice in a row produces no diff** — not in the views, not in `akr.lock`,
   not anywhere. The second run reports every view unchanged.
4. **`akr review-queue` reports exactly 2 stale and 4 at-risk records**, with the causes
   and propagation paths of MANIFEST §7, and exits 0.
5. **`akr context --goal sys.milestone.m3-playable-day --paths "sim/src/project/**"`
   returns the bundle of MANIFEST §9** — M3, `sys.work.m3-plan/2`, the in-scope policies
   and constraints, the blocked work item with its blocking question, both M3 checks with
   the satisfied one marked, `sim.obs.projection-gaps` with a staleness warning, and the
   acknowledged contradiction — and excludes `sys.work.m3-plan/1`,
   `lege.decision.renderer-boundary/1` and `sys.policy.weekly-demo`.
6. **`akr complete sys.milestone.m3-playable-day` fails with `AKR-R022`**, naming
   `no-placeholder-assets` as the unsatisfied check. Completion succeeds only after
   evidence is added.
7. **`akr supersede sys.work.m3-plan` without dispositions fails with `AKR-R014`**,
   listing both unfinished children, and writes nothing.
8. **Editing a sealed revision by hand fails `akr check` with `AKR-R051`**, naming the
   key, the revision and the expected hash.
9. **Hand-editing a generated view fails `akr check --views-current` with `AKR-E011`**,
   naming the file and the first differing line.
10. **An agent completes a real task through the MCP surface alone** — reading context,
    proposing an observation, and handing back a `knowledge.validate` that is clean —
    without reading a `.akr` file directly and without touching `.akr/cache/`.

Steps 1–9 are automatable and belong in CI. Step 10 is the one that decides whether the
design was worth building: the whole system exists so that an agent can get correct,
current, sourced context and write back durable knowledge without a human transcribing
anything. If step 10 is awkward, the surface is wrong, and no amount of steps 1–9 makes
up for it.

## 7. What is deliberately not in the plan

- **A server, a daemon, or a hosted service.** Files plus git, and a subprocess when
  something wants to read them.
- **A web editor.** Records are edited with the write commands or with a text editor.
- **Multi-project federation.** No cross-project references in 0.1 (D-009).
- **Grammar migration tooling.** Before 1.0, breaking changes are handled by `akr fmt`
  upgrades shipped with the tool (D-021).
- **An RDF or JSON-LD export.** Straightforward, deferred until a consumer exists
  (`01-architecture.md` §8).
- **Performance work beyond the targets of `06-compiler-pipeline.md` §12.** Ten thousand
  records in under five seconds is the bar. Nothing is optimised past it without a
  measurement.

## 8. Standardisation posture

**Nothing is published as a standard before AKR has been dogfooded on two or three real
projects.**

That means: no RFC, no specification submission, no versioned "AKR 1.0 format
specification" for other implementations, and no compatibility promise beyond D-021 —
unknown minor version is a warning, unknown major version is an error, breaking changes
expected, handled by `akr fmt` upgrades.

The reasoning is stated once in D-021 and holds for the whole design set. A format
that standardises before it has met a second real project acquires its mistakes
permanently, because the cost of fixing them is then borne by everyone who adopted it.
The specific mistakes this design is most likely to have made, and which only real use
will reveal:

- The twelve kinds may be eleven or fourteen. `assessment` may not earn its place beside
  `observation`; `track` and `milestone` may want merging.
- The conservative scope-overlap test may produce enough false positives to be annoying,
  or may miss cases that matter in a large repository.
- Propagation along three relations may be too narrow or too wide.
- `topic` may go unused, in which case D-004(b) is dead weight.
- The `acceptance` block may want a richer method vocabulary, or a poorer one.

Each of those is a cheap change now and an expensive one after publication. The
sequence is: build it, use it on AKR itself, use it on two more projects, fix what
those reveal, and only then consider whether anyone else should be asked to implement
it.

---

Next: [`14-glossary.md`](14-glossary.md) for the terminology, or
[`../examples/save-your-skin/README.md`](../examples/save-your-skin/README.md) for the
project the acceptance test runs against.
