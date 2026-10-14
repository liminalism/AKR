# `akr.lock` — Format Specification

The lock file makes a build reproducible and makes a floating reference's target change
visible in review. This document specifies its format, field by field. Its purpose and
the review procedure around it are in `docs/04-references-and-versioning.md` §8.

---

## 1. Grammar

`akr.lock` is written in the AKR grammar with the header `akr-lock 0.1` (D-014). It uses
the same lexer, parser, and formatter as record files; only the set of top-level items
differs. There is no second syntax to learn, no second parser to maintain, and no second
canonicalisation question to answer.

```
akr-lock 0.1
project save-your-skin

build { ... }
source "<path>" { ... }
resolution @<referring-revision> { ... }
seal @<revision> { ... }
```

It lives at `.akr/akr.lock` and is **committed**.

Digests are written as quoted strings in the form `"sha256:" + 64 lowercase hex`, not as
bare literals: the grammar has no digest literal and does not need one.

---

## 2. Top-level items

### 2.1 `build` — exactly one, first

```
build {
    tool "akr 0.1.0"
    grammar "0.1"
    vocabulary "0.1"
    commit git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64
    source_graph "sha256:18cd53738ba80ab55997a6cf5087debd5b53ebf7845d2eea7f56b7d825c88bd7"
    built_at 2026-08-03T09:14:00Z
}
```

| Slot | Type | Meaning |
| --- | --- | --- |
| `tool` | string | Tool name and version that wrote this lock. |
| `grammar` | string | Grammar version of the sources. |
| `vocabulary` | string | `vocabulary_version` from `spec/tables/vocabulary.json`. |
| `commit` | commit | The commit the build resolved against. `HEAD` at build time. |
| `source_graph` | string | Source-graph hash (§3.2). |
| `built_at` | timestamp | UTC, informational only — never an input to any check. |

`built_at` is the one field that changes on every build regardless of content. It is kept
because "when was this last built" is a question people ask, and excluded from every
comparison so that it never causes a spurious `AKR-R052`.

### 2.2 `source` — one per `.akr` source file

```
source ".akr/records/sys/policies.akr" {
    hash "sha256:ee2557459c2c85d6e29828e260202cf7b6616f11b9624d8b86245a0a2075daec"
    records 3
}
```

| Slot | Type | Meaning |
| --- | --- | --- |
| head | string | Path, repo-root-relative, forward slashes. |
| `hash` | string | Source file hash — SHA-256 over the file's raw bytes (§3.1). |
| `records` | integer | Record count. Informational; makes a truncated file obvious at a glance. |

`records` is the one integer-typed slot in the whole design set, and it lives here rather
than in the record vocabulary — see `docs/03-syntax.md` §3.8 on why the literal exists.

### 2.3 `resolution` — one per current-head reference resolved

```
resolution @sys.work.m3-plan/2 {
    slot implements
    to @sys.policy.tandem-work/1
    hash "sha256:83b7cd31ff8d90e295e68c962aae2c4eca811ca5042e19c64f730ccb758dd914"
}
```

| Slot | Type | Meaning |
| --- | --- | --- |
| head | reference | The referring revision, always pinned. |
| `slot` | name | The slot the reference appeared in. |
| `to` | reference | What `@key` resolved to, always pinned. |
| `hash` | string | Content hash of the resolved revision (§3.3). |

One entry per distinct (referring revision, slot, target key). Pinned references are
**not** recorded: a pinned reference cannot change what it points at, so locking it would
be noise. Anchors are not recorded either — the anchor is part of the referring record's
text, and the resolution being locked is the revision.

The `hash` field is what makes a repointing visible even when the revision number is
unchanged, which happens when a `proposed` head is edited in place.

### 2.4 `seal` — one per sealed revision

```
seal @sys.policy.tandem-work/1 {
    state active
    hash "sha256:83b7cd31ff8d90e295e68c962aae2c4eca811ca5042e19c64f730ccb758dd914"
}
```

| Slot | Type | Meaning |
| --- | --- | --- |
| head | reference | The sealed revision, always pinned. |
| `state` | enum | Its state when sealed. Informational; the seal applies to any non-`proposed` state. |
| `hash` | string | Content hash of the revision (§3.3). |

Every revision in a state other than `proposed` has exactly one `seal` entry (D-015). A
missing entry for a sealed revision, or an entry for a revision that no longer exists, is
`AKR-R052`.

---

## 3. Hash definitions

All three are SHA-256, rendered as 64 lowercase hex digits with a `sha256:` prefix.

### 3.1 Source file hash

SHA-256 over the file's **raw bytes on disk** (see `docs/06-compiler-pipeline.md` §9:
this hash answers "are the inputs on disk the same inputs?", a question about the
filesystem, not about meaning). A file that is not canonical fails `akr fmt --check`
(`AKR-F001`) before the lock is ever consulted, so in a passing build the raw bytes
and the canonical form are the same bytes anyway.

### 3.2 Source-graph hash

SHA-256 over the concatenation, for every source file sorted by path:

```
<path> NUL <file-hash> LF
```

Paths are repo-root-relative with forward slashes, sorted bytewise. This single value
answers "are the sources the same as last time" without comparing every file, and it is
what generated views carry in their banner (D-025) so a reader can tell whether a view
matches the ledger in front of them.

### 3.3 Revision content hash

SHA-256 over the **canonically formatted text of that record alone**: from the `record`
keyword through its closing brace, inclusive, with LF line endings, no leading
indentation, and a single trailing newline.

Two exclusions, both deliberate:

- **Comment trivia is excluded.** Comments are commentary, not content. Adding a
  clarifying comment to a sealed record must not trip `AKR-R051`, or people will stop
  writing comments, which is the opposite of what the format wants.
- **Surrounding file content is excluded.** Moving a record between files, or reordering
  it, does not change its hash. Identity is the key, never the file (D-018).

Because the hash is over the *canonical* form, reformatting cannot change it. If a
reformat does change a seal hash, the file was not canonical before, and the mismatch is
real information rather than a false alarm.

---

## 4. Ordering

The lock is fully sorted, so that two builds of the same sources produce byte-identical
files and a diff shows only what changed:

1. `build` first, exactly once.
2. `source` entries, sorted by path bytewise.
3. `resolution` entries, sorted by referring key, then referring revision, then slot
   name, then target key.
4. `seal` entries, sorted by key, then revision ascending.

Within each block, slots follow the order given in §2. No blank lines inside a block;
exactly one between top-level items.

---

## 5. Worked example

An excerpt from `examples/save-your-skin/.akr/akr.lock`, showing one of each item type:

```
akr-lock 0.1
project save-your-skin

build {
    tool "akr 0.1.0"
    grammar "0.1"
    vocabulary "0.1"
    commit git:e806b3f54a2d7091c5e13b8a26f490dc7b135e64
    source_graph "sha256:5159d2714f688627a85906b161def17b0358155fbfc2a3fc00b681d29a305f55"
    built_at 2026-08-03T09:14:00Z
}

source ".akr/project.akr" {
    hash "sha256:7af283fc0ca469613119b9e146ce111f85f311821939f09e9c44b8de1deaebb9"
    records 0
}

source ".akr/records/sys/policies.akr" {
    hash "sha256:335dfe637e083fbccdb4eaf1b4ea51c8d7dc8136f96acc048fd3f41846b37c99"
    records 2
}

resolution @sys.assessment.projection-gaps/1 {
    slot supported_by
    to @sim.obs.projection-gaps/1
    hash "sha256:bd65a0916647a85e0e30d58d15c8e6a531392d22141ffc9021c80b34dade1a64"
}

resolution @sys.work.m3-plan/2 {
    slot implements
    to @sys.policy.tandem-work/1
    hash "sha256:2663167b4b66a4ed4b0bf7c20f5edd7f5023234671435307aaf302e86ad8f914"
}

seal @lege.decision.renderer-boundary/1 {
    state superseded
    hash "sha256:89c817d141949f9adc59c6f417206d8791c9f10b1572611800897af5ea912258"
}

seal @sys.policy.tandem-work/1 {
    state active
    hash "sha256:2663167b4b66a4ed4b0bf7c20f5edd7f5023234671435307aaf302e86ad8f914"
}
```

**These digests are real.** The excerpt above is copied from
`examples/save-your-skin/.akr/akr.lock`, which is generated output: every value in it is
computed by the implementation from the sources beside it. The source hashes are SHA-256
over raw disk bytes (§3.1), the seal hashes over canonical record text (§3.3), and
`source_graph` over the sorted pairs (§3.2) — which is why the same `source_graph` value
appears in the banner of all six generated views (D-025). A snapshot test regenerates the
file and fails on any drift, so the excerpt and the file cannot diverge.

Note that `@sys.policy.tandem-work/1` shows the same digest in its `seal` entry and in the
`resolution` that points at it. That is not a coincidence and not an error: both record the
content hash of the same revision.

The shape examples in §2 above are **not** real digests — they are placeholders chosen to
be readable, and no build produces them.

---

## 6. Regeneration and verification

| Command | Behaviour |
| --- | --- |
| `akr build` | Rewrites the lock from the sources. Always. |
| `akr lock` | Rewrites the lock without emitting views. |
| `akr lock --check` | Verifies without writing. CI gate. Non-zero on any mismatch. |
| `akr lock --reseal` | Recomputes seal hashes for revisions whose canonical form changed under a grammar upgrade. Produces a large diff by design. |
| `akr check` | Verifies seals (`AKR-R051`) and lock currency (`AKR-R052`) as part of resolve. |

Verification compares everything except `build.built_at`. A lock whose `source_graph` no
longer matches the sources is stale, not wrong: the fix is `akr build`, and the
diagnostic says so.

Exact comparison (`Lock::verify`) includes commit provenance. Currency checks
(`akr lock --check`, `akr build --check`) additionally accept a recorded absent
commit becoming a known commit when every other compared field agrees. This is
the ordinary commit of an exact ledger built while dirty. The lock retains its
original absent provenance; the check neither rewrites it nor invents a commit.
All source, graph, tool, resolution and seal differences remain mismatches, as
does losing or changing a recorded commit.

A missing lock file is not an error on first build — `akr build` creates it. It **is** an
error for `akr check` in a repository that has one committed elsewhere in history, which
is a sign of an accidental deletion rather than a fresh start.

---

## 7. See also

- `docs/04-references-and-versioning.md` §8 — purpose, review reading, merge procedure.
- `docs/05-validation-rules.md` — V-024 (`AKR-R051`), and `AKR-R052`.
- `docs/06-compiler-pipeline.md` — where in the build the lock is read and written.
