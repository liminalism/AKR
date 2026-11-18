# 20 — Upgrading a ledger to AKR 0.7

AKR 0.7 changes what agents are asked to write, not what a ledger already holds. Install
the new binaries, run `akr build`, and an existing ledger validates as it did: no sealed
revision changes, no reference breaks, and no record needs editing.

The reasons are in `docs/19-ledger-usage-and-reasoning-audit.md`; the decisions are
D-046 to D-049.

## 1. What changes for a ledger written by 0.6

| | Before | After |
| --- | --- | --- |
| Vocabulary version in `akr.lock` | `0.2` | `0.3`, written by the next `akr build` |
| Records of `term`, `requirement`, `constraint`, `assessment`, `track` | written | still valid, still revisable; **no new key** may take one (`AKR-T008`) |
| `CURRENT-STATE.md`, `ROADMAP.md` | a section per kind, `_(none)_` when empty | a retired kind's section appears only when the ledger has such records |
| Commit messages from `akr git commit` | opened with the work record's `intent` | carry `AKR-Why:` trailers instead; no intent copy |
| `akr evidence add --summary` | title and summary stored twice | a short summary becomes the title and is stored once |

`akr check` gains one build fact, for example:

```
6 live record(s) of retired kinds (1 requirement -> policy, 5 assessment -> observation) — still valid; fold with ...
```

It is a fact, not a diagnostic. Nothing fails because of it.

## 2. What is new

**A work record carries its reasoning.** `akr trace <work> "<why>"` (`knowledge.trace`)
appends a short entry to the work record, in place — no revision, no acceptance check
reopened. One entry is one reason or one dead end, about 25 words; over 40 is refused.
`akr get <work>` shows each entry with the commit that introduced it. It is never
required.

**A command check records its own pass.** `akr complete <key> --pass <check>`
(`knowledge.complete` with `{"checks": {"<check>": {"passed_at": "HEAD"}}}`) completes
work without an evidence record, for a `method command` check whose command was run and
passed. Evidence records remain for measurements, failures, artefacts and manual
observations.

**The brief says what the ledger is for.** `scripts/agent-section.md` is the one text;
`akr init` embeds it. Re-run `scripts/setup-akr-mcp.sh` (or the `.ps1`) to refresh the
installed copies between the `AKR_START` / `AKR_END` markers.

**Hooks by default.** `akr init` installs them in a git repository. For a workspace that
already exists:

```
akr git install-hooks
```

Hooks live in the git directory and are not cloned, so each checkout needs this once.
`akr start` says so in one line when they are absent.

## 3. Folding a retired kind, when it is worth it

Nothing has to be folded. A record leaves a retired kind by an ordinary supersession:

```
akr supersede sys.assessment.projection-gaps --with sys.obs.projection-gaps --from body.akr
```

where `body.akr` holds the replacement's slots. The old head becomes `superseded`, the
new record carries `supersedes [ @sys.assessment.projection-gaps/1 ]`, and unpinned
references follow it. `supersedes` crosses kinds only along this table:

| From | To | The replacement needs |
| --- | --- | --- |
| `assessment` | `observation` | `observed_at`, and provenance (`method`, a source, or `supported_by`) |
| `requirement` | `policy` | `rule` in place of `statement` |
| `constraint` | `policy` | `rule`; `measure` carries over |
| `term` | `policy` | `rule` in place of `definition`; `aliases` carries over |
| `track` | `milestone` | an acceptance block — or fold standing work into a `policy` instead |

`akr explain <kind>` prints the slots of the kind you are folding into.

## 4. For the three audited projects

- **SYS** holds nearly all the retired-kind records (about 200). They stay valid. Fold
  the ones that are still read; leave the rest.
- **Lege-ecosystem** and **nail-salon** hold almost none and need only the binaries, the
  refreshed brief, and `akr git install-hooks`.
- In all three, the change that matters is behavioural: `akr trace` as work proceeds,
  `--pass` instead of an evidence record for a routine test run, and commits through
  `akr change`.
