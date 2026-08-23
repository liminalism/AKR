//! The conformance corpus: `fixtures/validate/ok` must resolve clean, and
//! `fixtures/validate/err` must produce exactly the codes its `.expected` file names.
//!
//! Exit criterion 1 of `docs/13-implementation-roadmap.md` P3.
//!
//! # What is asserted, and what is not
//!
//! Codes and their multiplicity are asserted. **Line numbers are not**, because the
//! validation rules produce [`Subject`](akr_core::diagnostics::Subject)-bearing
//! diagnostics with `span: None` — attaching spans by mapping a subject back to a byte
//! range is the join between P2 and P3 and has not been written. Every `.expected` file
//! carries a line number, and this harness reads and ignores it; when span attachment
//! lands, the two `assert_codes` helpers below grow one more comparison and the fixtures
//! need no change. That is the seam, and it is deliberately narrow.

use akr_core::diagnostics::{Diagnostic, FileId, Severity};
use akr_core::model::{Ledger, Project};
use akr_core::resolve::{BuildInputs, ResolvedModel};
use akr_core::syntax::{lower::lower_all, parse};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/validate")
}

/// The project declaration every validate fixture shares.
fn fixture_project() -> Project {
    let text = fs::read_to_string(fixtures().join("project.akr")).expect("project.akr");
    let parsed = parse(&text, FileId(0));
    let file = parsed.file.expect("project.akr parses");
    lower_all(&[("project.akr".to_owned(), file)]).0.project
}

/// Loads one fixture — a single `.akr` file, or a directory of them — into a ledger.
fn load(paths: &[PathBuf]) -> (Ledger, Vec<Diagnostic>) {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let text = fs::read_to_string(path).expect("readable fixture");
        let parsed = parse(&text, FileId(u32::try_from(index).expect("few files")));
        diagnostics.extend(parsed.diagnostics);
        if let Some(file) = parsed.file {
            let name = path
                .file_name()
                .expect("named")
                .to_string_lossy()
                .into_owned();
            files.push((name, file));
        }
    }
    let (mut ledger, lower_diagnostics) = lower_all(&files);
    diagnostics.extend(lower_diagnostics);
    ledger.project = fixture_project();
    (ledger, diagnostics)
}

/// The codes a fixture expects, from its `.expected` file. Line numbers are read and
/// discarded — see the module docs.
fn expected_codes(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("readable .expected");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().next().expect("a code").to_owned())
        .collect()
}

/// Fixtures that produce diagnostics their `.expected` file does not list.
///
/// `fixtures/README.md` §2 says a `.expected` file is the **complete** set of diagnostics
/// for the stage under test, "which is what keeps fixtures isolated to one fault". This
/// table once carried five waivers found during P3 cross-validation; all five were
/// resolved at the source — v004 and v011 were re-isolated, V-008 now defers to the
/// rule that owns a slot (V-009/V-010/V-011), and v017's deliberate two-code pair is
/// listed in its own `.expected` and documented in `fixtures/README.md` §2. The table
/// stays so any future non-isolated fixture has a documented, greppable home.
const WAIVED_EXTRA: &[(&str, &[&str])] = &[];

fn waived_for(fixture: &str) -> Vec<String> {
    WAIVED_EXTRA
        .iter()
        .filter(|(name, _)| fixture.starts_with(name))
        .flat_map(|(_, codes)| codes.iter().map(|c| (*c).to_owned()))
        .collect()
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<String> {
    let mut codes: Vec<String> = diagnostics
        .iter()
        .map(|d| d.code.as_str().to_owned())
        .collect();
    codes.sort();
    codes
}

fn describe(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| format!("  {} {}", d.code, d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every `.akr` file directly in a directory, sorted.
fn akr_files_in(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)?
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "akr"))
        .collect();
    out.sort();
    Ok(out)
}

fn build_model<'a>(ledger: &'a Ledger, inputs: &BuildInputs) -> ResolvedModel<'a> {
    ResolvedModel::build(ledger, inputs)
}

fn inputs() -> BuildInputs {
    BuildInputs {
        tool: "akr 0.1.0".to_owned(),
        grammar: "0.1".to_owned(),
        vocabulary: "0.1".to_owned(),
        ..BuildInputs::default()
    }
}

// -------------------------------------------------------------------------------------

#[test]
fn every_ok_fixture_resolves_clean() {
    let dir = fixtures().join("ok");
    let mut checked = 0;
    for path in akr_files_in(&dir).expect("fixtures/validate/ok") {
        let (ledger, parse_diagnostics) = load(std::slice::from_ref(&path));
        assert!(
            parse_diagnostics.is_empty(),
            "{}: parse/lower diagnostics\n{}",
            path.display(),
            describe(&parse_diagnostics)
        );
        let model = build_model(&ledger, &inputs());
        assert!(
            model.diagnostics.is_empty(),
            "{}: expected a clean resolve, got\n{}",
            path.display(),
            describe(&model.diagnostics)
        );
        assert!(!model.has_errors());
        checked += 1;
    }
    assert!(checked >= 3, "expected the ok corpus to be non-trivial");
}

#[test]
fn every_err_fixture_produces_exactly_its_expected_codes() {
    let dir = fixtures().join("err");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixtures/validate/err")
        .collect::<io::Result<Vec<_>>>()
        .expect("readable")
        .into_iter()
        .map(|e| e.path())
        .collect();
    entries.sort();

    let mut checked = 0;
    for entry in entries {
        // Two shapes: `vNNN-name.akr` + `vNNN-name.expected`, or a directory of `.akr`
        // files plus a file named `expected`.
        let (sources, expected_path) = if entry.is_dir() {
            (
                akr_files_in(&entry).expect("fixture directory"),
                entry.join("expected"),
            )
        } else if entry.extension().is_some_and(|e| e == "akr") {
            (vec![entry.clone()], entry.with_extension("expected"))
        } else {
            continue;
        };
        if !expected_path.exists() {
            continue;
        }

        let name = entry
            .file_name()
            .expect("named")
            .to_string_lossy()
            .into_owned();
        let expected = expected_codes(&expected_path);
        let (mut ledger, parse_diagnostics) = load(&sources);

        // V-024 fixtures ship a lock; without it the rule is silent by design.
        let lock_path = if entry.is_dir() {
            entry.join("akr.lock")
        } else {
            entry.with_extension("lock")
        };
        let mut extra = Vec::new();
        if lock_path.exists() {
            let text = fs::read_to_string(&lock_path).expect("readable lock");
            let lock = akr_core::lock::Lock::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));

            // Compute what the build would produce, so the seal comparison is real.
            let mut computed = BTreeMap::new();
            for (index, path) in sources.iter().enumerate() {
                let source_text = fs::read_to_string(path).expect("readable");
                let parsed = parse(&source_text, FileId(u32::try_from(index).expect("few")));
                if let Some(file) = &parsed.file {
                    for at in 0..file.items.len() {
                        if let Some(canonical) = akr_core::resolve::canonical_record_text(file, at)
                            && let akr_core::syntax::cst::Item::Record(record) = &file.items[at]
                            && let Ok(logical) = akr_core::model::LogicalKey::parse(&record.key)
                        {
                            computed.insert(
                                akr_core::model::RevisionId::new(logical, record.revision),
                                akr_core::hash::content_hash(&canonical),
                            );
                        }
                    }
                }
            }
            lock.apply_facts(&mut ledger, &computed);

            // The other half of AKR-R052: the lock's own currency against the sources.
            let mut computed_lock = lock.clone();
            for source in &mut computed_lock.sources {
                let path = entry.join(&source.path);
                if let Ok(bytes) = fs::read(&path) {
                    source.hash = akr_core::hash::source_file_hash(&bytes);
                }
            }
            extra.extend(akr_core::lock::currency_diagnostics(
                &lock,
                &computed_lock,
                "akr.lock",
            ));
        }

        let model = build_model(&ledger, &inputs());
        let mut found = parse_diagnostics;
        found.extend(model.diagnostics.clone());
        found.extend(extra);

        let mut want = expected.clone();
        want.extend(waived_for(&name));
        want.sort();
        assert_eq!(
            codes_of(&found),
            want,
            "{name}: diagnostics were\n{}",
            describe(&found)
        );
        assert!(
            found.iter().all(|d| d.severity == Severity::Error),
            "{name}: validate fixtures assert errors"
        );
        checked += 1;
    }
    assert!(
        checked >= 20,
        "expected the err corpus to cover most of V-001..V-025, checked {checked}"
    );
}

#[test]
fn every_v_rule_with_a_fixture_is_reachable() {
    // The fixture names encode the rule they exercise. This asserts the corpus covers the
    // catalogue rather than a subset of it.
    let dir = fixtures().join("err");
    let mut covered: Vec<u16> = fs::read_dir(&dir)
        .expect("err fixtures")
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix('v')
                .and_then(|rest| rest.get(..3))
                .and_then(|digits| digits.parse::<u16>().ok())
        })
        .collect();
    covered.sort_unstable();
    covered.dedup();
    for rule in akr_core::validate::RULES {
        assert!(
            covered.contains(&rule.id.0),
            "{} has no fixture in fixtures/validate/err",
            rule.id
        );
    }
}
