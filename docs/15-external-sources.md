# 15 — External sources

The immutable source library: what `sources/` is, how a document is registered and
superseded, how it is chunked and searched, and how a record cites an exact passage of one.

Normative for the catalog, the registration/finalization rule, the citation form and the
chunking rules.
Advisory for chunk sizes and ranking weights. Decided in D-031.

---

## 1. Three layers, one responsibility each

```text
┌──────────────────────────────────────────────────────────────┐
│ IMMUTABLE SOURCE LIBRARY        sources/external/*.md        │
│ Exact outside advice, reports, audits.                       │
│ Non-authoritative, content-hashed while registered.          │
└───────────────────────────┬──────────────────────────────────┘
                            │ deterministic chunking
┌───────────────────────────▼──────────────────────────────────┐
│ DERIVED SOURCE INDEX            .akr/cache/sources.sqlite     │
│ Heading paths, semantic chunks, symbols, BM25, byte ranges.  │
│ Rebuildable, non-authoritative.                              │
└───────────────────────────┬──────────────────────────────────┘
                            │ citations: document + byte range
┌───────────────────────────▼──────────────────────────────────┐
│ AKR LEDGER                      .akr/records/**              │
│ Accepted decisions, requirements, work, evidence.            │
│ The project's interpretation and execution state.            │
└──────────────────────────────────────────────────────────────┘
```

The library says *this is what the advisor said*. The ledger says *this is what the project
believes and intends to do about it*. The index says *this is where the relevant material
is*. Combining those three responsibilities is what produced the failure D-031 records.

## 2. Registration creates no records

```bash
akr source add advice/jp2lam-audit.md --id jp2lam-audit-2026-08-05 \
  --title "jp2lam decoder performance audit" --scope "codecs/jp2lam/**"
```

reads the exact bytes, computes a SHA-256, copies the file to
`sources/external/<id>--<short-hash>.md`, and adds an entry to `sources/catalog.json`.

It proposes nothing. An outside report is source material until the project decides to do
something with it, and a workflow that turned every heading into a `proposed` work record
would fill `ACTIVE-WORK.md` with somebody else's opinions.

`akr import` is the one caller that both saves and proposes: it registers the legacy
document exactly as above (origin `internal-reference`, reused by hash on re-import)
and then drafts its claims with the saved copy's id in every `source` block
(`docs/12-migration.md` §3).

`akr source list`, `akr source get <id> [--whole|--lines a:b|--section "heading"]`. With
`--lines`, the output also carries the exact citation locator for that range — see §7.

## 3. Immutability has teeth

`akr source verify` — and `akr check`, which runs it — recomputes every registered hash and
reports `AKR-S021` on a mismatch, exiting 1. A folder plus a warning is not enough: someone
will eventually edit a file by accident, and a source that quietly changed is worse than no
source at all, because every citation into it is now a citation into something else.

A correction is a new version:

```bash
akr source supersede jp2lam-audit-2026-08-05 advice/revised.md --id jp2lam-audit-2026-08-12
```

The older document stays registered and retrievable. Searches exclude superseded versions
unless `--all-versions` is given.

## 4. Chunking rules

Deterministic, dependency-free, and a pure function of (bytes, parser version). Chunking
errors can only harm search quality; they can never change project semantics.

1. **Fences are recognised before headings.** A `#` inside a code block is content. A
   line-oriented heading parser reads a shell comment as structure, which is exactly the
   bug this ordering exists to prevent.
2. Headings establish a section path and are never chunks themselves.
3. Paragraphs, list groups, block quotes and tables are semantic blocks.
4. A fenced code block is never split. A table is never split.
5. Consecutive blocks under one heading pack to roughly 450–700 estimated tokens.
6. No chunk crosses a heading boundary.
7. No overlap. `akr source get --chunk <id> --neighbors 1` is how a caller widens.
8. The parser version is stored with every chunk and mixed into every chunk id.

Prose is normalised for ranking: soft wraps are joined, so two documents that differ only
in where their prose wraps rank identically. Source formatting must not decide search
results.

Technical identifiers get expanded into their searchable variants —
`DecodeRequest::default()` also indexes as `DecodeRequest`, `default` and
`decode request default` — because the query is as likely to be the words as the call.

## 5. Search

```bash
akr source search "nonzero tile origins"
akr source search --literal "DecodeRequest::default()"
akr source search --fts 'origins NEAR/5 dwt'
```

The default mode **escapes punctuation into ordinary terms**. `akr search` takes raw FTS5,
which is a trap for an agent: `DecodeRequest::default()` is a parse error, not a query.
`--literal` narrows and then verifies an exact substring against the stored bytes, and
`--fts` is there for anyone who wants the operators.

Every result says what it is:

```text
4.82  source:jp2lam-audit-2026-08-05
      external · non-authoritative
      6. P1: nonzero tile origins unnecessarily disable optimized DWT
      lines 283-316  chunk c_9f2a41c0d3b8e517
```

The words **non-authoritative** are not decoration. They are the difference between an
agent citing a report as advice and citing it as the plan of record.

Ranking weights heading paths and symbols above ordinary prose: a query naming a section or
an identifier is asking for that section, and undifferentiated BM25 buries it under a
paragraph that happens to repeat the words.

## 6. Two cache generations

`.akr/cache/index.sqlite` is stamped with the ledger's `source_graph_hash`;
`.akr/cache/sources.sqlite` is stamped with the corpus hash and the parser version. They
move independently, so a record write does not rechunk the corpus and a registration does
not re-resolve the ledger.

`akr source search` and `akr source get --chunk` bring the chunk index up to date before
reading it, the way `akr search` has always done for the record index, so a document is
searchable the moment it is registered rather than after the next build. `--no-rebuild` is
still the opt-out and still refuses rather than serving a stale answer.

Registered bytes are immutable while present, which makes source sync trivially incremental.
Registration is not permanent: an advisor document may be finalized into retained cited
fragments or metadata-only lineage, while its catalog identity and full-document hash remain.

## 7. Citations

A record reaches the library through its `source` block:

```akr
source {
    kind external
    document "jp2lam-audit-2026-08-05"
    start_byte 10482
    end_byte 12391
    start_line 203
    end_line 233
    excerpt_hash "sha256:..."
}
```

The byte range is the machine locator; the line range is for people and rendered citations.
The four range slots are all-or-nothing **in the record**: a half-written range resolves to
a passage nobody chose.

That is a rule about what is stored, not a demand on the author, who reads a document by
line. Both write surfaces close the gap rather than making a caller count bytes: `akr
source get <id> --lines a:b` reports the locator for the lines it just served, and
`knowledge.propose` / `knowledge.revise` accept `document` with `start_line` and `end_line`
alone and read the byte offsets off the registered bytes. A located range covers whole
lines, includes the newline ending the last, and carries the `excerpt_hash` of exactly the
bytes it selected. Only a `full` document can be located this way — a finalized one no
longer has the text to scan, so it must be cited by bytes from a retained range.

**A citation names a document and a byte range, never a chunk id.** Chunk boundaries belong
to a rebuildable index and are allowed to move when the scanner improves; provenance is not.

`akr check` resolves every citation against the full document or retained fragments and reports
`AKR-S022` when required source content is unavailable, the range is out of bounds or off a
character boundary, the excerpt hash disagrees, or the line range describes a different
passage than the byte range.

Because the excerpt can be rendered from the source, it no longer has to be copied into
every record.

## 8. Retrieval never authorises

`docs/09-context-assembly.md` §1 holds here unchanged, and matters more, because these bytes
come from outside the project.

* `knowledge.context` includes a `SOURCE REFERENCES` section listing the passages that
  records actually cite. It does not include unrelated source-search hits.
* `knowledge.start` may show relevant external sources, under a separately labelled
  `EXTERNAL REFERENCE MATERIAL — NON-AUTHORITATIVE` heading.
* A source result enters the plan only through an explicit record and the normal lifecycle.

## 9. Sparse adoption is the default

Most reports contain explanatory material, benchmarks, examples, alternative
implementations, caveats and restatements. Requiring a disposition for every unit produces
administrative work without improving planning.

Create a record when the project *does* something with the material — adopts it, rejects
it, defers it, or decides to track the review. Everything else stays readable in the report,
which is where it was always going to be read.

The correction D-031 records, in one line:

> Review every source item if you like, but do not turn every source item into knowledge.

---

Next: [`16-change-protocol.md`](16-change-protocol.md) for the other bridge between AKR and
the outside world, or [`12-migration.md`](12-migration.md) for the legacy-migration
workflow that `akr source add` deliberately is not.

## Source finalization

Registered bytes are immutable while present, but registration is not permanent. Records
must remain semantically self-contained: hiding provenance must not make their intent or
acceptance conditions unintelligible. A source citation explains origin; it does not make a
record's meaning live in the source document.

Use `akr source status <id>` and `akr source dependents <id>` before cleanup. Finalization
retains exact cited ranges in content-addressed blobs under `.akr/source-fragments/`, without
rewriting or duplicating records:

```text
akr source finalize audit --retain cited --context block --remove-file
```

`block` retains the enclosing indexed semantic block; `--context exact` retains only the
cited bytes. `--retain metadata` is allowed only when no record has an exact citation. A
`cited-only` source is excluded from ordinary source search, but `akr check` and explicit
`akr source get <id>` continue to resolve retained provenance. Finalization never changes a
work state, satisfies evidence, or implies that advice was implemented.

The source states are:

| State | Full document | Cited bytes | Exact citations |
| --- | --- | --- | --- |
| `full` | retained | optional | resolve from the document |
| `cited-only` | removed | retained | resolve from fragments |
| `metadata-only` | removed | none | only lineage references are valid |

This is source finalization, not work completion. The catalog tombstone preserves the source
identity and full-document hash even after the Markdown leaves the active source tree.
