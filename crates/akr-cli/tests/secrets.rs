//! Exercise the public/private boundary through the shipped command line.
mod support;
use support::Example;

const VALUE: &str = "synthetic-private-value-never-published\nD:\\private\\developer.env\n";

fn put(example: &Example) -> String {
    // Model a workspace scaffolded before 0.6.5: old ignores, no root secret rule.
    example.write_file(
        ".gitignore",
        ".akr/cache/\n.agent/scratch/\n.agent/handoffs/\n",
    );
    example.write_file(".agent/scratch/input.txt", VALUE);
    let output = example.run(&["secret", "put", "--from", ".agent/scratch/input.txt"]);
    assert_eq!(output.code, 0, "{}", output.output());
    assert!(!output.output().contains(VALUE));
    assert!(!output.output().contains("input.txt"));
    output.stdout.trim().to_owned()
}

#[test]
fn private_developer_and_public_clone_workflows() {
    let example = Example::materialise("secret-public-clone");
    let explain = example.run(&["explain", "secret", "--format", "json"]);
    assert_eq!(explain.code, 0);
    assert!(explain.stdout.contains("\"category\": \"secret\""));
    let reference = put(&example);
    example.write_file(
        ".agent/scratch/document.txt",
        &format!("Developer config: {reference}."),
    );
    example.write_file(
        ".agent/scratch/proposal.akr",
        &format!("intent \"Developer config: {reference}.\"\n"),
    );
    let proposed = example.run(&[
        "propose",
        "sys.work.private-config",
        "--kind",
        "work",
        "--from",
        ".agent/scratch/proposal.akr",
    ]);
    assert_eq!(proposed.code, 0, "{}", proposed.output());
    assert_eq!(example.run(&["build"]).code, 0);
    for args in [
        vec!["get", "sys.work.private-config"],
        vec!["get", "sys.work.private-config", "--format", "json"],
        vec!["context", "--goal", "sys.work.private-config"],
        vec!["view", "active-work"],
        vec!["search", "Developer"],
        vec!["handoff", "capsule"],
    ] {
        let run = example.run(&args);
        assert_eq!(run.code, 0, "{args:?}: {}", run.output());
        assert!(
            !run.output().contains("synthetic-private-value"),
            "{args:?}"
        );
        assert!(
            !run.output().contains(&reference),
            "public output must show placeholders: {args:?}"
        );
    }
    let canonical = example.run(&["get", "sys.work.private-config", "--detail", "canonical"]);
    assert!(canonical.stdout.contains(&reference));
    assert!(!canonical.stdout.contains("synthetic-private-value"));
    let get = example.run(&["secret", "get", &reference]);
    assert_eq!(get.stdout, "[*]\n");
    let reveal = example.run(&["secret", "get", &reference, "--reveal"]);
    assert_eq!(reveal.stdout, format!("{VALUE}\n"));
    let resolve = example.run(&[
        "secret",
        "resolve",
        "--from",
        ".agent/scratch/document.txt",
        "--reveal",
    ]);
    assert_eq!(resolve.stdout, format!("Developer config: {VALUE}."));
    let public_before = example.read_file("docs/generated/ACTIVE-WORK.md");
    assert!(public_before.contains("Developer config: [*]."));
    fn scan(path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(&path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                assert!(
                    !bytes
                        .windows(b"synthetic-private-value".len())
                        .any(|w| w == b"synthetic-private-value"),
                    "private value leaked into {}",
                    path.display()
                );
            }
        }
    }
    for path in [
        ".akr/records",
        ".akr/cache",
        ".agent/handoffs",
        "docs/generated",
    ] {
        scan(&example.root().join(path));
    }
    example.git(&["add", "-A"]);
    let tracked = std::process::Command::new("git")
        .args(["ls-files", ".akr/secrets"])
        .current_dir(example.root())
        .output()
        .unwrap();
    assert!(
        tracked.stdout.is_empty(),
        "private files must be ignored, including in old workspaces"
    );
    let tree = std::process::Command::new("git")
        .args(["write-tree"])
        .current_dir(example.root())
        .output()
        .unwrap();
    assert!(tree.status.success());
    let tree = String::from_utf8(tree.stdout).unwrap();
    let archive = std::process::Command::new("git")
        .args(["archive", tree.trim()])
        .current_dir(example.root())
        .output()
        .unwrap();
    assert!(archive.status.success());
    assert!(
        archive
            .stdout
            .windows(reference.len())
            .any(|w| w == reference.as_bytes()),
        "published source retains opaque citations"
    );
    assert!(
        !archive
            .stdout
            .windows(b"synthetic-private-value".len())
            .any(|w| w == b"synthetic-private-value"),
        "Git publication must not contain private values"
    );
    // Remove the private category to model someone receiving only the public ledger.
    std::fs::remove_dir_all(example.root().join(".akr/secrets")).unwrap();
    assert_eq!(
        example
            .run(&["secret", "get", &reference, "--reveal"])
            .stdout,
        "[*]\n"
    );
    assert_eq!(example.run(&["build"]).code, 0);
    assert_eq!(
        public_before,
        example.read_file("docs/generated/ACTIVE-WORK.md")
    );
    assert_eq!(example.run(&["check"]).code, 0);
}

#[test]
fn force_added_private_entries_and_public_input_routes_are_refused() {
    let example = Example::materialise("secret-publication-guard");
    let reference = put(&example);
    let path = format!(".akr/secrets/{}.json", &reference[9..41]);
    for args in [
        vec!["source", "add", path.as_str()],
        vec!["import", path.as_str()],
        vec!["fmt", path.as_str()],
        vec!["secret", "resolve", "--from", path.as_str()],
        vec![
            "propose",
            "sys.work.leak",
            "--kind",
            "work",
            "--from",
            path.as_str(),
        ],
    ] {
        let run = example.run(&args);
        assert_ne!(run.code, 0, "{args:?}");
        assert!(run.output().contains("AKR-C046"), "{}", run.output());
        assert!(!run.output().contains("synthetic-private-value"));
    }
    example.git(&["add", "-f", &path]);
    let request = example.run(&["handoff", "session", "begin", "--request-file", &path]);
    assert_ne!(request.code, 0);
    assert!(request.output().contains("private secret storage"));
    assert!(!request.output().contains("synthetic-private-value"));
    assert_eq!(
        example
            .run(&[
                "change",
                "begin",
                "--kind",
                "feat",
                "--summary",
                "Test guard",
                "--untracked-reason",
                "test"
            ])
            .code,
        0
    );
    for args in [
        vec!["check"],
        vec!["build"],
        vec!["change", "prepare", "--staged"],
    ] {
        let run = example.run(&args);
        assert!(run.output().contains("AKR-C046"), "{}", run.output());
        assert!(!run.output().contains("synthetic-private-value"));
    }
}
