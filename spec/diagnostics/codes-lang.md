# Diagnostic Registry — Language Stages (`P`, `F`, `T`, `L`, `R`)

The complete registry for the five language stages: parse, format, type-check, link, and
resolve. Runtime stages (`I`, `E`, `X`, `G`, `C`, `M`) are in
`spec/diagnostics/codes-runtime.md`.

The code scheme, severity model, and rendering are specified in
`spec/diagnostics/README.md`. Rules `V-001`–`V-025` are catalogued in
`docs/05-validation-rules.md` and frozen in `spec/tables/vocabulary.json`.

**Reading the tables.** *Sev* is `error` or `warning`; under the default `--strict`
profile a warning fails the build. *Rule* is the `V-nnn` rule that raises the code, or
`—` for faults with no rule (most parse and format codes are structural, not rule-driven).
*Message* is the template, with `{placeholders}`.

Codes are never renumbered and never reused. Numbers are grouped in tens by topic with
gaps left for growth.

---

## `P` — Parse

Lexing, grammar, and literal forms. One file's bytes are all this stage can see. A parse
error halts the build for that file; no later stage runs on it.

### P001–P009 — File and header

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-P001` | Unexpected token | error | — | `expected {expected}, found {found}` | The generic syntax error. The message always names what was expected, because "syntax error" alone is useless. |
| `AKR-P002` | Byte order mark | error | — | `file begins with a UTF-8 byte order mark` | Strip the BOM. AKR files are UTF-8 without one (`docs/03` §2). |
| `AKR-P003` | Carriage return | error | — | `carriage return at line {line}; AKR files use LF line endings` | Configure the editor or `.gitattributes`. CRLF is rejected rather than normalised so the canonical form has one representation. |
| `AKR-P004` | Missing final newline | error | — | `file does not end with a newline` | Add one. `akr fmt` fixes it. |
| `AKR-P005` | Missing header | error | — | `expected `akr` or `akr-lock` header on the first non-comment line` | Every file declares its profile and grammar version. |
| `AKR-P006` | Unsupported grammar major version | error | — | `grammar version {found} is not supported by akr {tool}` | The tool is older than the file. Upgrade the tool. |
| `AKR-P007` | Unknown grammar minor version | warning | — | `grammar version {found} is newer than {supported}; parsing as {supported}` | Pre-1.0 there are no forward-compatibility promises (D-021). Fails under strict; `--lenient` proceeds. |
| `AKR-P008` | Missing project declaration | error | — | `expected `project <name>` after the header` | Every file names its project. |
| `AKR-P009` | Empty source file | warning | — | `file contains no records` | Usually a leftover after moving records. Delete it or add records. |

### P011–P019 — Strings and prose

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-P011` | Newline in quoted string | error | — | `unescaped newline in a quoted string; use a prose block` | Multi-line text is `"""..."""` (`docs/03` §3.7). |
| `AKR-P012` | Unknown escape sequence | error | — | `unknown escape `\{char}`; legal escapes are \" \\ \n \t \r \u{...}` | The escape set is closed. Inside a prose block there are no escapes at all. |
| `AKR-P013` | Unterminated string | error | — | `unterminated quoted string` | Missing closing `"`. |
| `AKR-P014` | Unterminated prose block | error | — | `unterminated prose block opened at {line}` | The closing `"""` must be alone on its line. |
| `AKR-P015` | Tab in prose indentation | error | — | `tab character in prose indentation at line {line}` | Prose dedent is defined over spaces; a tab makes the common prefix ambiguous. |
| `AKR-P016` | Prose content on the opening line | error | — | `prose block content must begin on the line after `"""`` | Move the first line down. |
| `AKR-P017` | Prose closing delimiter shares a line | error | — | `closing `"""` must be the only content on its line` | |
| `AKR-P018` | Prose block contains `"""` | error | — | `prose block content may not contain `"""`` | Quoting AKR source inside a record belongs in a document, not a record. |

### P021–P029 — Scalar literals

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-P021` | Abbreviated commit hash | error | — | `commit hash must be 40 hex digits, found {n}` | Abbreviations collide as history grows and would make parsing depend on repository state (D-008). |
| `AKR-P022` | Invalid date | error | — | `{value} is not a valid calendar date` | Includes 2026-02-30 and similar. |
| `AKR-P023` | Timestamp is not UTC | error | — | `timestamp must end in `Z`; offsets are not permitted` | A ledger read by agents in unknown timezones has one clock (D-008). |
| `AKR-P024` | Leading zero in integer | error | — | `integer literal may not have a leading zero` | One representation per value. |
| `AKR-P025` | Invalid revision number | error | — | `revision must be a positive integer without a leading zero` | Revisions start at 1. |
| `AKR-P026` | Uppercase in commit hash | error | — | `commit hash must be lowercase hex` | |

### P031–P049 — Structure

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-P031` | Duplicate slot | error | — | `slot `{name}` appears twice in this {record\|block}` | Slots are unique; multi-valued content uses a plural array slot (D-012). |
| `AKR-P032` | Duplicate block head | error | — | `{block} `{head}` appears twice` | Two `claim` blocks with the same anchor, or two `check` blocks with the same id. |
| `AKR-P033` | Block head where none permitted | error | — | `block `{name}` does not take a head` | `acceptance` and `source` take no head. |
| `AKR-P034` | Missing block head | error | — | `block `{name}` requires a {kind-of-head}` | `claim` and `check` need an identifier; `disposition` needs a reference. |
| `AKR-P041` | Malformed identifier | error | — | `{value} is not a valid {segment\|name}` | Segments carry hyphens, names carry underscores, and never the reverse (D-005). |
| `AKR-P042` | Malformed key | error | — | `key must have 2 to 8 segments, found {n}` | |
| `AKR-P043` | Malformed reference | error | — | `reference must be @key[/revision][#anchor]` | Four forms, no others (D-009). |
| `AKR-P044` | Unbalanced brace | error | — | `unclosed `{` opened at {line}` | |
| `AKR-P045` | Unbalanced bracket | error | — | `unclosed `[` opened at {line}` | |
| `AKR-P046` | Content after closing brace | error | — | `unexpected content after the end of a record` | Usually a missing `record` keyword on the next record. |

---

## `F` — Format

Canonicalisation. These are raised by `akr fmt --check` and by `akr check`, which
verifies formatting before doing anything else — an uncanonical file has an unstable
content hash, and the seal check (V-024) depends on it.

`akr fmt` fixes every code in this table. None of them requires a human decision.

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-F001` | File is not canonically formatted | error | — | `{path} is not canonically formatted; run `akr fmt`` | The umbrella code reported by `akr fmt --check`. The specific codes below are reported by `akr fmt --check --explain`. |
| `AKR-F002` | Slot order is not canonical | error | — | `slot `{name}` is out of canonical order` | Order is fixed (`docs/03` §6.3), not a preference. |
| `AKR-F003` | Records are not sorted | error | — | `record {key} is out of order in this file` | Records sort by key then revision (`docs/03` §6.2). |
| `AKR-F004` | Array wrapping is not canonical | error | — | `array should be {inline\|one element per line}` | 96-column threshold. |
| `AKR-F005` | Indentation is not canonical | error | — | `expected {n} spaces of indentation, found {m}` | Four spaces per level, never tabs. |
| `AKR-F006` | Trailing whitespace | error | — | `trailing whitespace` | |
| `AKR-F007` | Blank line inside a record body | error | — | `unexpected blank line inside a record body` | One blank line between records, none within. |
| `AKR-F008` | Trailing comma | error | — | `trailing comma in array` | Accepted on input, removed on output. |
| `AKR-F009` | Empty array | warning | — | `empty array `[ ]` means the same as omitting `{name}`` | The formatter removes it. |
| `AKR-F010` | Prose indentation is not canonical | error | — | `prose content should be indented {n} spaces` | Content sits one level deeper than its slot. |
| `AKR-F011` | Unsorted array elements | error | — | `{reference\|scope} array elements are not in canonical order` | Applies to reference and scope arrays only, which carry no authored order. String, glob and identifier arrays keep the order they were written in and never raise this (`docs/03` §6.3, §6.6). |

---

## `T` — Type-check

One record against the vocabulary. This stage sees a record and
`spec/tables/vocabulary.json`, and nothing else — no other records, no graph, no git.

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-T001` | Missing required slot | error | V-008 | `{kind} requires slot `{name}`` | See the kind's table in `docs/02` §4. |
| `AKR-T002` | Unknown slot | error | V-008 | `{kind} has no slot `{name}`; did you mean `{suggestion}`?` | Rejecting unknown slots is what keeps the vocabulary closed. |
| `AKR-T003` | Unknown kind | error | V-008 | `{kind} is not a record kind` | Twelve kinds (D-001). `plan` and `goal` are not among them; see `docs/02` §12. |
| `AKR-T004` | Unknown block | error | V-008 | `{name} is not a block` | Five blocks, and no others (`docs/02` §1.1). |
| `AKR-T005` | Block not permitted for kind | error | V-008 | `{kind} may not contain a `{block}` block` | `acceptance` is milestone and work only; `disposition` is planning kinds only. |
| `AKR-T006` | Missing required block | error | V-008 | `{kind} requires an `acceptance` block` | Milestones without acceptance are the failure the format exists to prevent. |
| `AKR-T007` | Block in the wrong place | error | V-008 | ``check` blocks appear only inside `acceptance`` | |
| `AKR-T011` | Illegal state for kind | error | V-007 | `{state} is not a valid state for {kind} ({class}); expected one of {states}` | Four class lifecycles (`docs/02` §5). `needs-review` is not a state anywhere (D-003). |
| `AKR-T012` | Unknown enum value | error | V-008 | `{value} is not a valid `{slot}`; expected one of {values}` | |
| `AKR-T013` | Wrong value type | error | V-008 | `slot `{name}` expects {expected}, found {found}` | Where a stray integer or unquoted string lands (`docs/03` §3.8). |
| `AKR-T014` | Array/scalar mismatch | error | V-008 | `slot `{name}` expects {an array\|a single value}` | Relation slots are always arrays, even with one element. |
| `AKR-T021` | Observation missing `observed_at` | error | V-009 | `observation requires `observed_at`` | An observation without a commit is a rumour, and can never go stale. |
| `AKR-T022` | Evidence missing required slot | error | V-010 | `evidence requires `{slot}`` | `result`, `method`, and `observed_at`, all three. |
| `AKR-T023` | Evidence artefact under disposable scratch | error | V-025 | `artifact {path} is under .agent/scratch, which is disposable` | Unkept scratch is pruned on an ordinary handoff (D-036), so the citation outlives its backing. |
| `AKR-T031` | Resolved question missing resolution | error | V-011 | `question in state `resolved` requires a `resolution` slot` | The companion check — that something `resolves` it — is at resolve, under the same rule. |
| `AKR-T032` | Malformed scope term | error | V-008 | `scope term must be `all`, `ref @key`, or `path "glob"`` | Three forms (D-010). |
| `AKR-T033` | Malformed glob | error | V-008 | `glob may use * ** ? and [...]; brace expansion and negation are not supported` | |
| `AKR-T034` | `topic` on a non-normative kind | error | V-008 | ``topic` applies only to normative kinds` | Exclusivity (V-013) is a governance rule. |

---

## `L` — Link

All records, references unresolved into revisions. This stage sees the whole record set
but not the graph's transitive properties.

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-L001` | Unresolved reference | error | V-001 | `no record with key {key}; did you mean {suggestion}?` | Suggestion is by edit distance over declared keys. |
| `AKR-L002` | Key has no resolvable head | error | V-001 | `{key} has no single head; revisions {revs} are unsuperseded (no incoming supersedes edge)` | Only raised when the second tier of head resolution is also ambiguous (`docs/04` §3). A key whose head is merely terminal resolves fine; V-019 decides whether that is acceptable. The diagnostic names the edge-less revisions; `akr revise` adds missing same-key `supersedes` edges when it creates the next head. |
| `AKR-L003` | Unknown revision | error | V-001 | `{key} has no revision {n}; revisions are 1..{max}` | |
| `AKR-L004` | Undeclared namespace | error | V-002 | `namespace `{ns}` is not declared in project.akr` | The defence against typo-drift creating a second graph. |
| `AKR-L005` | Project mismatch | error | V-002 | `file declares project `{a}`, project.akr declares `{b}`` | Usually a file copied between repositories. |
| `AKR-L006` | Key split across files | error | V-003 | `revisions of {key} appear in {file_a} and {file_b}` | A key's history must be one diff. |
| `AKR-L011` | Unknown anchor | error | V-004 | `{key}/{rev} has no claim or check `{anchor}`` | |
| `AKR-L012` | Retired anchor | error | V-004 | `claim `{anchor}` was retired at revision {n}` | Pin to a revision that had it, or cite the replacement. This distinct message is why `retired_claims` exists (D-011). |
| `AKR-L021` | Historical reference in a live slot, or a `supersedes` edge that is not one | error | V-006 | `slot `{slot}` may not reference {key}/{rev}, which is {state}` | Historical and structural exemptions may point at terminal records; `depends_on` may additionally point at completed planning records, and `resolves` at the question it resolved, whether that revision is `resolved` or the `superseded` one that was open when the answer was written. The code also covers the two ways a `supersedes` edge fails to be historical: naming a key with no revision, which follows the head and re-aims itself as the target is revised (D-044), and naming a still-live record in another key, which records a replacement that never happened — across keys the edge is documentation and `akr supersede` is what moves the state. |
| `AKR-L031` | Relation target out of range | error | V-005 | `{relation} may not target a {kind}; its range is {range}` | The diagnostic also names relations that would accept the target. |
| `AKR-L032` | Relation source out of domain | error | V-005 | `a {kind} may not declare `{relation}`; its domain is {domain}` | |
| `AKR-L033` | Kind-restricted slot target invalid | error | V-005 | `{slot} may only reference {kinds}` | Applies to `exceptions`, `into`, and `ref` scope terms. |
| `AKR-L041` | Duplicate revision | error | V-001 | `{key}/{rev} is defined twice` | Two records with the same revision identifier, usually a bad merge. |
| `AKR-L042` | Revision gap | warning | V-001 | `{key} jumps from revision {a} to {b}` | Not fatal, but usually means a revision was deleted rather than superseded. |

---

## `R` — Resolve

The whole graph, heads resolved, plus git history and the lock. Everything here is a
property of the ledger as a system rather than of any one record.

### R001–R009 — Heads and exclusivity

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-R001` | Two live revisions of one key | error | V-012 | `{key} has {n} live revisions: {list}` | The core identity invariant (D-004a). Supersede or withdraw all but one. Seeing this mid-revision is normal. |
| `AKR-R002` | Normative topic conflict | error | V-013 | `{a} and {b} are both live, share topic `{topic}`, and have overlapping scope` | The diagnostic prints the overlapping scope terms so the conservative overlap test (D-010) is auditable. |

### R011–R019 — Graph shape and planning

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-R011` | Supersession cycle | error | V-014 | `supersession cycle: {cycle}` | Prints the full cycle. Decide which record is current. |
| `AKR-R012` | Relation cycle | error | V-015 | `cycle in `{relation}`: {cycle}` | Covers `depends_on`, `derived_from`, `part_of`, `implements`, `blocks`. |
| `AKR-R013` | Ordering cycle | error | V-016 | `cycle in `after`: {milestone sequence}` | Separate from R012 so the message can render a plan sequence. |
| `AKR-R014` | Missing disposition | error | V-017 | `{superseding} supersedes {superseded} but does not dispose of {children}` | Lists every child needing one. The most valuable check in the system (D-017). |
| `AKR-R015` | Disposition outcome mismatch | error | V-017 | ``into` is {required\|forbidden} for outcome `{outcome}`` | Required for `carried_forward` and `completed_elsewhere`; forbidden for `intentionally_dropped`. |
| `AKR-R016` | Disposition of a non-child | error | V-017 | `{target} is not `part_of` {superseded}` | Usually a copy-paste from a previous replan. |
| `AKR-R017` | Supersession across kinds | error | V-014 | `a {kind_a} may not supersede a {kind_b}` | Supersession replaces like with like. |
| `AKR-R018` | Multiple plans of record | error | V-018 | `{target} has {n} live plans of record: {list}` | Model an alternative plan as a revision, not a second plan. |

### R021–R029 — Liveness and completion

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-R021` | Live record relies on an invalid terminal record | error | V-019 | `{key} is {state} but `{slot}` resolves to {target}, which is {target_state}` | The resolved counterpart of L021: repoint or revise when a floating head becomes invalid. A completed planning prerequisite remains valid. |
| `AKR-R022` | Completion with unsatisfied acceptance | error | V-020 | `{key} is `completed` but check `{check}` is not satisfied` | Names why: no evidence, evidence not `pass`, or evidence predates the record's last content change (D-016). Evidence co-committed with that change is current; the age gate is waived for a `legacy`-sourced record (D-028). |
| `AKR-R023` | Blocked without a blocker | warning | V-020 | `{key} is `blocked` but no live record `blocks` it` | A blocked item with no blocker is a stalled item nobody has named. |

### R031–R049 — Justification and contradiction

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-R031` | Active decision cites nothing | error | V-021 | `active decision {key} cites no requirement, policy, constraint, observation, or evidence` | A decision resting on nothing is a preference. Cite what motivated it — an observation counts (D-042) — or leave it `proposed`, which means recorded rather than pending. |
| `AKR-R032` | Observation lacks provenance | error | V-022 | `verified observation {key} has no `method`, `source`, or supporting evidence` | The commit says when; this says how. |
| `AKR-R041` | Undispositioned contradiction | error | V-023 | `{a} contradicts {b}; both are live and the contradiction is not acknowledged` | Resolve it by superseding one side, or set `acknowledged true` and explain. Acknowledging is a legitimate ledger state. |

### R051–R059 — Sealing and the lock

| Code | Title | Sev | Rule | Message | Cause and fix |
| --- | --- | --- | --- | --- | --- |
| `AKR-R051` | Sealed revision modified | error | V-024 | `{key}/{rev} is {state} and sealed; recorded {a}, computed {b}` | Move the change into a new revision (`akr revise`). Comments are excluded from the hash, so commentary is always safe to add. |
| `AKR-R052` | Lock stale or incomplete | error | V-024 | `akr.lock does not match the sources: {detail}` | Run `akr build`. Never hand-merge a lock; regenerate it (`docs/04` §8.4). |

---

## Retired codes

None. This section exists so that the first retirement has an obvious home: a retired
code stays listed, marked retired, with a pointer to what replaced it. Codes appear in
CI logs, commit messages, and agent transcripts, and recycling one falsifies history.

---

## Coverage

Every code above is cited by at least one of `docs/02-data-model.md`,
`docs/03-syntax.md`, `docs/04-references-and-versioning.md`,
`docs/05-validation-rules.md`, or a fixture under `fixtures/`, and
`tools/check-design.py` enforces that in both directions: no uncited code, no
unregistered citation.

Every rule `V-001`–`V-025` maps to exactly one primary code here, and the mapping in
`spec/tables/vocabulary.json` is authoritative where this document and that file could
ever disagree.
