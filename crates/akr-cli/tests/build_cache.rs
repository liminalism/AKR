//! The fact cache through the binary: a warm cache and an empty one say the same thing.
//!
//! `akr-core`'s `build_cache` tests hold each cached computation to the one it replaces.
//! This holds the commands to it: every diagnostic, every line of a report, and every
//! byte `akr build` writes must be identical whether `.akr/cache/facts` was full or
//! empty — before a commit, and after one the warm cache has to extend over.

mod support;

use support::{Example, SAVE_YOUR_SKIN, SYS_TANDEM};

const COMMANDS: &[&[&str]] = &[
    &["check"],
    &["check", "--format", "json"],
    &["review-queue"],
    &["build", "--check"],
];

/// What every command printed, and what `akr build` wrote, as one comparable text.
fn observe(example: &Example) -> String {
    let mut out = String::new();
    for args in COMMANDS {
        let run = example.run(args);
        out.push_str(&format!(
            "$ akr {}\nexit {}\n{}\n{}\n",
            args.join(" "),
            run.code,
            run.stdout,
            run.stderr
        ));
    }
    let build = example.run(&["build"]);
    out.push_str(&format!("$ akr build\nexit {}\n", build.code));
    let mut files = vec![".akr/akr.lock".to_owned()];
    let mut views: Vec<String> = std::fs::read_dir(example.root().join("docs/generated"))
        .expect("views written")
        .filter_map(Result::ok)
        .map(|entry| format!("docs/generated/{}", entry.file_name().to_string_lossy()))
        .collect();
    views.sort();
    files.extend(views);
    for file in files {
        out.push_str(&format!("--- {file}\n{}\n", example.read_file(&file)));
    }
    out
}

fn clear(example: &Example) {
    let _ = std::fs::remove_dir_all(example.root().join(".akr/cache/facts"));
}

fn facts_written(example: &Example) -> bool {
    std::fs::read_dir(example.root().join(".akr/cache/facts"))
        .is_ok_and(|mut entries| entries.next().is_some())
}

#[test]
fn cold_and_warm_caches_produce_byte_identical_output() {
    for (fixture, name) in [
        (&SAVE_YOUR_SKIN, "save-your-skin"),
        (&SYS_TANDEM, "sys-tandem"),
    ] {
        let example = Example::of(fixture, &format!("cache-parity-{name}"));

        // As materialised: the examples' history is synthetic and holds no record file.
        clear(&example);
        let cold = observe(&example);
        let warm = observe(&example);
        assert_eq!(cold, warm, "{name}: warm output differs from cold");

        // The ledger committed: now every record file has a history to cache. The cache
        // itself is ignored, as `akr init` arranges, so it never enters history.
        example.write_file(".gitignore", ".akr/cache/\n");
        example.git(&["add", "-A"]);
        example.git(&["commit", "--quiet", "-m", "ledger"]);
        clear(&example);
        let cold = observe(&example);
        assert!(
            facts_written(&example),
            "{name}: a cold run fills the cache"
        );
        let warm = observe(&example);
        assert_eq!(cold, warm, "{name}: warm output differs from cold");

        // A commit to one record file: the warm cache extends over it.
        let logged = example.run(&["papercut", "-m", "test", "a cache parity papercut"]);
        assert_eq!(logged.code, 0, "{name}: {}", logged.output());
        example.run(&["build"]);
        example.git(&["add", "-A"]);
        example.git(&["commit", "--quiet", "-m", "papercut"]);
        let extended = observe(&example);
        clear(&example);
        let rebuilt = observe(&example);
        assert_eq!(
            extended, rebuilt,
            "{name}: output after extending the cache differs from a cold build"
        );
    }
}
