//! `docs/07` §4: one writer at a time, and a racing edit is refused rather than overwritten.
//!
//! The write pipeline is a read-modify-write — step 1 reads every source, step 5 writes
//! each touched file whole — so per-file atomicity, which is all the rename gives, says
//! nothing about two writers. Before step 0 existed, two processes that both read before
//! either wrote each held a ledger that did not know about the other's record, and the
//! second rename discarded the first while both reported success. Six concurrent `akr
//! papercut` processes landed two records
//! (`@akr.observation.concurrent-writes-are-silently-lost`).
//!
//! The claim under test is the strong one: every writer that reports success has its
//! record in the file afterwards. Counting records is not enough — a run that loses one
//! record and gains another counts the same — so each test checks that every key it asked
//! for is present.

mod ops_support;

use akr_core::model::{ContentSlot, ContentValue, Kind, Record, RecordBuilder, key};
use akr_core::ops::{self, WriteContext};
use ops_support::Sandbox;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn term(key_text: &str, title: &str) -> Record {
    let mut record = RecordBuilder::new(key_text, 1, Kind::Term)
        .title(title)
        .all_scope()
        .build();
    record.content.insert(
        ContentSlot::Definition,
        ContentValue::prose("A definition supplied by the caller."),
    );
    record
}

/// Sixteen writers proposing distinct keys into the same conventional file, at once.
///
/// Distinct keys are the case the ledger has no other defence for: `base_rev` is
/// optimistic concurrency over one key's head revision, and sixteen *new* keys engage it
/// nowhere. They all land in the same `terms.akr`, so every writer rewrites the file the
/// other fifteen are rewriting.
#[test]
fn concurrent_proposals_of_distinct_keys_all_survive() {
    let sandbox = Sandbox::save_your_skin();
    let akr_dir = sandbox.akr_dir();
    const WRITERS: usize = 16;

    let accepted = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for n in 0..WRITERS {
            let akr_dir = akr_dir.clone();
            let accepted = Arc::clone(&accepted);
            scope.spawn(move || {
                let context = WriteContext::new(akr_dir).with_author("racer");
                let key_text = format!("sys.term.racer-{n:02}");
                if ops::propose(
                    &context,
                    &key(&key_text),
                    Kind::Term,
                    "Racer",
                    Some(term(&key_text, "Racer")),
                )
                .is_ok()
                {
                    accepted.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });

    // Serialised writers all succeed; that is the point of queueing rather than refusing.
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        WRITERS,
        "every writer should have been able to take its turn"
    );

    let ledger = sandbox.ledger();
    for n in 0..WRITERS {
        let target = key(&format!("sys.term.racer-{n:02}"));
        assert!(
            !ledger.revisions_of(&target).is_empty(),
            "sys.term.racer-{n:02} reported success but is not in the ledger; \
             a concurrent write clobbered it"
        );
    }
}

/// The same race with every writer aimed at a file that already holds records.
///
/// `propose` picks the conventional file for the kind, so proposing decisions puts every
/// new record into a file with existing content to preserve. A lost update here would
/// show up as missing *existing* records as well as missing new ones.
#[test]
fn concurrent_proposals_preserve_the_records_already_in_the_file() {
    let sandbox = Sandbox::save_your_skin();
    let akr_dir = sandbox.akr_dir();
    let before: Vec<String> = sandbox
        .ledger()
        .records()
        .iter()
        .map(|record| record.id.to_string())
        .collect();
    assert!(!before.is_empty(), "the example has records to preserve");

    std::thread::scope(|scope| {
        for n in 0..8 {
            let akr_dir = akr_dir.clone();
            scope.spawn(move || {
                let context = WriteContext::new(akr_dir).with_author("racer");
                let key_text = format!("sys.term.keeper-{n:02}");
                let _ = ops::propose(
                    &context,
                    &key(&key_text),
                    Kind::Term,
                    "Keeper",
                    Some(term(&key_text, "Keeper")),
                );
            });
        }
    });

    let after = sandbox.ledger();
    let present: Vec<String> = after
        .records()
        .iter()
        .map(|record| record.id.to_string())
        .collect();
    for id in &before {
        assert!(
            present.contains(id),
            "{id} was in the ledger before the concurrent writes and is gone"
        );
    }
}

/// A source that changes between the read and the rename is refused, not overwritten.
///
/// This is the half a lock cannot cover: an editor, a `git checkout`, or a filesystem
/// whose locking does not work. AKR holds the lock for the whole operation, so the change
/// has to arrive from outside it.
///
/// The check is exercised directly rather than through `propose`, because a change made
/// before the call is a change the operation simply *reads*, and one made after it returns
/// is too late. Landing an edit inside the window would need the pipeline to pause
/// mid-operation for the test's benefit. What is asserted instead is the whole contract
/// `apply_inner` relies on, over exactly the map it passes: unchanged verifies, and
/// edited, deleted and newly-created files are each reported.
#[test]
fn a_source_edited_under_the_write_is_refused_with_c034() {
    let sandbox = Sandbox::save_your_skin();
    let target = key("sys.term.raced-away");
    let file = ops::conventional_file(&target, Kind::Term);
    let full = sandbox.akr_dir().join(&file);

    let original = std::fs::read_to_string(&full).unwrap_or_default();
    let before: std::collections::BTreeMap<std::path::PathBuf, Option<String>> =
        [(file.clone(), Some(original.clone()))]
            .into_iter()
            .collect();

    assert!(
        akr_core::ops::verify_unchanged(&sandbox.akr_dir(), before.iter()).is_none(),
        "an untouched file must verify"
    );

    std::fs::write(&full, format!("{original}\n// a hand edit\n")).expect("writable");
    assert_eq!(
        akr_core::ops::verify_unchanged(&sandbox.akr_dir(), before.iter()),
        Some(file.clone()),
        "an edited file must be reported, so the write refuses instead of overwriting it"
    );

    // A file that vanished counts as changed too.
    std::fs::remove_file(&full).expect("removable");
    assert_eq!(
        akr_core::ops::verify_unchanged(&sandbox.akr_dir(), before.iter()),
        Some(file.clone()),
        "a deleted file must be reported"
    );

    // And so does one that appeared where the operation read nothing.
    let absent: std::collections::BTreeMap<std::path::PathBuf, Option<String>> =
        [(file.clone(), None)].into_iter().collect();
    std::fs::write(&full, "anything").expect("writable");
    assert_eq!(
        akr_core::ops::verify_unchanged(&sandbox.akr_dir(), absent.iter()),
        Some(file),
        "a file created where the operation read none must be reported"
    );
}

/// The lock is taken and released per operation, so a second write in the same process
/// is not blocked by the first.
///
/// Worth pinning: file locks on Unix are held per open file description, not per process,
/// so a guard that outlived its operation — or an operation that re-entered another —
/// would deadlock against itself rather than fail a test somewhere visible.
#[test]
fn successive_writes_in_one_process_do_not_deadlock() {
    let sandbox = Sandbox::save_your_skin();
    let context = WriteContext::new(sandbox.akr_dir()).with_author("tester");
    for n in 0..3 {
        let key_text = format!("sys.term.sequential-{n}");
        ops::propose(
            &context,
            &key(&key_text),
            Kind::Term,
            "Sequential",
            Some(term(&key_text, "Sequential")),
        )
        .expect("each write in turn is accepted");
    }
    let ledger = sandbox.ledger();
    for n in 0..3 {
        assert!(
            !ledger
                .revisions_of(&key(&format!("sys.term.sequential-{n}")))
                .is_empty()
        );
    }
}
