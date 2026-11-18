# 19 — AKR Ledger Usage & Reasoning Audit (August–November 2026)

**Status:** Acted on in AKR 0.7.0 by D-046 (retired kinds), D-047 (the `trace` slot), D-048
(a check records its own pass) and D-049 (one agent brief, hooks by default). One
recommendation was not taken as written: a `why` is never mandatory (§4.2 proposed
"required or strongly prompted"); it is steered by the brief and by advisory notes.
See `docs/20-upgrading-to-0.7.md`.

**Scope:** Empirical three-month audit (`2026-08-18` to `2026-11-18`) of AKR ledger records and Git commit histories across `SYS` (`D:\projects\SYS`), `Lege-ecosystem` (`D:\projects\Lege-ecosystem`), and `nail-salon` (`D:\projects\nail-salon` + pre-split `D:\projects\Kitchen-Concept`).

---

## Executive Summary

An audit of **4,467 unique AKR records** (**5,698 total revisions**) and **1,181 Git commits** across the three target projects over the last three months confirms the core suspicion:

1. **Agents overwhelmingly use `work` and `evidence` records to restate *what* a commit or test run did in natural language, rather than *why* it was done.**
   - In `SYS`, **72.9%** of completed `work` records contain no mention of rejected alternatives, trade-offs, or why the design was chosen.
   - Across all three projects, nearly half of all `evidence` records (**47.7%** in `SYS`, **49.3%** in `Lege-ecosystem`, and **~90%+** of sampled test evidence in `Kitchen-Concept`) copy-paste the exact same test-output summary into both `title` and `summary`.
   - Even when agents write rich prose in `decision` or `work` records, they frequently copy-paste the commit message body verbatim into the `.akr` record (or vice versa) in a separate `chore(akr):` catch-up commit minutes later.
2. **Contrary to perception, `evidence`—not `work`—is the dominant category by volume (`46.5%` in `SYS`, `38.7%` in `Lege-ecosystem`, `53.8%` in pre-split `nail-salon`).**
   - `work` *feels* dominant in `SYS` (`572` unique keys / `778` revisions, #2 at `17.7%`) because `akr start`, `akr context`, `ACTIVE-WORK.md`, and `OUTSTANDING-WORK.md` foreground `work` records (`184 proposed` + `41 active` in `SYS`), whereas `1,499` `evidence` records sit out of sight behind `verified_by` edges.
3. **`work` records are almost never attached to commits via AKR trailers as intended (`<6%` across all three projects).**
   - **`SYS`**: Only **23 of 572 (`4.0%`)** `work` keys (and only **9 of 166 (`5.4%`)** `completed` `work` keys) appear in `AKR-Work` commit trailers. Trailer usage ceased completely after `2026-10-15` (`0 of 63` commits in November). Over half (**57.3%**, `134 / 234`) of commits touching `.akr/` are standalone ledger-only commits with zero code files.
   - **`Lege-ecosystem`**: **97.4% (`301 / 309`)** of commits in the last 3 months carry no AKR trailers at all; only **11 of 212 (`5.2%`)** `work` keys have ever been linked by a commit trailer, and **54.7% (`29 / 53`)** of `.akr/` commits are retroactive ledger-only syncs.
   - **`nail-salon`**: **0 of 53 (`0.0%`)** pre-split salon `work` records were linked via `AKR-Work` trailers, and post-split (`2026-11-16` to `2026-11-18`) the repository contains **0 `work` records**, **0 `evidence` records**, and **0 commit trailers** (`46.4%` of its 28 records were committed in zero-code `chore(akr)` catch-up commits).
4. **Genuine "why did we do this back then" reasoning *does* exist in the ledgers—but it lives in `observation` records and in the `context`/`consequences` slots of `decision` records, not in `work` records.**
5. **Architectural recommendation:**
   - **Do not add a standalone top-level `reasoning` record kind (`record ... : reasoning`).** A separate record kind with a 20-word cap would require 8–10 lines of record envelope boilerplate per 20-word thought, detach the reasoning from the work/commit it explains, and multiply record bloat.
   - **Instead, add an OptMem-style bounded `why` / `step` reasoning trail directly onto `work` records and `akr change begin` transactions (≤ 20–25 words per entry, linked to the commit), and prune/consolidate underused and bloated categories (`term`, `assessment`, `track`, and standalone test-pass `evidence`).**

---

## 1. Cross-Project Quantitative Census (2026-08-18 to 2026-11-18)

### 1.1 Record Category Distribution Across the Three Projects

> **Note on `nail-salon`:** `D:\projects\nail-salon` was split out of `D:\projects\Kitchen-Concept` via `git filter-repo` on `2026-11-16` (`1ec63b3`), and a fresh `.akr/` ledger was initialized in `nail-salon` (`cef46a5`) while leaving the pre-split history in `Kitchen-Concept`. Both the post-split `nail-salon` ledger and the 368 pre-split salon records in `Kitchen-Concept` are shown below.

| Kind | Class | `SYS` Unique (Revs) | `Lege-ecosystem` Unique (Revs) | `nail-salon` Post-Split | `nail-salon` Pre-Split (`Kitchen`) | Combined Unique Keys | Share of Combined |
|---|---|---:|---:|---:|---:|---:|---:|
| **`evidence`** | empirical | **1,499** (1,599) | **328** (365) | 0 | **198** | **2,025** | **45.3%** |
| **`work`** | planning | **572** (778) | **212** (246) | 0 | 53 | **837** | **18.7%** |
| **`papercut`** | empirical | 175 (274) | 201 (**381**) | 1 | 48 | **425** | **9.5%** |
| **`observation`** | empirical | 356 (744) | 49 (73) | 5 (5) | 4 | **414** | **9.3%** |
| **`decision`** | normative | 231 (269) | 30 (32) | **18** (18) | 62 | **341** | **7.6%** |
| `policy` | normative | 108 (130) | 12 (12) | 4 (4) | 0 | **124** | 2.8% |
| `requirement` | normative | 92 (102) | 6 (6) | 0 | 0 | **98** | 2.2% |
| `assessment` | empirical | 52 (62) | 7 (12) | 0 | 1 | **60** | 1.3% |
| `question` | inquiry | 42 (83) | 1 (1) | 0 | 1 | **44** | 1.0% |
| `milestone` | planning | 39 (72) | 2 (3) | 0 | 0 | **41** | 0.9% |
| `constraint` | normative | 34 (50) | 0 (0) | 0 | 0 | **34** | 0.8% |
| `track` | planning | 11 (15) | 0 (0) | 0 | 1 | **12** | 0.3% |
| `term` | normative | 12 (23) | 0 (0) | 0 | 0 | **12** | 0.3% |
| **Total** | | **3,223** (4,201) | **848** (1,131) | **28** (28) | **368** | **4,467** | **100.0%** |

### 1.2 Git Commit vs. `work` Record Attachment Census (Last 3 Months)

| Metric (2026-08-18 to 2026-11-18) | `SYS` | `Lege-ecosystem` | `nail-salon` (incl. pre-split) |
|---|---:|---:|---:|
| Total Git commits in 3-month window | **712** | **309** | **160** (109 pre-split, 51 post-split) |
| Commits with `AKR-Change` trailer | **91 (12.8%)** | **8 (2.6%)** | **3 (1.9%)** (0 post-split) |
| Commits with `AKR-Primary` / `AKR-Kind` / `AKR-Untracked` | **0 (0.0%)** | **0 (0.0%)** | **0 (0.0%)** |
| Commits with `AKR-Work` trailer | **87 (12.2%)** | **8 (2.6%)** | **3 (1.9%)** (pointed to non-`work` keys!) |
| Unique `work` records referenced by `AKR-Work` trailer | **23 / 572 (4.0%)** | **11 / 212 (5.2%)** | **0 / 53 (0.0%)** |
| `completed` `work` records referenced by `AKR-Work` trailer | **9 / 166 (5.4%)** | **7 / 66 (10.6%)** | **0 / 34 (0.0%)** |
| Commits touching `.akr/` with **0 code files** (retroactive/catch-up) | **134 / 234 (57.3%)** | **29 / 53 (54.7%)** | **13 / 28 records (46.4%)** post-split |
| `work` records currently in `abandoned` state (mostly bulk imports) | **161 / 572 (28.1%)** | **129 / 212 (60.8%)** | **0** |

---

## 2. Project-by-Project Findings

### 2.1 `SYS` (`D:\projects\SYS`)

1. **Why `work` appears dominant even though `evidence` is 2.6× larger:**
   - `SYS` grew by **+2,578 unique keys** in the last 3 months (`645` on `2026-08-17` → `3,223` on `2026-11-18`).
   - More than half of that growth (**+1,354 unique keys / 52.5%**) was `evidence`, whereas `work` grew by `+434` (`572` total: `184 proposed`, `166 completed`, `161 abandoned`, `41 active`).
   - Because `akr start` and `ACTIVE-WORK.md` / `OUTSTANDING-WORK.md` surface the `225` live `work` items (`184 proposed` + `41 active`) on every session boot, `work` dominates the *human/agent reading surface* while `evidence` dominates the *storage volume*.
2. **How `work` is used in `SYS`:**
   - **Pre-implementation checklists and post-hoc changelogs (`LANDED` notes):** Agents routinely treat `work.intent` as a feature specification and `work.note` as a post-commit changelog listing which files/functions landed in which commit hash.
   - **Example 1 — Redundant diff restatement while real reasoning was written to a separate markdown file:**
     - Commit `d103e9b284` (`2026-09-25`) explains *why* colliders were dropped (`"colliders snapped the coat hem on descending stairs: 310 mm/frame^2 against the feet's 106, and changed nothing on the gown"`) and remarks: *"`docs/v2-character-worklog-2026-09-25.md` records every change of both worktrees and the problem-solving path, dead ends included."*
     - Minutes later, ledger-only commit `57fc25b5d9` recorded `@saveyourskin.work.v2-late-coat-chains-and-legs/1` (`state completed`) with `intent`: *"End the late coat's flat plate and give its skirt the gown's treatment: a leg pass moving Hips-bound lower-leg cloth onto its own leg (opt-in per body), COAT chains on the coat skirt found by geometry, and the bake following each chain joint's leg at its rest height."* It has **no `note`** and omits the dead ends entirely.
   - **Example 2 — Post-hoc commit-inventory notes:**
     - Code commit `cd39c68c9a` (`2026-11-08`, Phase 1) lands duty rumor distortion and UI zones. Six minutes later, ledger-only commit `1fdc76563f` updates `@saveyourskin.work.review-plan-phase-zero-and-one-20261108/1` with `note`: *"Phase 1 landed in cd39c68c (2026-11-08 night): the surface check passes... Also in Phase 1 beyond the checks: the four-zone play screen with the drawer (sys_present ui/zones.rs, two sys_ui layout fixes) and the outcome card (game outcome.rs, ui/outcome.rs)."*
   - **Example 3 — Module inventory restatement:**
     - `@saveyourskin.work.courtierwork-p1-shared-spine/1`: `note` reads *"LANDED 2026-07-30. engine/practice.rs owns commissions, manifests, observations, typed drafts, scheduled work, presentations, compensation, professional social records, six-part audits, purse, and professional legacy. engine/reckoner.rs proves the reducer end to end."* Every word of this is directly readable from `engine/practice.rs` module items.
   - **Example 4 — Verbatim copy-paste between commit messages and AKR records:**
     - In `95e4d3f198` (`2026-10-15`), the first two paragraphs of the commit message are word-for-word identical to `@saveyourskin.work.dinner-any-angle-physical-movement-20261015/1`'s `intent`.
     - In `aa6ff49e1d` (`2026-09-16`), the commit message body is word-for-word identical to `@saveyourskin.decision.the-evidence-pointer-is-simulation-plumbing/1`'s `decision` + `context` + `consequences`.
3. **Where `SYS` *does* capture high-value at-the-time reasoning:**
   - **`observation` records (root causes & falsifications):**
     - `@saveyourskin.observation.v2-late-coat-plate-is-hips-bound-leg-cloth-20260925/1` (`57fc25b5d9`): Records the non-obvious root cause and why an obvious fix fails (*"is not the coat. It is lower-leg cloth... Moving leg cloth onto its own leg makes the cloak body worse (stretched 710 -> 836) because its long coat hangs inside the leg tube; that coat must be chained first"*).
     - `@saveyourskin.observation.dunwold-day-local-and-per-beat-redispatch-corrections-20260913/2` (`32726fb6c6`): Explains *why* three performance fixes were allowed to break digest parity (the *"correct for one day, wrong for the second"* bug class where `now - 15` wrapped across day boundaries).
     - `@saveyourskin.observation.seed-ten-dinner-start-screens-door-deadlock-20261107/1` (`04db4f1d22`): Traces the exact 3-mover cycle at the screens passage and why `age >= 10` failed to break it.
   - **`decision` records (`context` and `consequences`):**
     - `@saveyourskin.decision.the-evidence-pointer-is-simulation-plumbing/1` (`aa6ff49e1d`): Explains *why* an outside review's advice to move `position.rs` into `save_your_skin_game` would have created a crate dependency cycle, and *why* a guard test is required (*"this drift is silent: nothing breaks when the enum moves back"*).
     - `@saveyourskin.decision.a-guest-asks-the-door-keeper-for-leave-of-the-lords-chamber-20261111/1`: Captures both the playtest failure trigger (*"a request made as he passed was refused at the tick because he had walked on"*) and deliberate scope boundaries (*"Not built and deliberately left: an attendant, a seal, commissions, an inquiry into whether the dues are owed"*).

---

### 2.2 `Lege-ecosystem` (`D:\projects\Lege-ecosystem`)

1. **Category & Workflow Breakdown:**
   - Out of **848 unique keys** (`1,131` revisions), **`evidence` (`328` unique), `work` (`212` unique), and `papercut` (`201` unique / `381` revisions)** make up **87.4%** of unique keys.
   - **60.8% (`129 / 212`) of all `work` records are `abandoned`**: `136` `work` items were bulk-imported on `2026-08-06` (`9ca5c9fd`, `b87839ca`) from markdown files, and `126` of them had to be mass-abandoned on `2026-09-19` (`1b2ce26246`: *"Mis-imported ledger noise... promoted every markdown heading to its own work record"*).
   - Of the `55` new `work` keys created in the last 3 months, **16** were dumped in a single 799-file mega-commit (`3bdc98d405`, which created revisions `/1` through `/9` of `jbig2-070-quality-and-provenance` in one commit) and **14** were committed in retroactive zero-code ledger commits (`34c0a40720`, `0eb43f379c`).
2. **Redundancy in `work` and `evidence` vs. High Signal in `decision` and `observation`:**
   - **Redundant `work` & `evidence`:**
     - `@lege-ecosystem.work.gui-file-tiles-beside-process-button/1` (`adced943e3`, `2026-11-08`) copies the commit message's widget coordinates almost word-for-word (*"Input tiles sit at the bottom-left and output tiles at the bottom-right of the status card, with the process button centred between them... default 1040x760, minimum 720x660"*).
     - `@lege-ecosystem.work.lege-gpu-tensor-core-gemm/1` (`8f2285d884`, `2026-11-08`) repeats the commit message's WGSL kernel feature list.
     - `@lege-ecosystem.work.context-menu-watch/1` and `@lege-ecosystem.evidence.context-menu-watch-msvc-integration-20261104/1` (`0eb43f379c`) record passing MSVC build/test evidence for a tool (`tools/context-menu-watch/`) that was never even committed to the repository.
   - **Irreplaceable at-the-time reasoning in `decision` & `observation`:**
     - `@lege-ecosystem.decision.binarization-best-model-fast-adaptive/1` (`98dd5cc2dcc3`, `2026-11-14`): Whereas the commit message says only *"Also lands... the Best (tilenet model) and Fast (wgpu adaptive) binarizers replacing the old methods"*, the `decision` record preserves the entire 3-step chain of thought: (1) deleting the 8 wgpu shaders when the neural model looked better visually, (2) running a 120-page / 20-book comparison showing the model won on 27 tinted scans (`-22%` size) but lost on 19 born-digital PDFs, and (3) the owner ruling that born-digital PDFs are *not* why adaptive returns (*"not what a rasterizer is for"*), **bulk throughput is** (`12 s` vs `2.7 s` at 3000 px), restoring the 8 shaders from Git history the same day with an explicit warning not to delete them again.
     - `@lege-ecosystem.decision.layout-model-v2-without-old-model-corrections/1` (`34c0a40720`, `2026-11-10`): Explains *why* four post-processing heuristics were deleted (measured `AP50` drop `0.663 -> 0.486`, swallowing `193/1,034` running headers, turning dark photos into black blobs) and why a line-art vs. photo classifier was rejected (labels do not exist in training data; `+13%` file size accepted as the trade-off).
     - `@lege-ecosystem.decision.jbig2-is-the-default-pdf-text-format-again/1` (`b3391a2bf1`, `2026-09-06`): Explains why an earlier 20-page benchmark (`183 KB` vs `378 KB`) had misled them into making TrueTyping the default, and how an 886-page book exposed TrueTyping's 65,533-glyph-ID bank limit (`1,000 IDs/page` from position/compound variants → 22 fonts, `45.3 MB`, `120 s` vs JBIG2 `21.2 MB`, `33 s`).

---

### 2.3 `nail-salon` (`D:\projects\nail-salon` & Pre-Split `D:\projects\Kitchen-Concept`)

1. **Two Distinct Eras (Pre-Split vs. Post-Split):**
   - **Pre-split (`Kitchen-Concept`, `2026-08-18` to `2026-11-16`):** `368` salon records (`198 evidence`, `62 decision`, `53 work`, `48 papercut`, `4 observation`). **78%** of `work` records were either squashed into multi-feature mega-commits (`66%`, e.g., `03d16ae8` adding `+14` salon `work` records across 126 files) or backfilled in zero-code `chore(akr):` catch-up commits (`12%`, e.g., `aed06ea8`).
   - **Post-split (`nail-salon`, `2026-11-16` to `2026-11-18`):** `28` records total (`18 decision`, `5 observation`, `4 policy`, `1 papercut`, **`0 work`**, **`0 evidence`**). All `22` normative records (`18 decision` + `4 policy`) remain in `proposed` state (`0 active`). Nearly half (`13 / 28 = 46.4%`) were added in zero-code `chore(akr)` catch-up commits (`6091b60` added 8 records at once after the code commits).
2. **Prose Duplication Between Commits and Decisions:**
   - In `nail-salon`, the commit messages themselves (`5a62bba4`, `ea0c6ba5`, `310e8c3b`, `9da97924`) are written in rich, narrative prose—and then the agent copies the same sentences into `decisions.akr` in a subsequent `chore(akr)` commit:
     - `@salon.decision.past-sales-entered-on-the-mac/1` vs. `5a62bba4`: Both the `decision` slot and the commit body read nearly word-for-word: *"the day stays between sales with arrows to step it, a service is found by typing and brings the menu's price for that day... The command is the existing ImportOpeningSale, unchanged... The tablet's Enter past sales panel, its form and its command builder are removed."* Only one sentence in `context` adds non-commit reasoning (*"A spreadsheet import with a preview was offered and declined in favour of manual entry"*).
     - `@salon.line.worker-deployed-2026-11-18/1..3` (`ce18c5d6`, `0aa0c966`): Three revisions of the exact same 1,200-character deployment observation, superseded twice in one day solely to advance `observed_at` (`5a62bba4` → `ca80a297`) after `watches` flagged it stale on a merge.

---

## 3. How `OptMem` Worked — and What AKR Lost When `OptMem` Was Retired

Traces in `LegeOS` (`f8b45b7:AGENTS.md`, `addc6df4`, `1d813747`) and `SYS` (`observations.akr`, `work.akr`, `docs/handoff/*/ORCHESTRATION.md`) show how `OptMem` (`~/.optmem/memo`) operated before it was phased out in late August / October 2026:

1. **The `OptMem` Mechanics:**
   - `memo wake`: Forced the agent at session start to read a tree of compressed historical summaries plus recent raw notes.
   - `memo note "<1 line, max 280 bytes>"`: Required the agent to log a single-line, high-density note whenever it learned a non-redundant fact or completed meaningful work.
   - `memo nap`: Periodically forced hierarchical compression of raw notes (e.g., notes `32-35` or `180-181`) into a single ≤280-byte summary node (`memo zoom <lo>-<hi>` expanded it).
2. **Why `OptMem` Was Deprecated (Rightly):**
   - **Out-of-tree state:** `~/.optmem/memory` lived in the user's home directory outside Git, so it went out of sync across OS partitions (`D:\` vs `E:\` in `SYS`, `@saveyourskin.observation.livingcourt-planning-survived-only-in-session-transcripts/1`) and across machines (`@legeos.papercut.on-the-installed-templaros-agents-md-mandates/1`).
   - **Multi-agent incompatibility:** Subagents had to be banned from touching it (`Subagents never run ~/.optmem/memo` in `SYS`'s `ORCHESTRATION.md`) because concurrent uncoordinated writes/compressions corrupted the linear note numbering.
   - **Hard-cap retry friction:** Seven separate papercuts in `LegeOS` (`@legeos.papercut.optmem-nap-rejected-a-dense-final-compression/1`, etc.) logged agents failing the 280-byte UTF-8 check at 281–287 bytes and burning extra turns counting bytes.
3. **What `OptMem` Did Right That AKR Currently Misses:**
   - Because `OptMem` gave an agent **only one line (~20–35 words / 280 bytes)**, an agent *could not* waste space listing struct fields or copying `cargo test` output. It had to write compressed **at-the-time reasoning and operational boundaries**: *"why X broke, why Y was rejected, what constraint holds, and what commit landed it."*
   - When AKR replaced `OptMem`, agents were given multi-line `"""..."""` prose blocks (`intent`, `note`, `summary`, `statement`) with **no brevity constraint and no prompt asking "why / what was rejected"**. Without a length bound or a dedicated `why` prompt, LLMs default to their lowest-effort habit: summarizing the diff in 150 words of natural language.

---

## 4. Assessment: Do We Need a New `reasoning` Category, and Which Categories Should Be Removed?

### 4.1 Should AKR Add a New Top-Level `reasoning` Category?

**No as a standalone record kind (`record ... : reasoning`); YES as a bounded ≤20-word `why` / reasoning-step primitive attached to `work` records and commit transactions.**

Why a standalone top-level `reasoning` record kind would backfire:
1. **Envelope overhead vs. payload size:** In AKR's grammar (`docs/02-data-model.md`), every standalone record requires ~8–10 lines of envelope (`record ns.reasoning.slug/1 : reasoning`, `title`, `state`, `observed_at`, `author`, `created_at`, plus a seal block in `akr.lock`). Creating a 10-line record + lock seal to hold a 20-word sentence is a 5:1 boilerplate-to-signal ratio.
2. **Detachment from work and commits:** Looking at historical `work` entries to answer *"why did we do this thing back then"* requires the reasoning to live **inside or directly on the `work` record and commit trailer**, not in a separate `.akr/records/<ns>/reasoning.akr` file that a reader has to cross-reference.
3. **D-032 invariant ("no `commit` record kind"):** A standalone 20-word `reasoning` record per commit would effectively become a shadow `commit` record kind, multiplying record counts even faster than `evidence` did.

### 4.2 What Should Be Added Instead: The Bounded `why` Slot & Reasoning Trail

To make historical `work` records and commits answer *"why did we do this thing back then"* without duplicating the code diff:

1. **Add a required (or strongly prompted) bounded `why` slot (max 20–25 words) on `akr change begin` and `work` completion/revision:**
   - Currently, `akr change begin` takes `--kind`, `--summary "<imperative>"`, and `--primary <work-key>`. `--summary` asks *what* the commit does, so agents write *what* the commit does.
   - Adding `--why "<≤20 words: why this approach / what forced it>"` (emitted as an `AKR-Why:` commit trailer and appended as a timestamped/commit-anchored reasoning line on the primary `work` record) forces the agent at commit time to record the **non-obvious reason** in ≤20 words.
   - Example of how this transforms a real `SYS` commit (`d103e9b284`):
     - *Current `work.intent` (48 words, redundant with diff):* `"End the late coat's flat plate and give its skirt the gown's treatment: a leg pass moving Hips-bound lower-leg cloth onto its own leg (opt-in per body), COAT chains on the coat skirt found by geometry, and the bake following each chain joint's leg at its rest height."`
     - *20-word `why` entry (non-redundant reasoning):* `"Hips-bound leg cloth caused coat plate; dropped colliders because stairs snapped hem at 310 mm/frame²; cloak needs coat chained first."` (20 words).
2. **Support a lightweight chain-of-thought `trace` / `step` list inside `work` records (replacing unstructured `LANDED` prose in `note`):**
   - As an agent progresses through hypotheses and dead ends under an active `work` item, each step is a single ≤20-word line inside the `work` record:
     ```akr
     trace [
         "rejected leg-cloth-only pass: worsened cloak stretch (710->836) because unchained coat hangs inside leg tube"
         "dropped capsule colliders: snapped coat hem on stairs (310 vs 106 mm/frame²), zero gown benefit"
         "d103e9b2: sampled chain joint leg at rest height + geometry COAT chains"
     ]
     ```
   - This gives full **chain-of-thought auditing** (including rejected dead ends!) right inside the `work` record, at ~15–20 words per step, with zero extra record-header or `akr.lock` seal bloat.
3. **Soft lint (`AKR-W...` warning, not hard byte-count failure) on `why` / `trace` entries:**
   - Rather than `OptMem`'s hard 280-byte rejection (which caused agents to waste turns when a line was 281 bytes), enforce a **soft warning at >25 words** (and hard cap only at e.g. 40 words), plus a lint warning when `work.intent` or `decision.decision` contains file paths/rust module paths (`crates/...`, `::`, `.rs`) that belong in `scope` or the Git diff.

### 4.3 At What Granularity Is Reasoning Needed? (The 3-Tier Model)

A single 20-word limit works for commit-level and iteration-level steps, but not every level of software engineering has the same granularity:

| Tier | Granularity | Where It Belongs | Word Budget | What It Must Answer |
|---|---|---|---|---|
| **Tier 1: Local Code** | Single function, loop, struct field, or borrow-checker workaround | **In-code `//` comments** (tracked by `git blame`) | 1–2 sentences inline | Why this line is written this way (invariants, safety, local edge case). **Never create an AKR record for Tier 1.** |
| **Tier 2: Work Step & Commit** | A commit, bug fix, parameter choice, or dead-end iteration within a task | **`work` record `why` / `trace` line + `AKR-Why` commit trailer** | **≤ 20 words per entry** (OptMem style) | **Why** this change was needed and **why** alternatives/dead ends were rejected. Never restate what files/functions changed. |
| **Tier 3: Cross-Cutting Architecture** | A standing design choice across modules/crates, or an empirical system/hardware finding | **`decision` (`context` + `consequences`) & `observation` (`statement` + `observed_at`)** | **60–150 words** (concise prose with exact metrics) | What empirical measurement or owner ruling forced a binding rule across the codebase, and what trade-off was accepted. |

### 4.4 Which Existing AKR Categories Should Be Removed or Consolidated?

Across `4,467` unique records in the three projects, several of AKR's 13 kinds (`docs/02-data-model.md`) are either dead weight or actively encourage bureaucratic bloat:

| Category | 3-Project Usage (`4,467` unique) | Assessment & Recommendation |
|---|---|---|
| **`term`** | `12` (`0.3%`) — **0 created in last 3 months across all 3 projects** | **REMOVE.** Completely unused in practice; project terminology belongs in `AGENTS.md` or `policy`. |
| **`track`** | `12` (`0.3%`) — `11` in `SYS`, `0` in `Lege`, `0` in `nail-salon` | **REMOVE / FOLD INTO `milestone` OR `policy`.** Standing rules with no end are policies; staged goals are milestones. |
| **`assessment`** | `60` (`1.3%`) — `52` in `SYS`, `7` in `Lege`, `1` in `Kitchen` | **REMOVE / FOLD INTO `observation`.** Agents cannot reliably distinguish an `observation` ("what was found true") from an `assessment` ("a judgement drawn from observations"); root-cause judgements already live in `observation` and choices live in `decision`. |
| **`requirement` & `constraint`** | `98` (`2.2%`) + `34` (`0.8%`) — **0 created in `Lege` or `nail-salon` in 3 months** | **CONSOLIDATE INTO `policy` (or `constraint`).** Having four normative kinds alongside `decision` (`term`, `requirement`, `policy`, `constraint`) causes classification paralysis; in practice (`nail-salon`, `Lege`), agents only use `decision` and `policy`. Keeping just **`decision`** (a choice made between alternatives) and **`policy`** (a standing rule or constraint the project must respect) cuts the normative surface from 5 kinds to 2. |
| **`evidence`** | **`2,025` (`45.3%`)** — #1 source of ledger bloat; ~48% have `title == summary` | **REFORM (Stop minting standalone records for routine test passes).** A passing `cargo test` run that satisfies an acceptance check should be recorded inline on the `work`/`milestone` check block (or via `akr complete <key> --check <id> --command "..." --observed-at <sha>`) rather than minting a standalone 15-line `record ... : evidence` every time `cargo test` exits 0. Reserve standalone empirical records (`observation`) for durable measurements, benchmarks, hardware findings, and falsifications. |

#### Proposed Streamlined AKR Vocabulary (7 Core Kinds Instead of 13)

1. **Planning (2):** `milestone`, `work` (with inline acceptance-check evidence and ≤20-word `why`/`trace` entries)
2. **Normative (2):** `decision` (choice + context + consequences), `policy` (standing rules, requirements, and constraints)
3. **Empirical (2):** `observation` (root-cause findings, benchmarks, and falsifications at `observed_at`), `papercut` (in-the-moment tool/workflow friction with `--about`)
4. **Inquiry (1):** `question` (open blockers)

---

## 5. Immediate Protocol Fixes Required in `AGENTS.md` / Tooling

Even before schema changes land, three concrete workflow gaps uncovered by this audit should be closed:

1. **Restore the `akr change` commit protocol and install git hooks (`akr git install-hooks`) in active workspaces:**
   - `SYS`, `Lege-ecosystem`, and `nail-salon` all dropped to **0% AKR commit trailers** by mid-November because the condensed `<!-- AKR_START -->` block in `CLAUDE.md` / `AGENTS.md` omitted the `akr change begin -> prepare --staged -> akr git commit` steps and no git hooks enforced them.
2. **Fix `evidence add` title/summary duplication in `akr-cli` / `akr-mcp`:**
   - Nearly 50% of all `evidence` records across `SYS` and `Lege-ecosystem` have `title` and `summary` byte-for-byte identical because agents pass the same string to both (or a tool default copies one into the other). `akr evidence add` should reject or auto-omit `summary` when it is identical to `title`.
3. **Ban diff-restatement in `AGENTS.md` instructions:**
   - Explicitly instruct agents in `AGENTS.md`: *"Never use `work`, `decision`, or `observation` prose to summarize what files, structs, or functions a commit changed—Git already stores the diff. Record only WHY the change was made, what alternatives or dead ends were rejected, and what empirical finding motivated it, in ≤20 words for work steps."*
