# 16 — The change protocol

How an AKR ledger and a git history stay in step: change transactions, the semantic staged
delta, generated commit messages, trailers, and the implementation digest that lets
evidence name the code it verified.

Normative for the transaction shape, the trailer names and the refusal conditions.
Advisory for message wording and hook installation. Decided in D-032.

---

## 1. The boundary

> **AKR leads the intent and the verification; git seals the exact snapshot.**

| Question | Authority |
| --- | --- |
| What should be done? | AKR work and decision records |
| Why? | AKR rationale, sources and decisions |
| What proves it worked? | AKR evidence and acceptance checks |
| What exact bytes changed? | The git tree and diff |
| What was committed together? | The git commit |
| Which work did that commit advance? | AKR-generated commit trailers |
| Has it landed on the default branch? | Git reachability |
| Is the work complete? | AKR, on its evidence — never the commit |

Neither system impersonates the other. In particular there is **no `commit` record kind**:
git history is not project knowledge, and duplicating it would drown the decisions and
evidence the ledger exists to hold. See D-032 for the five specific ways a durable commit
record goes wrong.

## 2. The change transaction

A short-lived object holding only what cannot be inferred from a work record or a git diff.

```text
$(git rev-parse --git-path akr/current-change.akr)
```

`--git-path`, not `.git/`: in a linked worktree those are different directories, and a
transaction is per worktree by construction. It is local, disposable, invisible to `akr
search` and `akr context`, and never committed. Its durable projection is the commit
message and the trailers.

```text
change 0.1

id "chg-01k25r5t4g8f7b2n"
base_commit "ff74d3b2..."
kind fix
scope "tone"
summary "gate reconstructed highlight chroma by uncertainty"
primary_work "@raw.work.slice-6-uncertainty-gated-chroma/2"
related_work "@raw.work.slice-1-diagnostic-dumps/2"
```

`summary` is not redundant with the work record's title. "Slice 6 uncertainty-gated chroma
limiting phase" is a good planning name and a poor commit subject, and commit boundaries
are finer-grained than work records anyway — several commits legitimately reference one
active work record without revising it.

## 3. The staged tree is the synchronisation boundary

Not the working tree. An agent finishing a task typically has more modified files than the
change it means to make; "if the code is dirty the ledger must be dirty in the same
direction" is both too strict and too loose, and the git index already answers the question
it was trying to ask.

```bash
git add src/tone.rs src/pipeline.rs \
        .akr/records/raw-autotune/work.akr .akr/akr.lock docs/generated/
akr change prepare --staged
```

## 4. `akr diff --staged` is semantic

It parses the `HEAD` ledger and the index ledger and compares the two, and reports:

```text
records added
revisions added
state transitions
evidence added
code
```

It never reads `git diff` text. A reformat, a reordering, or a record moved between files
is not a semantic change, and a textual diff cannot say so. Generated views and `akr.lock`
are validated but excluded from the descriptive file summary.

## 5. What preparation refuses

| Condition | Why |
| --- | --- |
| Nothing staged | There is no change to describe |
| Code staged, no work reference and no `--untracked-reason` | This is the drift the protocol exists to catch |
| Several work records moved, none named primary | The subject can only be about one; guessing misdescribes the commit |
| The staged tree moved after preparation | The message would describe a commit that is no longer the one being made |

The exemption is explicit and cheap, so nobody invents a fake work record for a formatting
pass:

```bash
akr change begin --kind chore --scope ci \
  --summary "pin the windows build image" \
  --untracked-reason "repository maintenance; no project behaviour changed"
```

## 6. The generated message

Five sources and no sixth: the transaction summary becomes the subject, the primary work
record's intent explains why, the semantic delta says what happened, the evidence records
say what was verified, and the trailers carry the links.

```text
fix(tone): gate reconstructed highlight chroma by uncertainty

Restore the display-linear near-white proxy and combine it with the
reconstruction uncertainty map.

- raw.work.slice-6-uncertainty-gated-chroma active -> completed
- raw.work.slice-8-local-tone-hdr proposed -> active

Verified by:
- Uncertainty-gated chroma differential

AKR-Change: chg-01k25r5t4g8f7b2n
AKR-Work: @raw.work.slice-6-uncertainty-gated-chroma/2
AKR-Evidence: @raw.evidence.slice-6-verify/1
AKR-Graph: sha256:5a0aa895...
AKR-Tree: 41cd7e...
```

The full evidence record does not go in. Git wants a concise historical explanation; the
ledger keeps the method, artefacts, commands, metrics and acceptance mapping. A second copy
is a second thing that goes stale.

Generation is deterministic: the same transaction and staged tree produce the same bytes. A
message an author cannot predict is a message nobody reviews.

## 7. Trailers, not stored commit hashes

`AKR-Change`, `AKR-Work`, `AKR-Evidence`, `AKR-Decision`, `AKR-Graph`, `AKR-Tree` — all
`git interpret-trailers`-compatible.

The link points *from the commit to the records*, which is the direction with no hash
cycle. Trailers survive ordinary rebases, usually survive cherry-picks, can be collected
during squash preparation, are searchable with `git log --grep`, and let every AKR-to-git
association be rebuilt by walking history.

```bash
akr git log raw-autotune.work.slice-6-uncertainty-gated-chroma
```

## 8. The implementation digest

Evidence should be able to say which code it verified. It cannot store the commit id — that
commit does not exist yet — and it cannot store the tree id either, because writing it into
a file inside that tree changes the tree.

The digest is over the **implementation portion** of the staged tree: sorted
`(path, mode, blob)` triples, excluding `.akr/**` and `docs/generated/**`.

Excluding AKR's own files breaks the cycle, and it also makes the digest mean the right
thing — the implementation that was tested, not the tree including the note about having
tested it. Acceptance freshness uses Git ancestry normally. When evidence and the verified
record have the same last-change commit, it recognizes the prepared-tree case: the test
ran against the parent plus staged implementation, and the future commit did not yet exist
to be named. Evidence from any older commit remains too old.

Generated views use the same direction of indirection. Their banner embeds the stable
`source-graph`, never the future commit hash; `AKR-Graph` trailers map commits back to that
graph after Git advances.

## 9. Hooks are guardrails

```bash
akr git install-hooks
```

writes two-line wrappers:

```sh
#!/bin/sh
exec akr git-hook pre-commit "$@"
```

The checks stay in the binary. A hook that carried them would be a second implementation
nobody keeps in step with the first, and hooks are bypassable anyway — CI is the final
authority.

**A commit that changes only its message passes.** The transaction closes at
`akr git commit`, so every later commit in the worktree meets a hook with no transaction
to check — including `git commit --amend` that rewords a subject, which leaves the tree
untouched and the AKR trailers byte-identical. Refusing that made `--no-verify` the only
way to fix a commit subject, which is a bad habit to teach for a change the protocol has
no opinion about. The hook detects it precisely: no transaction open, and an index equal
to `HEAD`. An ordinary commit has something staged or git refuses it before any hook runs,
and an amend that also stages changes is still refused. A transaction in flight is still
checked, because an author mid-protocol wants the check. Deciding the subject up front
remains better and needs no amend: `akr change begin --summary` and `--kind` write it.

## 10. A commit never completes work

```text
commit created   ≠ work completed
commit landed    ≠ acceptance satisfied
tests passed     ≠ all evidence accepted
```

Git can show that code exists, that CI ran and that a commit reached the default branch. It
cannot decide whether the acceptance criteria were met, whether the output quality is
acceptable, or whether a benchmark is meaningful. The transaction consumes AKR state
transitions; it never invents them.

---

Next: [`10-freshness-and-git.md`](10-freshness-and-git.md) for what git tells the ledger
about staleness, or [`15-external-sources.md`](15-external-sources.md) for the other place
outside material meets the ledger.
