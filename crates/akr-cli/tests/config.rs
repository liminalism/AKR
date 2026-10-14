//! Configuration edits through the public CLI, including refusal atomicity.
mod support;

use support::Example;

#[test]
fn configuration_cannot_declare_tracked_code_untracked() {
    let example = Example::materialise("config-tracked");
    let before = example.sources();
    let run = example.run(&["config", "untracked", "add", "sim/src/**"]);
    assert_eq!(run.code, 1, "{}", run.output());
    assert!(run.output().contains("tracked"));
    assert_eq!(before, example.sources());
}

#[test]
fn narrow_binary_declaration_can_share_a_tree_with_tracked_source() {
    let example = Example::materialise("config-narrow-assets");
    let run = example.run(&["config", "untracked", "add", "sim/src/**/*.bin"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let build = example.run(&["build"]);
    assert_eq!(build.code, 0, "{}", build.output());
    assert!(!build.output().contains("AKR-G026"), "{}", build.output());
    example.write_file("missing-code.akr", "intent \"Check missing source is still visible.\"\nscope [ path \"sim/src/missing-code/**\" ]\n");
    let run = example.run(&[
        "propose",
        "sys.work.missing-code",
        "--kind",
        "work",
        "--from",
        "missing-code.akr",
    ]);
    assert_eq!(run.code, 0, "{}", run.output());
    let run = example.run(&["check", "--lenient"]);
    assert!(
        run.output().contains("AKR-G023"),
        "binary glob hid missing source: {}",
        run.output()
    );
}

#[test]
fn replacement_rejects_records_invalid_paths_and_project_renames_atomically() {
    let example = Example::materialise("config-malformed");
    let original = example.read_file(".akr/project.akr");
    let before = example.sources();
    for text in [
        format!("{original}\nuntracked {{ path \"path src/**\" }}\n"),
        format!("{original}\nrecord sys.work.injected/1 : work {{ intent \"Injected record\" }}\n"),
        original.replace("project save-your-skin", "project renamed"),
    ] {
        example.write_file("replacement.akr", &text);
        let run = example.run(&["config", "set", "--from", "replacement.akr"]);
        assert_eq!(run.code, 1, "{}", run.output());
        assert_eq!(before, example.sources());
    }
}

#[test]
fn removing_one_untracked_declaration_preserves_the_rest_of_the_configuration() {
    let example = Example::materialise("config-untracked");
    let original = example.read_file(".akr/project.akr");
    example.write_file(
        ".akr/project.akr",
        &format!("{original}\n# Deliberately excluded assets.\nuntracked {{\n    path \"human/**\"\n    path \"assets/binary/**\"\n}}\n"),
    );
    let before = example.sources();
    let run = example.run(&["config", "untracked", "remove", "human/**"]);
    assert_eq!(run.code, 0, "{}", run.output());
    let text = example.read_file(".akr/project.akr");
    assert!(!text.contains("human/**"), "{text}");
    assert!(text.contains("assets/binary/**"), "{text}");
    assert!(text.contains("Deliberately excluded assets."), "{text}");
    assert!(text.contains("namespace sys"), "{text}");
    assert!(text.contains("defaults"), "{text}");
    let after: std::collections::BTreeMap<_, _> = example.sources().into_iter().collect();
    for (path, bytes) in before {
        if path != ".akr/project.akr" {
            assert_eq!(after.get(&path), Some(&bytes), "changed {path}");
        }
    }
    assert!(run.output().contains("akr build"));
    let show = example.run(&["config", "show"]);
    assert_eq!(show.code, 0, "{}", show.output());
    assert_eq!(show.stdout, text);
}

#[test]
fn configuration_replacement_refuses_to_remove_a_used_namespace() {
    let example = Example::materialise("config-invalid");
    let original = example.read_file(".akr/project.akr");
    let replacement = original
        .lines()
        .filter(|line| !line.starts_with("namespace sys "))
        .collect::<Vec<_>>()
        .join("\n");
    example.write_file("replacement.akr", &replacement);
    let before = example.sources();
    let run = example.run(&["config", "set", "--from", "replacement.akr"]);
    assert_eq!(run.code, 1, "{}", run.output());
    assert!(
        run.output().contains("nothing was written"),
        "{}",
        run.output()
    );
    assert_eq!(before, example.sources());
}

#[test]
fn untracked_add_is_idempotent_and_configuration_replacement_is_supported() {
    let example = Example::materialise("config-set");
    let args = ["config", "untracked", "add", "binary/**"];
    let run = example.run(&args);
    assert_eq!(run.code, 0, "{}", run.output());
    let once = example.read_file(".akr/project.akr");
    assert_eq!(example.run(&args).code, 0);
    assert_eq!(once, example.read_file(".akr/project.akr"));
    example.write_file("replacement.akr", &once.replace("binary/**", "textures/**"));
    let run = example.run(&["config", "set", "--from", "replacement.akr"]);
    assert_eq!(run.code, 0, "{}", run.output());
    assert!(
        example
            .read_file(".akr/project.akr")
            .contains("textures/**")
    );
}
