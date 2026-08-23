# 00 — Overview

The entry point to the AKR specification set. Read this after the repository
[`README.md`](../README.md) and before anything else. It states the problem, sketches
the solution, maps the rest of the documents, and walks one worked example end to end.

Everything here is a summary of something specified precisely elsewhere. Where this
document and a specification document disagree, the specification document wins; where a
specification document and [`DECISIONS.md`](DECISIONS.md) disagree, `DECISIONS.md` wins.

---

## 1. The problem: Markdown cannot be enforced

A software project worked by AI agents produces documents at a rate no human review
process absorbs. Within a few weeks the repository holds a `docs/` directory of plans,
notes, retrospectives, architecture sketches, and status updates, and nobody — human or
agent — can tell which of them are still true.

The failure is not that the documents are badly written. It is that Markdown has no
place to put the six facts that decide whether a statement should be acted on:

| Question | What Markdown offers | What is needed |
| --- | --- | --- |
| **Authority** — is this decided, proposed, or somebody's note? | A heading, maybe a bold word | A lifecycle state the tool can read |
| **Scope** — what does it govern, and where does it stop? | A file path and a hope | A declared, comparable scope |
| **Currency** — does it describe reality now? | A date at the top, usually of the first draft | A commit, and a rule for when it goes stale |
| **Evidence** — what supports it, observed when? | A prose sentence | A typed reference to an evidence record with a commit |
| **Supersession** — what replaced it, and what happened to the unfinished parts? | A file someone may have deleted | A revision chain with mandatory disposition |
| **Invalidation** — what code change would make it wrong? | Nothing | A watched path glob |

Because the answers are absent, agents behave in the three ways that make long-running
agent projects untrustworthy:

1. **Re-derivation.** An agent re-reads the prose pile every session and reconstructs a
   different mental model each time. Consistency across sessions is accidental.
2. **Stale trust.** A statement written six weeks and four hundred commits ago reads
   exactly like one written this morning, so it is acted on.
3. **Re-litigation.** A settled decision has no mechanical trace of being settled, so
   the next agent reopens it, and the project pays for the same argument twice.

The prose pile is also *lossy at replan time*. When a plan is replaced, the unfinished
items under the old plan do not raise their hands. They are simply not mentioned in the
new document, and the project discovers six weeks later that lighting was never done.

## 2. The solution shape: records, a compiler, and generated views

AKR replaces the pile with three things.

**Records.** The unit is a record, not a document. A record has a stable dotted key
(`sys.policy.tandem-work`), a kind drawn from a closed vocabulary of thirteen
(D-001, D-027), numbered revisions (`@key/2`), individually addressable claims
(`@key/2#lag-bound`), a lifecycle state from its class's state machine (D-002), a
declared scope, and typed relations that carry mechanical consequences. Records live in
`.akr` files, which are containers only: identity comes from the key, never from a
path (D-018).

**A compiler.** `akr build` runs six stages — parse, type-check, link, resolve, index,
emit — over the whole ledger and either produces output or produces diagnostics with
spans. The build is a pure function of (source files, git commit, tool version). No
language model participates in any stage (D-020). Everything the compiler asserts is
mechanical: which revision is the head, whether a graph has a cycle, whether an
acceptance check is satisfied by evidence observed after the last content change,
whether two live normative records govern the same topic in overlapping scope.

**Generated views.** `docs/generated/ROADMAP.md` and its five siblings are build
outputs, committed to the repository so that every reader and tool sees current
knowledge without installing anything, and never hand-edited — a rule CI enforces by
rebuilding the views and diffing (D-025).

The three layers have different trust and mutability rules, and the distinction is the
load-bearing one in the whole design:

| Layer | Contents | Canonical | Written by |
| --- | --- | --- | --- |
| Scratch | `.agent/scratch/` — disposable working notes | No | Agents, freely, with no ceremony — and cleaned up by them, since nothing else does (D-036) |
| Ledger | `.akr/**` — typed records | **Yes** | Humans and agents, through validated operations |
| Views | `docs/generated/**` | No | `akr build`, never by hand |

## 3. What the compiler does and does not guarantee

The compiler guarantees three things and refuses to guarantee a fourth.

- **Currency** — every empirical record states the commit it was observed at, and the
  build tells you which observations a watched path has moved out from under.
- **Sourcing** — every claim that rests on evidence says which evidence, and the
  evidence says when it was observed.
- **Consistency** — one live head per key, no cycles in the dependency graphs, no two
  live normative records governing the same topic in overlapping scope, no supersession
  that silently drops unfinished children, no acceptance check closed by evidence older
  than the thing it claims to verify.
- **Not truth.** Nothing in AKR decides whether a record is *correct*. Staleness is a
  question raised, never an answer given (D-003): a stale record keeps its state and its
  content, and a human or agent decides what to do about it.

## 4. Guided tour of the document set

Read in this order. The frozen spine is authoritative over everything else.

**Spine — frozen.**

| Document | What to look for |
| --- | --- |
| [`DECISIONS.md`](DECISIONS.md) | D-001..D-025. Every question the design had to settle, and why. Read at least D-001, D-003, D-016, D-017, D-020, D-024. |
| [`../spec/tables/vocabulary.json`](../spec/tables/vocabulary.json) | The machine-readable spine: kinds, classes, lifecycles, slots, relations, rules. |
| [`../spec/exemplar.akr`](../spec/exemplar.akr) | Every syntactic construct, once, canonically formatted. |
| [`../spec/diagnostics/README.md`](../spec/diagnostics/README.md) | How diagnostics are numbered and who owns which letters. |
| [`../examples/save-your-skin/MANIFEST.md`](../examples/save-your-skin/MANIFEST.md) | The worked example's frozen inventory and its synthetic git history. |

**The language.**

| Document | What to look for |
| --- | --- |
| [`01-architecture.md`](01-architecture.md) | Layers, the pipeline in one diagram, the determinism contract, repository layouts. |
| [`02-data-model.md`](02-data-model.md) | The thirteen kinds, four classes, four lifecycles, twelve relations, scope, claims, acceptance. |
| [`03-syntax.md`](03-syntax.md) | Lexical structure and canonical formatting. |
| [`04-references-and-versioning.md`](04-references-and-versioning.md) | Keys, revisions, the four reference forms, supersession, `akr.lock`. |
| [`05-validation-rules.md`](05-validation-rules.md) | V-001..V-025, each with its diagnostic code and a failing example. |

**The tool.**

| Document | What to look for |
| --- | --- |
| [`06-compiler-pipeline.md`](06-compiler-pipeline.md) | Stage contracts A–F, hashing, incremental rebuild, ordering guarantees. |
| [`07-cli.md`](07-cli.md) | Every command, its flags, its output, its exit status. |
| [`08-mcp.md`](08-mcp.md) | The agent tool surface, and the `AGENTS.md` text that makes agents use it. |
| [`09-context-assembly.md`](09-context-assembly.md) | How a context bundle is built, deterministically, and why search only ranks. |
| [`10-freshness-and-git.md`](10-freshness-and-git.md) | `observed_at`, watches, `review_after`, staleness, propagation, impact. |
| [`11-projections.md`](11-projections.md) | The seven generated views and their rendering rules. |
| [`12-migration.md`](12-migration.md) | Getting a legacy Markdown pile into the ledger without losing anything. |

**Everything else.**

| Document | What to look for |
| --- | --- |
| [`13-implementation-roadmap.md`](13-implementation-roadmap.md) | Phases P1–P9, the crate layout, and the ten-step dogfood acceptance test. |
| [`14-glossary.md`](14-glossary.md) | Every term, one sentence, and the document that defines it normatively. |
| [`../examples/save-your-skin/`](../examples/save-your-skin/) | A whole small project: sources, lock, generated views, transcripts. |
| [`../fixtures/`](../fixtures/) | Parse, format and validation conformance fixtures. |

## 5. Five minutes with a ledger

Everything below is quoted verbatim from [`../spec/exemplar.akr`](../spec/exemplar.akr),
which is the only source of quotable syntax in this design set.

### A term fixes a word

Vocabulary drift is the cheapest thing to fix and the most expensive to leave. A `term`
record fixes a meaning and gives it an addressable anchor:

```
record demo.term.playable-day/1 : term {
    title "Playable day"
    state active
    scope [ all ]
    definition """
        One in-game day, from the morning wake state to the following morning
        wake state, played end to end by one player.
        """
    aliases [ "playable day", "day-loop build" ]
    claim day-boundary {
        text """
            A day boundary is the morning wake state, not midnight.
            """
    }
    author "dkoepke"
    created_at 2026-01-14
}
```

Read the header as *key* `/` *revision* `:` *kind*. The `claim day-boundary` block can
now be cited from anywhere as `@demo.term.playable-day/1#day-boundary`, and that
citation survives every future revision of the record, because a claim belongs to the
revision that contains it (D-011).

### A policy is revised, and the revision says so

The second revision of a policy is a separate record, in the same file as the first
(V-003), with the first in `superseded` state:

```
record demo.policy.tandem-work/2 : policy {
    title "Engine and simulator advance in tandem"
    state active
    scope [ ref @demo.milestone.playable-day, path "engine/**" ]
    topic tandem-work
    rule """
        No engine change lands without the matching simulator change in the same
        commit, except on the tracks listed under exceptions, where the
        simulator may lag by at most one milestone.
        """
    rationale """
        Divergence between the two has cost more time than the coupling does.
        """
    exceptions [ @demo.track.lighting ]
    claim lag-bound {
        text """
            Permitted simulator lag is at most one milestone, never two.
            """
    }
    claim same-commit {
        text """
            Matching changes ship in one commit, not in a follow-up commit.
            """
        supported_by [ @demo.assessment.coverage-gaps/1#projection-coverage ]
    }
    retired_claims [ no-exceptions ]
    depends_on [ @demo.term.playable-day/1#day-boundary ]
    supersedes [ @demo.policy.tandem-work/1 ]
    supported_by [ @demo.assessment.coverage-gaps#projection-coverage ]
    author "dkoepke"
    created_at 2026-03-04
    source {
        kind legacy
        path "docs/legacy/WORKING-AGREEMENTS.md"
        excerpt """
            Engine and sim should stay in step. (No exceptions were listed.)
            """
    }
}
```

Six mechanically checkable facts are in that record that no Markdown paragraph carries:

- `topic tandem-work` plus `scope` makes this policy *exclusive*: a second live policy
  claiming the same topic over an overlapping scope is a build failure (D-004b).
- `retired_claims [ no-exceptions ]` means a reader who follows an old citation to
  `#no-exceptions` gets "anchor retired at revision 2", not "not found" (D-011).
- `supersedes` puts revision 1 into `superseded` state and is checked for cycles.
- `supported_by` names the assessment this policy rests on. When that assessment goes
  stale, this policy is flagged `at_risk` — flagged, not changed (D-024).
- The two references show both resolution modes: `@demo.assessment.coverage-gaps/1#…`
  is pinned to revision 1 forever; `@demo.assessment.coverage-gaps#…` floats to whatever
  head is live at build time (D-009).
- The `source` block records where the rule came from, which is what makes the migration
  from `WORKING-AGREEMENTS.md` auditable rather than a rewrite (D-022).

### A milestone says what "done" means

```
record demo.milestone.playable-day/1 : milestone {
    title "M1 — playable day"
    state active
    intent """
        A player can start, play and finish one in-game day without a crash, a
        soft-lock, or a placeholder asset.
        """
    target 2026-09-30
    acceptance {
        check full-day-demo {
            statement """
                A recorded session shows one complete in-game day, start to finish.
                """
            method observation
            verified_by [ @pkg.evidence.day-loop-demo/1 ]
        }
        check no-placeholder-assets {
            statement """
                The asset audit reports zero placeholder assets on the day-loop path.
                """
            method command
            command "cargo run -p tools -- audit-assets --path content/day-loop"
        }
    }
    author "dkoepke"
}
```

`check full-day-demo` is satisfied: an `evidence` record with `result pass` is pinned to
it, and its `observed_at` commit descends from the last commit that changed this
milestone's content. `check no-placeholder-assets` has no evidence, so it is not
satisfied, and completing this milestone would fail with `AKR-R022` (V-020). That is the
whole acceptance mechanism: the milestone says what done means and what proved it, and
the evidence record says only what it observed. Evidence never declares what it verifies
(D-016) — one direction, one source of truth.

### What the tool does with those three records

```
$ akr build
parsed 9 records in 1 file
resolved 8 heads, 1 superseded revision
wrote .akr/cache/index.sqlite
wrote docs/generated/ (6 views)
updated akr.lock
```

- `akr check` re-runs stages A–D and exits 0, 1, 2, or 3.
- `akr review-queue` lists records whose watched paths have moved since they were
  observed, and everything downstream of them along `supported_by`, `depends_on` and
  `derived_from`.
- `akr context --goal demo.milestone.playable-day` assembles the bundle an agent should
  read before touching the day loop: the milestone, its plan of record, in-scope
  policies and constraints, blockers, acceptance checks with their status, observations
  touching the paths in question, open questions, contradictions, and staleness
  warnings — in that order, every time, with no model in the loop.

## 6. Anti-goals

Stated once, in [`01-architecture.md`](01-architecture.md) §9, and repeated here because
they explain more about the design than the goals do:

- **No Markdown plus frontmatter.** Frontmatter puts a schema on a document while
  leaving the payload unstructured; the payload is where the claims are.
- **No generic wiki.** Wikis optimise for page-level authorship. The unit here is a
  record.
- **No RDF authoring surface.** The notation must be pleasant to write by hand and
  legible in a diff. An export mapping can come later (`01-architecture.md` §8).
- **No newest-wins.** Two live revisions of one key is a build failure, not a tiebreak.
- **No automatic deletion.** Nothing removes knowledge. Records reach terminal states
  and move to `archive/`, where they still resolve.
- **No line-number identity.** Citations are keys, revisions and anchors, none of which
  a reformat can break.
- **Not everything an agent produces is durable.** That is what `.agent/scratch/` is for.
- **No published standard before dogfooding on two or three real projects.**

## 7. Where to go next

- Building a mental model: [`01-architecture.md`](01-architecture.md).
- Writing records: [`02-data-model.md`](02-data-model.md), then
  [`03-syntax.md`](03-syntax.md).
- Implementing: [`06-compiler-pipeline.md`](06-compiler-pipeline.md), then
  [`13-implementation-roadmap.md`](13-implementation-roadmap.md).
- Adopting on an existing project: [`12-migration.md`](12-migration.md).
- Wiring up an agent: [`08-mcp.md`](08-mcp.md).
