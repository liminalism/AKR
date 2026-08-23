# Conformance Fixtures

Hand-authored fixtures for the parser, formatter, and validator. They are written ahead
of the implementation on purpose: a fixture is a claim about what the specification
means, and writing them is how the specification gets tested before any code exists.
Phases P2 and P3 of `docs/13-implementation-roadmap.md` consist largely of making these
pass.

---

## 1. Layout

```
fixtures/
    parse/ok/NNN-name.akr              must parse
    parse/err/NNN-name.akr             must fail at parse
    parse/err/NNN-name.expected        the diagnostics it must produce
    format/NNN-name.in.akr             input to `akr fmt`
    format/NNN-name.out.akr            required output, byte for byte
    validate/project.akr               namespace declarations for validate fixtures
    validate/ok/NNN-name.akr           must pass `akr check`
    validate/err/vNNN-name.akr         must fail rule V-NNN
    validate/err/vNNN-name.expected    the diagnostics it must produce
    validate/err/vNNN-name/            multi-file fixture: *.akr plus `expected`
```

Validate fixtures are named for the rule they exercise, so `v017-missing-disposition`
tests V-017 and nothing else. Parse and format fixtures are numbered by topic.

## 2. The `.expected` format

One diagnostic per line:

```
CODE line[:col]
```

Lines beginning with `#` are comments and are ignored. Diagnostics are listed in the
order the tool emits them, which is sorted by file path and then by span
(`docs/05-validation-rules.md` §1.3). A column is given where the fixture isolates a
token; it is omitted where the diagnostic points at a whole record.

`.expected` is the **complete** set of diagnostics for the stage under test. A fixture
that produces an extra diagnostic fails, which is what keeps fixtures isolated to one
fault. Multi-file fixtures use a file named `expected` in the fixture directory, with
paths implied by the directory's contents.

### One deliberate exception

`validate/err/v017-missing-disposition` lists **two** codes, and this is a property of
the design rather than a leaky fixture. A live child may pin a superseded plan revision
only when the superseding record disposes of it (`docs/04` §5.1); that exemption is what
gives V-017 something to bite on. Take the disposition away and two things become true
at once — the disposition is missing (`AKR-R014`) and the `part_of` reference has lost
its excuse (`AKR-L021`) — so no arrangement of records separates them. Every other
`validate/err` fixture produces exactly one diagnostic.

Where a fixture produced a second code that was *not* by design, the fixture was fixed
rather than its `.expected` widened: `v004-retired-anchor` now cites an assessment
instead of a policy, so `supported_by`'s range is satisfied, and
`v011-resolved-question-no-resolution` now supplies the `resolves` edge so that only the
missing `resolution` slot is at fault.

## 3. What each group asserts

| Group | Assertion | Stage |
| --- | --- | --- |
| `parse/ok` | Parses with zero diagnostics, and `format(format(s)) == format(s)` | A |
| `parse/err` | Produces exactly the listed diagnostics, and no later stage runs | A |
| `format` | `format(in) == out`, byte for byte; and `format(out) == out` | A |
| `validate/ok` | `akr check` exits 0 against `validate/project.akr` | A–D |
| `validate/err` | Produces exactly the listed diagnostics | B, C, or D |

Every `format` fixture doubles as a round-trip test: the `.out.akr` side must be a fixed
point, so `007-idempotent` is not the only idempotence check, just the most explicit one.

## 4. Coverage matrix — grammar

| Fixture | Productions exercised (`spec/grammar/akr.ebnf`) |
| --- | --- |
| `parse/ok/001-minimal-record` | `file`, `header`, `project_decl`, `record`, `body`, `slot`, `prose` |
| `parse/ok/002-all-literals` | `date`, `commit`, `boolean`, `glob`, `scope_term` (all three forms), enum |
| `parse/ok/003-prose-dedent` | `prose`, dedent rule, raw-backslash handling (`docs/03` §3.7) |
| `parse/ok/004-comment-placement` | `comment`, leading/trailing/scope-end attachment (`docs/03` §3.2) |
| `parse/ok/005-arrays` | `array`, `array_items`, wrapped and inline forms |
| `parse/ok/006-blocks` | `block`, `block_head` (identifier and reference), nested `check` in `acceptance` |
| `parse/ok/007-unicode-prose` | UTF-8 prose, `unicode_escape` in a quoted string |
| `parse/ok/008-string-escapes` | `escape` — every legal form |
| `parse/ok/009-key-depth` | `key` at two and eight segments |
| `parse/ok/010-project-file` | `namespace_decl`, `defaults_block`, `integer` |
| `parse/ok/011-lock-file` | `akr-lock` header, `lock_item`, string `block_head` |
| `parse/ok/012-planning-note` | The `note` slot on `work` and `track` (D-026), in its canonical position as the last content slot of its kind |

## 5. Coverage matrix — diagnostics

| Fixture | Code | Rule |
| --- | --- | --- |
| `parse/err/001-abbreviated-commit` | `AKR-P021` | — |
| `parse/err/002-local-timestamp` | `AKR-P023` | — |
| `parse/err/003-unknown-escape` | `AKR-P012` | — |
| `parse/err/004-duplicate-slot` | `AKR-P031` | — |
| `parse/err/005-newline-in-string` | `AKR-P011` | — |
| `parse/err/006-tab-in-prose` | `AKR-P015` | — |
| `parse/err/007-underscore-in-key` | `AKR-P041` | — |
| `parse/err/008-bad-reference` | `AKR-P043` | — |
| `parse/err/009-unclosed-brace` | `AKR-P044` | — |
| `parse/err/010-key-one-segment` | `AKR-P042` | — |
| `parse/err/011-leading-zero-revision` | `AKR-P025` | — |
| `parse/err/012-duplicate-claim-anchor` | `AKR-P032` | — |
| `validate/err/v001-unresolved-reference` | `AKR-L001` | V-001 |
| `validate/err/v002-undeclared-namespace` | `AKR-L004` | V-002 |
| `validate/err/v003-key-split` | `AKR-L006` | V-003 |
| `validate/err/v004-retired-anchor` | `AKR-L012` | V-004 |
| `validate/err/v005-wrong-relation-target` | `AKR-L031` | V-005 |
| `validate/err/v006-terminal-reference` | `AKR-L021` | V-006 |
| `validate/err/v007-illegal-state` | `AKR-T011` | V-007 |
| `validate/err/v008-missing-slot` | `AKR-T001` | V-008 |
| `validate/err/v009-observation-no-commit` | `AKR-T021` | V-009 |
| `validate/err/v010-evidence-missing-result` | `AKR-T022` | V-010 |
| `validate/err/v011-resolved-question-no-resolution` | `AKR-T031` | V-011 |
| `validate/err/v012-two-live-heads` | `AKR-R001` | V-012 |
| `validate/err/v013-topic-conflict` | `AKR-R002` | V-013 |
| `validate/err/v014-supersession-cycle` | `AKR-R011` | V-014 |
| `validate/err/v015-depends-cycle` | `AKR-R012` | V-015 |
| `validate/err/v016-after-cycle` | `AKR-R013` | V-016 |
| `validate/err/v017-missing-disposition` | `AKR-R014`, `AKR-L021` | V-017 (see §2) |
| `validate/err/v018-two-plans` | `AKR-R018` | V-018 |
| `validate/err/v019-live-depends-terminal` | `AKR-R021` | V-019 |
| `validate/err/v020-completed-unsatisfied` | `AKR-R022` | V-020 |
| `validate/err/v021-decision-cites-nothing` | `AKR-R031` | V-021 |
| `validate/err/v022-observation-no-provenance` | `AKR-R032` | V-022 |
| `validate/err/v023-contradiction` | `AKR-R041` | V-023 |
| `validate/err/v024-sealed-modified` | `AKR-R051`, `AKR-R052` | V-024 |
| `validate/err/v025-evidence-scratch-artifact` | `AKR-T023` | V-025 |

Every rule `V-001`–`V-025` has exactly one failing fixture. Codes with no fixture are
listed below with the reason.

## 6. Codes with no fixture

| Code | Reason |
| --- | --- |
| `AKR-P001`–`P009` except those above | Structural or environmental (BOM, CRLF, missing final newline, version mismatch). Better covered by unit tests over byte sequences than by files in git, which normalising tools would silently repair. |
| `AKR-F001`–`F011` | Covered by the `format/` pairs: a formatter that produces `out` from `in` has satisfied every `F` code the pair exercises. `F001` is the umbrella reported by `akr fmt --check`. |
| `AKR-T002`–`T007`, `T012`–`T014`, `T032`–`T034` | Same shape as `T001`/`T011`, which have fixtures. Add one when an implementation disagrees about any of them. |
| `AKR-L002`, `L003`, `L005`, `L011`, `L032`, `L033`, `L041`, `L042` | Same shape as the linked-fixture codes in their group. |
| `AKR-R015`–`R017`, `R023` | Secondary diagnostics of rules whose primary code has a fixture. |
| Runtime codes (`I`, `E`, `X`, `G`, `C`, `M`) | Owned by `spec/diagnostics/codes-runtime.md`; their fixtures belong with the pipeline and CLI work, not here. |

A fixture is worth adding when two implementations could plausibly disagree. A fixture
for every code would be a maintenance tax with no return.

## 7. Relationship to the worked example

`examples/save-your-skin/` is the opposite artifact: a complete, **valid** project that
exercises every mechanism at once. Fixtures isolate one thing each and are mostly
broken on purpose. Neither substitutes for the other — the example proves the pieces
compose, the fixtures prove each piece has an edge.

The example is deliberately never made invalid to demonstrate a rule. Every failure mode
lives here instead.
