# 01 — Architecture

How AKR is put together: what kind of system it is, the three layers, the six pipeline
stages, the determinism contract, where humans and agents and git each sit, how a
repository is laid out, what the compiler does and does not vouch for, what prior art it
borrows notation from, and what it deliberately is not.

Normative for: layer trust rules, the determinism contract, repository layout, the trust
model, and the anti-goal list. Stage mechanics are normative in
[`06-compiler-pipeline.md`](06-compiler-pipeline.md); this document gives the shape.

---

## 1. Framing: a compiler, not a retrieval store

The most common way to misread AKR is as a knowledge base with better metadata — a RAG
store with types. It is not, and every design decision that looks strange makes sense
once the right frame is in place.

**AKR is a compiler and a build system for project knowledge.**

A compiler takes source text, checks it against rules, and refuses to produce output
when the rules are broken. A build system tracks what changed, rebuilds what depends on
it, and produces artefacts that are a function of the inputs. AKR does both, over
records instead of code:

| Compiler concept | AKR |
| --- | --- |
| Source file | `.akr` file under `.akr/records/` |
| Compilation unit | The whole ledger — records refer to each other freely |
| Symbol | Key, with revisions and claim anchors |
| Type error | A kind-schema violation (`AKR-T001`, `AKR-T011`) |
| Link error | An unresolvable reference (`AKR-L001`) |
| Semantic analysis | Resolve: heads, cycles, acceptance, sealing |
| Object file / cache | `.akr/cache/index.sqlite` — rebuildable, gitignored |
| Lock file | `akr.lock` — pins current-head resolutions and sealed hashes |
| Emitted artefact | `docs/generated/*.md` |
| `make check` | `akr check` |

The consequences of this frame are the parts that distinguish AKR from a knowledge base:

- **The build fails.** A retrieval store degrades gracefully when its contents are
  contradictory; a compiler stops. Two live revisions of one key is `AKR-R001`, not a
  ranking problem.
- **The answer is not ranked.** When an agent asks for context, the set of records
  returned is *computed*, not retrieved by similarity. Search exists, and it only ever
  reorders a set that was already authorised by the graph
  ([`09-context-assembly.md`](09-context-assembly.md) §1).
- **Output is reproducible.** The same sources at the same commit with the same tool
  version produce byte-identical views, index and lock, on any machine.
- **The cache is not the truth.** SQLite is an implementation detail of one stage, and
  nothing outside the tool may read it (D-019).

A useful test: if a proposed feature would make two runs of `akr build` disagree, or
would require the tool to *guess*, it does not belong in stages A–F.

## 2. Three layers

| | Scratch | Ledger | Views |
| --- | --- | --- | --- |
| **Path** | `.agent/scratch/` | `.akr/` | `docs/generated/` |
| **Format** | Anything | AKR source | Markdown |
| **Canonical** | No | **Yes** | No |
| **Committed** | No (gitignored) | Yes | Yes (D-025) |
| **Written by** | Agents and humans, freely | Humans and agents, through validated operations | `akr build` only |
| **Validated** | Never | Every write | Not applicable — derived |
| **Lifetime** | A session | The project | Until the next build |
| **If deleted** | Nothing is lost | Everything is lost | Rebuilt exactly |

**Scratch** exists so that the ledger does not have to absorb every thought. An agent
mid-task writes plans, dead ends, and half-formed reasoning to `.agent/scratch/`, and
nobody reviews any of it. The discipline the design asks for is not "write everything
down carefully" — it is "when something becomes durable, promote it to a record". The
anti-goal "not every agent output is durable" is enforced by making scratch cheap and
records deliberate.

**The ledger** is the only canonical layer. Every byte of it is human-readable AKR
source under version control, reviewable in a pull request, and greppable without a
tool. Writes go through `akr propose`, `akr revise`, `akr supersede`, and their
siblings, each of which parses, validates, canonically formats, and only then writes
([`07-cli.md`](07-cli.md) §4). Hand-editing a `.akr` file is entirely legitimate; `akr
fmt` and `akr check` are what make it safe.

**Views** are build outputs that happen to be committed. Committing generated files has
a real cost — merge conflicts every time two branches touch the ledger — and it buys one
thing: a reader on a web interface, or a tool that will never install AKR, sees current
knowledge. The banner and the `akr check --views-current` CI gate keep the cost bounded
(D-025, [`11-projections.md`](11-projections.md) §9).

The layers are ordered by trust, and information only ever flows upward: scratch may
become a record; a record may become a view; a view is never a source for anything.

## 3. The pipeline

```
                      .akr/records/**.akr        .akr/project.akr
                              |                        |
                              v                        v
   ┌──────────────────────────────────────────────────────────────────┐
   │  A  PARSE          text  ->  concrete syntax tree + spans        │  AKR-P, AKR-F
   ├──────────────────────────────────────────────────────────────────┤
   │  B  TYPE-CHECK     CST   ->  typed records (kind schema applied) │  AKR-T
   ├──────────────────────────────────────────────────────────────────┤
   │  C  LINK           typed ->  reference graph, anchors bound      │  AKR-L
   ├──────────────────────────────────────────────────────────────────┤
   │  D  RESOLVE        graph ->  resolved model                      │  AKR-R
   │       heads, supersession chains, cycles, exclusivity,           │
   │       acceptance, sealing, contradictions, STALENESS             │
   └──────────────────────────────────────────────────────────────────┘
              |                    |                       |
              |                    | git history           | akr.lock
              |                    v                       v
              |            (read-only: commits,     (read: sealed hashes)
              |             ancestry, paths)        (write: at build end)
              v
   ┌──────────────────────────────────────────────────────────────────┐
   │  E  INDEX          resolved model -> .akr/cache/index.sqlite     │  AKR-I
   ├──────────────────────────────────────────────────────────────────┤
   │  F  EMIT           resolved model -> docs/generated/*.md         │  AKR-E
   └──────────────────────────────────────────────────────────────────┘
              |                                            |
              v                                            v
        akr context / get / search                   committed views
        (AKR-X)                                      (checked by CI)
```

**A — Parse.** Bytes to a concrete syntax tree with byte-offset spans on every node.
Enforces the lexical rules of D-005 through D-008: identifier charset, string and prose
escaping, scalar literal forms, four-space indentation, LF endings. Comments are
captured as trivia with the attachment rules of D-006, because the formatter must
round-trip them. Emits `AKR-P` codes; `akr fmt --check` compares the re-emitted form and
emits `AKR-F`.

**B — Type-check.** CST to typed records. Applies the kind schema from
`spec/tables/vocabulary.json`: does this kind exist, does the state belong to the kind's
class lifecycle, are all required slots present, are all present slots known to the
kind, does every value match its declared type, are repeated slots rejected and
repeatable blocks allowed. Purely local — one record at a time, no cross-record
knowledge. Emits `AKR-T`.

**C — Link.** Typed records to a reference graph. Resolves every `@key`, `@key/2`,
`@key#anchor` and `@key/2#anchor` against the record set: does the key exist, is its
namespace declared, does the revision exist, does the anchor exist or is it explicitly
retired, do all revisions of the key live in one file, is the target's kind legal for
the relation's range. Head references are resolved here and each resolution is recorded
for `akr.lock`. Emits `AKR-L`.

**D — Resolve.** The reference graph to the resolved model — the data structure every
later stage reads. At most one live revision per key; supersession chains walked and
checked for cycles; the acyclic relation graphs checked; normative exclusivity by
`topic` and scope overlap; disposition completeness for superseding planning records;
acceptance satisfaction under the descendant-commit rule; sealed-revision hashes against
`akr.lock`; declared contradictions dispositioned. This is also where **staleness is
derived** — from `observed_at` commits, `watches` globs and `review_after` dates — and
propagated to dependents. Staleness is a build fact carried in the model, not a
diagnostic (D-024). Emits `AKR-R` and reads git.

**E — Index.** The resolved model to `.akr/cache/index.sqlite`
([`../spec/schema/index.sql`](../spec/schema/index.sql)). A cache, gitignored, never
authoritative, rebuilt whenever `schema_version` or `source_graph_hash` changes, safe to
delete at any moment. Nothing outside the tool reads it (D-019). Emits `AKR-I`.

**F — Emit.** The resolved model to the seven generated views, each opening with the
banner of D-025. `akr build` writes them; `akr check --views-current` renders them in
memory and diffs. Emits `AKR-E`.

Stages A–D are `akr check`. Stages A–F are `akr build`. Stage boundaries are hard: a
stage collects **all** of its diagnostics before stopping, and the pipeline **halts**
between stages if any error was collected, because a later stage's diagnostics would be
noise built on a broken model ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §2).

## 4. The determinism contract

> A build is a pure function of (source files, git commit, tool version).

Concretely, given the same `.akr` sources, the same `HEAD` commit and repository
history, and the same `akr` binary version, two runs on two machines produce
byte-identical `docs/generated/**`, byte-identical `akr.lock`, and semantically
identical `index.sqlite` (SQLite page layout is not promised; every query result is).

The contract has five parts, and each closes a specific hole:

1. **No wall-clock reads inside stages A–F, except one.** The single exception is the
   `review_after` comparison in stage D, which needs today's date. It is threaded in as
   an explicit parameter — settable with `--today` for testing — never read ambiently.
   Everything else that looks like time is a git commit.
2. **No environment, locale, or filesystem-order dependence.** Source files are
   discovered by a sorted walk. Every collection that reaches an output is sorted by an
   explicit key ([`06-compiler-pipeline.md`](06-compiler-pipeline.md) §8). No output
   depends on `LANG`, `TZ`, a hash seed, or a `HashMap` iteration order.
3. **No network.** Stages A–F make no network calls. A `source` block may carry a `url`;
   the compiler stores it and never fetches it.
4. **The lock pins ambiguity.** Every current-head reference resolved during a build is
   written to `akr.lock`, so a build is reproducible from (sources, lock) even if a
   later revision has since been added (D-014).
5. **No language model.** D-020, below.

### The LLM boundary (D-020)

```
   drafting     import      ranking      summarising
   proposals    suggestions of results   a bundle
      |            |           |            |
      v            v           v            v
  ┌───────────────────────────────────────────────┐
  │            MODELS MAY OPERATE HERE            │
  └───────────────────────────────────────────────┘
  ═══════════════════════ boundary ═══════════════════════
  ┌───────────────────────────────────────────────┐
  │        STAGES A - F : NO MODEL, EVER          │
  │  authority · head resolution · scope overlap  │
  │  cycles · staleness · acceptance · supersession│
  └───────────────────────────────────────────────┘
```

Above the line, a model is a productivity tool whose output is a *proposal* that a human
or an agent reviews and that the compiler then validates like any other input. Below the
line, every question has a defined algorithm in this specification set, and the
algorithm is the answer.

The boundary is not a stylistic preference. The value proposition is that the ledger's
mechanical claims can be trusted without re-checking them. A probabilistic step anywhere
in the build destroys that property for every claim downstream of it, and the failures
would be quiet — a slightly-wrong head resolution produces plausible output forever.

Three places where the boundary is easy to breach, and what is done instead:

| Tempting | Why it breaches | What AKR does |
| --- | --- | --- |
| Infer contradictions from prose | Judgement in the build | Contradictions are declared with `contradicts`; one mechanical check on `topic` + scope (D-023) |
| Rank context by relevance | The set returned would vary | Membership is computed from the graph; ranking only orders within a section (`09-context-assembly.md` §1) |
| Summarise a record to fit a budget | Output not reproducible | Prose is truncated at a deterministic boundary and marked; the summarising is done by the *consumer*, above the line |

## 5. Where humans, agents and git sit

```
   HUMAN                     AGENT                      GIT
     |                         |                         |
     | reviews PRs             | akr context ------------|--> reads history,
     | authors records         | akr get                 |    ancestry, paths
     | resolves review queue   | akr propose/revise      |
     | decides dispositions    | akr evidence add        |<-- commits records
     |                         | writes .agent/scratch/  |    and views
     v                         v                         v
   ┌─────────────────────────────────────────────────────────────┐
   │                        THE LEDGER                           │
   └─────────────────────────────────────────────────────────────┘
                               |
                          akr build (pure)
                               |
                   index + views + akr.lock
```

**Humans** are the authority. They review pull requests that change `.akr` files, decide
dispositions at replan time, and answer the review queue. They are also the reason the
notation is hand-writable: a format only agents can produce is a format nobody audits.

**Agents** are first-class writers, not second-class ones, but every write they make
goes through a validated operation that produces canonically formatted source. An agent
never edits the index, never edits a generated view, and never writes a `.akr` file with
a raw file-write tool when an `akr` command exists for the operation
([`08-mcp.md`](08-mcp.md) §7). The `AGENTS.md` protocol text in `08-mcp.md` §8 is the
minimal statement of that contract.

**Git** is the clock. AKR has no notion of time other than commits and two dates
(`created_at`, `review_after`, plus milestone `target`s). "Is this observation still
current?" is answered as "did any commit between its `observed_at` and `HEAD` touch a
path it watches?" — a question git answers exactly. Git is read-only to the build: the
compiler never writes a `.akr` file, never commits, and never stages
([`10-freshness-and-git.md`](10-freshness-and-git.md) §8).

## 6. Repository layouts

### The tool repository (this one, later)

```
akr/
    Cargo.toml                    workspace
    crates/
        akr-core/                 syntax, model, validate, resolve, graph,
                                  git, store, render
        akr-cli/                  the akr binary
        akr-mcp/                  MCP server (phase P6)
    spec/                         grammar, tables, schemas, diagnostics
    docs/                         this specification set
    examples/save-your-skin/      the worked example
    fixtures/                     conformance fixtures
    tools/check-design.py         design-set coherence checker
```

Crate layout and phasing are specified in
[`13-implementation-roadmap.md`](13-implementation-roadmap.md) §4.

### A consumer repository

```
my-project/
    .akr/
        project.akr               namespaces, defaults          committed
        akr.lock                  head resolutions, hashes      committed
        records/
            sys/policies.akr                                    committed
            sys/milestones.akr
            sim/observations.akr
            ...
        archive/
            sys/policies-archived.akr                           committed
        cache/
            index.sqlite          Stage E output                GITIGNORED
    docs/
        generated/
            ROADMAP.md            Stage F output                committed
            CURRENT-STATE.md
            ACTIVE-WORK.md
            REVIEW-REQUIRED.md
            OPEN-QUESTIONS.md
            DECISION-HISTORY.md
        legacy/                   pre-AKR Markdown, being migrated
    .agent/
        plans/                    agent plans and handoffs      committed
        scratch/                  disposable agent notes        GITIGNORED
    AGENTS.md                     the protocol statement        committed
    src/ ...                      the actual project
```

Three rules about this layout:

- **`.akr/records/` structure is convention, not semantics.** Identity is the key
  (D-018). The suggested convention — one file per namespace subtree and kind group — is
  for review ergonomics. The one enforced rule is that every revision of a key lives in
  one file (V-003), so a key's whole history is one diff.
- **`.akr/archive/` is a filesystem convention with one semantic consequence.** Records
  in it are still parsed, still resolve, and still satisfy references; they are excluded
  from ordinary context assembly and from every generated view except
  `DECISION-HISTORY.md`. What makes a record terminal is its *state*, not its path.
- **`.gitignore` must contain `.akr/cache/` and `.agent/scratch/`.** `akr init` writes
  both.
- **`.agent/scratch/` is the one disposable location nothing empties on its own.** The OS
  clears its temp directory and `target/` is deleted without ceremony; this is gitignored
  but lives inside the repository, so it survives every session. `akr scratch prune` is how
  it goes, and the same command drops live evidence citations of the entries it removes and
  of scratch paths that are already gone. `akr scratch keep` is how one entry stays, and
  `akr check` reports the total as a build fact (D-036).

## 7. Trust model

The compiler is precise about what it vouches for, because a tool that overclaims is
worse than no tool.

**Guaranteed by a passing `akr check`:**

| Property | Mechanism |
| --- | --- |
| Well-formedness | Stages A and B |
| Referential integrity | Stage C — every reference resolves, or the build fails |
| Single authority per key | V-012 — one live revision, never a newest-wins tiebreak |
| Topic exclusivity | V-013 — no two live normative records over one topic in overlapping scope |
| Acyclicity | V-014, V-015, V-016 across supersession, dependency, containment and ordering |
| Nothing dropped at replan | V-017 — every unfinished child of a superseded plan is dispositioned |
| Acceptance means something | V-020 — completion requires passing evidence observed after the last content change |
| Immutability of settled records | V-024 — sealed revisions hash-checked against `akr.lock` |
| Contradictions are not lost | V-023 — declared contradictions are resolved or explicitly acknowledged |
| Currency is visible | Stage D staleness derivation; `REVIEW-REQUIRED.md` and `akr review-queue` |

**Explicitly not guaranteed:**

- **Truth.** A `verified` observation is one somebody wrote down as observed. The
  compiler checks that it names a commit and that the commit exists; it cannot check
  that the observation was accurate.
- **Completeness.** Nothing detects knowledge that was never written down.
- **Semantic consistency.** Two records may contradict each other in prose forever
  without any `contradicts` edge. The one inferred check is `topic` plus scope overlap
  (D-023). Detecting the rest requires judgement, which lives above the LLM boundary.
- **Correct scope.** The overlap test of D-010 is deliberately conservative: it may
  report an overlap that does not exist in practice, and it must never miss one. A false
  positive costs an author one narrowing edit; a false negative would silently permit
  contradictory governance.
- **That anyone read it.** AKR makes knowledge available and current. It does not make
  anybody use it.

The honest one-line summary: **the compiler guarantees currency, sourcing and internal
consistency; it never guarantees truth.**

## 8. Prior art

**PROV-N is the notation precedent.** The W3C provenance notation solved the same
presentation problem — expressing a typed, related, provenance-bearing graph in text a
person can write and read — and its shape (a keyword, an identifier, a parenthesised
attribute set) is recognisably an ancestor of `record key/rev : kind { slots }`. AKR
borrows the *ergonomics*: identifiers rather than URIs in the authoring surface, a
closed keyword vocabulary, one construct per line, and prose as a first-class value
rather than an escaped string.

**Why not RDF, OWL, or a triple store as the authoring surface.** The semantics would
fit — records are nodes, relations are predicates, and much of D-010 and D-024 could be
expressed as rules over triples. Four reasons it is the wrong surface here:

1. **Diff legibility.** A pull request that changes a policy must read as a change to
   that policy. Triples scatter one conceptual edit across many lines and lose the block
   structure that makes a record reviewable.
2. **Hand-authorability.** Turtle and JSON-LD are writable by hand and pleasant for
   nobody. The design requires humans to write and review records routinely.
3. **Open-world assumption.** RDF semantics say an unstated fact is unknown. AKR's rules
   are closed-world and total: a missing `disposition` is an error, a missing head is an
   error, an unknown slot is an error. Bolting closed-world validation onto an
   open-world model is exactly the complexity SHACL exists to manage, and it would be
   the largest single piece of the specification.
4. **Ecosystem cost.** A triple store is an operational dependency. `.akr` files plus
   git are not.

**Other borrowings.** Architecture Decision Records supply the idea that a decision is a
durable artefact with context and consequences (AKR's `decision` kind carries exactly
those slots). Cargo and other lock-file designs supply `akr.lock`. Rust's diagnostic
rendering supplies the span-and-note form in `spec/diagnostics/README.md` §5. Make and
Bazel supply the "output is a function of inputs" contract.

**Export mapping is deferred, deliberately.** A mapping from the resolved model to RDF
(or to JSON-LD, or to a SPARQL-queryable dump) is straightforward and can be added
without touching the authoring surface, since the model is already typed and the
relations already have domains and ranges. It is deferred until a consumer exists, on
the same grounds as D-021 and the standardisation posture in
[`13-implementation-roadmap.md`](13-implementation-roadmap.md) §8: designing an
interchange format before anybody interchanges anything produces permanent mistakes.

## 9. Anti-goals

Each of these is a thing AKR could plausibly become and deliberately does not.

**No Markdown plus frontmatter.** Frontmatter types the *document* and leaves the
payload — where every claim lives — unstructured. The result is a schema that validates
the wrapper of a stale document.

**No generic wiki.** Wikis optimise for page-level authorship, hyperlinks, and recency.
The unit here is a record with a lifecycle, and "recently edited" is not a form of
authority.

**No RDF authoring surface.** §8. An export mapping later is fine; an authoring surface
is not.

**No newest-wins.** Every conflict has a named resolution: supersession for revisions,
disposition for children, `topic` and scope for governance, `acknowledged` for
contradictions. "The most recent one" is how the Markdown pile fails, restated as a
feature.

**No automatic deletion, ever.** Nothing in the tool removes knowledge. Records reach
terminal states; files move to `archive/`; references keep resolving forever. A ledger
that forgets is a ledger nobody can audit.

**No line-number or file-path identity.** `docs/plan.md:42` is broken by the next
reformat, and `docs/plan.md` is broken by the next rename. Identity is
`@key/rev#anchor`, which survives both.

**Not every agent output is durable.** `.agent/scratch/` exists so the ledger does not
have to absorb reasoning traces. Promotion to a record is a deliberate act.

**No published standard before dogfooding on two or three real projects.** The grammar
version is `0.1`, breaking changes are expected, and no forward compatibility is
promised (D-021). A format that standardises before it has met a second real project
acquires its mistakes permanently.

---

Next: [`02-data-model.md`](02-data-model.md) for what a record is, or
[`06-compiler-pipeline.md`](06-compiler-pipeline.md) for how the stages above actually
work.
