//! The fact cache changes what a build costs, never what it says.
//!
//! Every cached answer is checked against the computation it replaces, on real
//! repositories whose histories exercise the cases the cache has to get right: a file
//! that only grows, one that is renamed, branches that merge, a version that does not
//! parse, revisions that are added and removed, and a history that is rewritten under
//! the cache.

mod support;

use akr_core::cache::FactCache;
use akr_core::git::{
    Repository, definition_facts, definition_facts_cached, last_changes, last_changes_cached,
};
use akr_core::model::{Ledger, Project};
use akr_core::syntax::{lower::lower_all, parse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use support::TempRepo;

/// A cache directory outside the repository, fresh for each name.
fn cache_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("akr-build-cache-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cache dir");
    dir
}

/// Every cache entry under a cache's `.akr` directory, by file name.
fn entries(akr_dir: &Path) -> BTreeMap<String, String> {
    let Ok(read) = std::fs::read_dir(akr_dir.join("cache").join("facts")) else {
        return BTreeMap::new();
    };
    read.filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).expect("entry"),
            )
        })
        .collect()
}

/// A deterministic generator, so a failure names a reproducible step.
#[derive(Clone)]
struct Steps(u64);

impl Steps {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % below
    }
}

#[derive(Clone)]
struct Rec {
    file: usize,
    name: String,
    revision: u32,
    state: &'static str,
    intent: String,
}

struct Model {
    files: Vec<String>,
    records: Vec<Rec>,
    counter: usize,
}

impl Model {
    fn render(&self, file: usize) -> String {
        let mut out = String::from("akr 0.1\nproject fixtures\n");
        let mut records: Vec<&Rec> = self.records.iter().filter(|r| r.file == file).collect();
        records.sort_by(|a, b| (&a.name, a.revision).cmp(&(&b.name, b.revision)));
        for record in records {
            out.push_str(&format!(
                "\nrecord fx.work.{}/{} : work {{\n    title \"{}\"\n    state {}\n    intent \"\"\"\n        {}\n        \"\"\"\n}}\n",
                record.name, record.revision, record.name, record.state, record.intent
            ));
        }
        out
    }

    fn write(&self, repo: &TempRepo) {
        for (index, path) in self.files.iter().enumerate() {
            repo.write(path, &self.render(index));
        }
    }

    /// One random edit of the kinds a real ledger sees.
    fn mutate(&mut self, steps: &mut Steps) {
        self.counter += 1;
        let tag = self.counter;
        let pick = if self.records.is_empty() {
            0
        } else {
            steps.next(7)
        };
        let at = if self.records.is_empty() {
            0
        } else {
            steps.next(self.records.len())
        };
        match pick {
            // A new record.
            0 => {
                let name = format!("r{tag}");
                self.records.push(Rec {
                    file: steps.next(self.files.len()),
                    name,
                    revision: 1,
                    state: "proposed",
                    intent: format!("Intent {tag}."),
                });
            }
            // A changed definition.
            1 => self.records[at].intent = format!("Redefined at {tag}."),
            // Bookkeeping only: not a definitional change.
            2 => {
                self.records[at].state = if self.records[at].state == "proposed" {
                    "active"
                } else {
                    "proposed"
                }
            }
            // A revision that does not redefine.
            3 | 4 => {
                let base = self.records[at].clone();
                let top = self
                    .records
                    .iter()
                    .filter(|r| r.name == base.name)
                    .map(|r| r.revision)
                    .max()
                    .unwrap_or(1);
                let mut next = base;
                next.revision = top + 1;
                if pick == 4 {
                    next.intent = format!("Revised at {tag}.");
                }
                self.records.push(next);
            }
            // A revision removed from the file again.
            5 => {
                if self.records[at].revision > 1 {
                    self.records.remove(at);
                }
            }
            // A record moved to the other file.
            _ => {
                let name = self.records[at].name.clone();
                let target = (self.records[at].file + 1) % self.files.len();
                for record in self.records.iter_mut().filter(|r| r.name == name) {
                    record.file = target;
                }
            }
        }
    }
}

/// The ledger as the working tree holds it, or `None` while a file does not parse.
fn load(root: &Path, files: &[String]) -> Option<Ledger> {
    let mut parsed = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(root.join(path)).ok()?;
        let result = parse(&text, akr_core::diagnostics::FileId(0));
        if !result.diagnostics.is_empty() {
            return None;
        }
        parsed.push((path.clone(), result.file?));
    }
    let (mut ledger, _) = lower_all(&parsed);
    ledger.project = Project::new("fixtures", &["fx"]);
    Some(ledger)
}

/// The reference walk, a cold cache and the long-lived warm cache all agree, and the warm
/// cache's entries are byte-identical to ones built from scratch.
fn assert_agree(repo: &TempRepo, files: &[String], warm: &Path, step: usize) {
    let Some(ledger) = load(repo.root(), files) else {
        return;
    };
    let reference = last_changes(&Repository::open(repo.root()).expect("opens"), &ledger)
        .expect("reference walk");
    let cold_dir = cache_dir(&format!("cold-{step}"));
    let cold = last_changes_cached(
        &Repository::open(repo.root()).expect("opens"),
        &ledger,
        &FactCache::new(&cold_dir),
    )
    .expect("cold cache");
    let warmed = last_changes_cached(
        &Repository::open(repo.root())
            .expect("opens")
            .with_fact_cache(FactCache::new(warm)),
        &ledger,
        &FactCache::new(warm),
    )
    .expect("warm cache");
    assert!(
        !reference.is_empty() || ledger.records().is_empty(),
        "step {step}"
    );
    assert_eq!(cold, reference, "cold cache disagrees at step {step}");
    assert_eq!(warmed, reference, "warm cache disagrees at step {step}");

    // A second warm call is a pure hit and still agrees.
    let again = last_changes_cached(
        &Repository::open(repo.root()).expect("opens"),
        &ledger,
        &FactCache::new(warm),
    )
    .expect("warm hit");
    assert_eq!(again, reference, "warm hit disagrees at step {step}");

    // Read against a working tree that matches `HEAD`, the working-tree facts are the
    // committed ones and nothing is uncommitted.
    let working = working_texts(repo.root(), files);
    let facts = definition_facts(
        &Repository::open(repo.root()).expect("opens"),
        &ledger,
        &working,
    )
    .expect("reference working facts");
    assert!(
        facts.uncommitted.is_empty(),
        "step {step}: {:?}",
        facts.uncommitted
    );
    assert_eq!(
        facts.last_change, reference,
        "clean working facts at step {step}"
    );
    let cached_facts = definition_facts_cached(
        &Repository::open(repo.root()).expect("opens"),
        &ledger,
        &working,
        &FactCache::new(warm),
    )
    .expect("cached working facts");
    assert_eq!(
        cached_facts, facts,
        "cached clean working facts at step {step}"
    );

    let cold_entries = entries(&cold_dir);
    let warm_entries = entries(warm);
    for (name, text) in &cold_entries {
        assert_eq!(
            warm_entries.get(name),
            Some(text),
            "extended entry {name} differs from a rebuilt one at step {step}"
        );
    }
    let _ = std::fs::remove_dir_all(&cold_dir);

    // What each commit touched, over the whole history and over partial ranges, from git
    // and from the warm cache.
    let head = akr_core::model::Commit::new(&repo.rev_parse("HEAD")).expect("hash");
    let plain = Repository::open(repo.root()).expect("opens");
    let cached = Repository::open(repo.root())
        .expect("opens")
        .with_fact_cache(FactCache::new(warm));
    let mut froms = vec![None];
    for back in ["HEAD~1", "HEAD~4", "HEAD~9"] {
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", back])
            .current_dir(repo.root())
            .output()
            && output.status.success()
        {
            froms.push(Some(
                akr_core::model::Commit::new(String::from_utf8_lossy(&output.stdout).trim())
                    .expect("hash"),
            ));
        }
    }
    for from in &froms {
        assert_eq!(
            cached
                .touches_in(from.as_ref(), &head)
                .expect("cached touches"),
            plain.touches_in(from.as_ref(), &head).expect("touches"),
            "touches from {from:?} disagree at step {step}"
        );
    }
}

/// Every ledger file's working-tree text, by path.
fn working_texts(root: &Path, files: &[String]) -> BTreeMap<String, String> {
    files
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(root.join(path)).ok()?;
            Some((path.clone(), text))
        })
        .collect()
}

/// With uncommitted edits on top of `HEAD`, the cached working-tree facts equal the
/// reference walk's, and a revision is uncommitted exactly when its definition is one no
/// commit holds at or below it. The committed tree is restored afterwards.
fn assert_working_agree(
    repo: &TempRepo,
    model: &Model,
    warm: &Path,
    steps: &mut Steps,
    step: usize,
) -> (usize, usize) {
    let mut seen = (0, 0);
    let mut edited = Model {
        files: model.files.clone(),
        records: model.records.clone(),
        counter: model.counter,
    };
    for _ in 0..3 {
        edited.mutate(steps);
    }
    edited.write(repo);
    if let Some(ledger) = load(repo.root(), &edited.files) {
        let working = working_texts(repo.root(), &edited.files);
        let reference = definition_facts(
            &Repository::open(repo.root()).expect("opens"),
            &ledger,
            &working,
        )
        .expect("reference working facts");
        let cached = definition_facts_cached(
            &Repository::open(repo.root()).expect("opens"),
            &ledger,
            &working,
            &FactCache::new(warm),
        )
        .expect("cached working facts");
        assert_eq!(cached, reference, "working facts disagree at step {step}");
        seen = (reference.last_change.len(), reference.uncommitted.len());
        let answered: usize = reference.last_change.len() + reference.uncommitted.len();
        assert_eq!(answered, ledger.records().len(), "step {step}");
        // A record the edits did not touch keeps its committed answer.
        let committed = last_changes(&Repository::open(repo.root()).expect("opens"), &ledger)
            .expect("committed walk");
        for record in &model.records {
            if edited.records.iter().any(|r| {
                r.name == record.name
                    && r.revision == record.revision
                    && r.intent == record.intent
                    && r.file == record.file
            }) && edited
                .records
                .iter()
                .filter(|r| r.name == record.name)
                .count()
                == model
                    .records
                    .iter()
                    .filter(|r| r.name == record.name)
                    .count()
            {
                let id = akr_core::model::RevisionId::new(
                    akr_core::model::LogicalKey::parse(&format!("fx.work.{}", record.name))
                        .expect("key"),
                    record.revision,
                );
                if let Some(commit) = committed.get(&id) {
                    assert_eq!(
                        reference.last_change.get(&id),
                        Some(commit),
                        "untouched {id} moved at step {step}"
                    );
                }
            }
        }
    }
    model.write(repo);
    seen
}

#[test]
fn cached_last_changes_match_the_reference_walk_across_a_rough_history() {
    let mut repo = TempRepo::new("build-cache-history");
    let warm = cache_dir("warm");
    let mut model = Model {
        files: vec![
            ".akr/records/one.akr".to_owned(),
            ".akr/records/two.akr".to_owned(),
        ],
        records: Vec::new(),
        counter: 0,
    };
    let mut steps = Steps(0x00ac_e5eed);
    let mut rewind_to = String::new();
    let mut working_seen = (0, 0);

    for step in 0..44 {
        match step {
            // The file is renamed: `--follow` history changes shape behind the cache.
            12 => {
                repo.rename(".akr/records/two.akr", ".akr/records/renamed.akr");
                model.files[1] = ".akr/records/renamed.akr".to_owned();
                model.mutate(&mut steps);
                model.write(&repo);
                repo.commit(&format!("step {step}: rename"));
            }
            // Two lines of work merge.
            20 => {
                repo.branch("side");
                model.mutate(&mut steps);
                model.write(&repo);
                repo.commit(&format!("step {step}: side"));
                let side = model.records.clone();
                repo.checkout("main");
                // The main line edits a different file so the merge is clean.
                repo.write("notes.txt", &format!("{step}\n"));
                repo.commit(&format!("step {step}: main"));
                repo.merge("side");
                model.records = side;
            }
            // A version that does not parse, fixed by the next commit.
            26 => {
                repo.write(&model.files[0], "akr 0.1\nproject fixtures\n\nrecord {{{\n");
                repo.commit(&format!("step {step}: broken"));
                continue;
            }
            28 => {
                rewind_to = repo.rev_parse("HEAD");
                model.mutate(&mut steps);
                model.write(&repo);
                repo.commit(&format!("step {step}"));
            }
            // History is rewritten under the cache.
            33 => {
                repo.reset_hard(&rewind_to);
                let ledger = load(repo.root(), &model.files);
                assert!(ledger.is_some(), "the rewound tree parses");
                model.records.retain(|r| r.revision < 3);
                model.mutate(&mut steps);
                model.write(&repo);
                repo.commit(&format!("step {step}: rewritten"));
            }
            _ => {
                model.mutate(&mut steps);
                if step % 5 == 0 {
                    model.mutate(&mut steps);
                }
                model.write(&repo);
                repo.commit(&format!("step {step}"));
            }
        }
        // Not every step is checked, so the warm cache also has to extend by several
        // commits at once.
        if step % 3 != 1 {
            assert_agree(&repo, &model.files, &warm, step);
            let seen = assert_working_agree(&repo, &model, &warm, &mut steps.clone(), step);
            working_seen.0 += seen.0;
            working_seen.1 += seen.1;
        }
    }
    assert_agree(&repo, &model.files, &warm, 99);
    // Both answers occurred, so the working-tree comparison was not vacuous.
    assert!(working_seen.0 > 0 && working_seen.1 > 0, "{working_seen:?}");
    let _ = std::fs::remove_dir_all(&warm);
}

#[test]
fn a_corrupt_or_foreign_cache_entry_is_a_miss() {
    let mut repo = TempRepo::new("build-cache-corrupt");
    let files = vec![".akr/records/one.akr".to_owned()];
    let model = Model {
        files: files.clone(),
        records: vec![Rec {
            file: 0,
            name: "a".to_owned(),
            revision: 1,
            state: "proposed",
            intent: "A.".to_owned(),
        }],
        counter: 0,
    };
    model.write(&repo);
    repo.commit("one");
    let warm = cache_dir("corrupt");
    assert_agree(&repo, &files, &warm, 0);

    for (name, text) in entries(&warm) {
        let path = warm.join("cache").join("facts").join(&name);
        // Truncated mid-entry, then from another tool version.
        std::fs::write(&path, &text[..text.len() / 2]).expect("truncate");
        assert_agree(&repo, &files, &warm, 1);
        std::fs::write(&path, text.replacen("akr-facts 1", "akr-facts 0", 1)).expect("stamp");
        assert_agree(&repo, &files, &warm, 2);
    }
    let _ = std::fs::remove_dir_all(&warm);
}

#[test]
fn source_hits_are_cached_across_ledger_commits_and_refreshed_by_source_commits() {
    let mut repo = TempRepo::new("build-cache-source-hits");
    repo.write("src/lib.rs", "fn alpha_test() {}\nfn beta_test() {}\n");
    repo.write(".akr/records/one.akr", "akr 0.1\nproject fixtures\n");
    repo.write("docs/generated/VIEW.md", "one\n");
    let first = repo.commit("source");
    let dir = cache_dir("source-hits");
    let cache = FactCache::new(&dir);
    let entry = dir.join("cache").join("facts").join("source-hits");
    let root = repo.root().to_path_buf();
    let ask = |head: &str, needles: &[&str], cache: Option<&FactCache>| {
        Repository::open(&root)
            .expect("opens")
            .tracked_source_hits(
                &akr_core::model::Commit::new(head).expect("hash"),
                needles.iter().copied(),
                cache,
            )
            .expect("hits")
    };

    let needles = ["alpha_test", "gamma_test"];
    let cached = ask(&first, &needles, Some(&cache));
    assert_eq!(cached, ask(&first, &needles, None));
    assert!(cached.contains("alpha_test") && !cached.contains("gamma_test"));
    let written = std::fs::read_to_string(&entry).expect("entry written");

    // A ledger-only commit, views included, leaves the source fingerprint alone: the
    // answer comes from the entry, which is not rewritten.
    repo.write(".akr/records/one.akr", "akr 0.1\nproject fixtures\n\n");
    repo.write("docs/generated/VIEW.md", "two\n");
    let ledger_only = repo.commit("ledger");
    assert_eq!(ask(&ledger_only, &needles, Some(&cache)), cached);
    assert_eq!(std::fs::read_to_string(&entry).expect("entry"), written);

    // A new needle against the same source is answered and remembered with the rest.
    let more = ["alpha_test", "beta_test", "gamma_test"];
    let answered = ask(&ledger_only, &more, Some(&cache));
    assert_eq!(answered, ask(&ledger_only, &more, None));
    assert!(
        std::fs::read_to_string(&entry)
            .expect("entry")
            .contains("hit beta_test")
    );

    // A source commit invalidates it.
    repo.write("src/lib.rs", "fn gamma_test() {}\n");
    let source = repo.commit("rename the tests");
    let after = ask(&source, &more, Some(&cache));
    assert_eq!(after, ask(&source, &more, None));
    assert!(after.contains("gamma_test") && !after.contains("alpha_test"));
    let _ = std::fs::remove_dir_all(&dir);
}
