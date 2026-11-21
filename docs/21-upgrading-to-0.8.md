# 21 — Upgrading to AKR 0.8

AKR 0.8 changes how ledgers meet git merges and how context fits a budget. No record
syntax, kind or rule of the language changes: install the new binaries, run `akr build`,
and an existing ledger validates as it did.

The decisions are D-050 (merges) and D-051 (context budgets).

## 1. One step per repository

```
akr git install-hooks
```

Run it once in every existing checkout. It registers the two merge drivers (`merge.akr`,
`merge.akr-rebuild`) in the repository's local git config, which is never cloned. Then add
the attributes that select them to `.gitattributes`, as `akr init` now does for a new
workspace:

```text
.akr/records/**/*.akr merge=akr
.akr/akr.lock merge=akr-rebuild
docs/generated/*.md merge=akr-rebuild
sources/** -text
```

Without the driver definitions git ignores the attributes and merges record files as text,
which is the 0.7 behaviour.

## 2. What changes

| | 0.7 | 0.8 |
| --- | --- | --- |
| Joining two worktrees' ledgers | rebase into one line; a second-parent `passed_at` was refused (`AKR-R022`/`AKR-G011`) | an ordinary merge commit; ancestry follows every parent |
| Observations after a `--no-ff` merge | staled when the merge's tree matched what they observed | staled only when the watched paths changed relative to the observed side |
| Two branches adding records to one `.akr` file | textual conflict, or an unparseable file under `merge=union` | merged by record (`docs/16-change-protocol.md` §11) |
| `pre-commit` during a merge | required a change transaction | passes with `MERGE_HEAD` and no transaction open |
| Context budget below the mandatory sections | `AKR-X021`, nothing delivered | the minimum, with `budget_exceeded` and `required_tokens` |
| Never-reduced context sections | goal, milestone, plan of record, every acceptance check, every staleness warning | goal, depth-1 milestone, the goal's own acceptance checks, contradictions; distant staleness collapses to a count |
| `knowledge.context` with no `budget_tokens` | 4,000-token transport ceiling | 6,000 |
| `knowledge.get` ceiling | 1,500 tokens | 4,000 |
| `workspace` argument in `tools/list` | only when the server's directory had no ledger | always |
| `AKR-G024` on evidence records | raised | not raised: evidence reports what was run; acceptance checks still gate |
| `akr git commit` message transport | command line (failed past 32 KB on Windows) | stdin |

A script that matched on `AKR-X021` should read `budget_exceeded` instead. The code is
retired, not reused.

## 3. What is new

- `akr git merge <branch> [--no-ff] [--no-commit] [-m <message>]` — merge, rebuild lock
  and views, check, commit.
- `akr complete --partial` / `knowledge.complete { "partial": true }` — record the checks
  that passed without completing the record.
- `akr context --section <name>` / `section` — one section only.
- `akr check --limit <n> --offset <n>`, `akr search --slot <name>`, and `--json` for
  `--format json`.
- `akr propose-many --from` accepts the `knowledge.propose_many` JSON shape, and every
  `--from` file may carry a UTF-8 BOM or CRLF line endings.
- `knowledge.source_list` pages with `limit` and `offset`; `knowledge.papercut` takes
  `observed_at`.
- `akr trace` refuses a superseded or abandoned record and names the live successor.
