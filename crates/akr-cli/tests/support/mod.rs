//! Materialising `examples/save-your-skin/` as a real repository the binary can run in.
//!
//! The worked example's history is fictional: `MANIFEST.md` §4 freezes five invented
//! commit hashes, and the records cite them in `observed_at` and `as_of`. A real
//! repository necessarily produces different hashes, so the example cannot be run against
//! git as it stands — every `observed_at` would be a stranded commit.
//!
//! This module replays the manifest's five commits into a throwaway repository, lays the
//! example's workspace on top with the invented hashes rewritten to the real ones, and
//! regenerates the two derived artefacts that the rewrite invalidates: the lock (whose
//! seals hash the record text) and the views (whose banners carry the source-graph hash).
//! Everything else — which records go stale, why, what goes at risk, along which
//! relation, what every command prints about it — is the example unchanged, and is what
//! the transcripts assert.
//!
//! [`Example::normalise`] maps the real hashes back to the manifest's, so a transcript can
//! go on quoting `e806b3f5` and still be checked against a real run.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// One worked example's synthetic history and the invented hashes its records cite.
pub struct Fixture {
    /// The example directory under `examples/`.
    pub name: &'static str,
    /// The commits of its manifest's history table, in order.
    pub steps: &'static [(&'static str, &'static [(&'static str, &'static str)])],
    /// The invented hashes, in the same order as `steps`.
    pub commits: &'static [&'static str],
    /// The frozen "today".
    pub today: &'static str,
}

/// `examples/save-your-skin/`.
pub const SAVE_YOUR_SKIN: Fixture = Fixture {
    name: "save-your-skin",
    steps: SAVE_YOUR_SKIN_STEPS,
    commits: SAVE_YOUR_SKIN_COMMITS,
    today: "2026-08-03",
};

/// `examples/sys-tandem/`, whose history table is `MANIFEST.md` §2.
pub const SYS_TANDEM: Fixture = Fixture {
    name: "sys-tandem",
    steps: SYS_TANDEM_STEPS,
    commits: SYS_TANDEM_COMMITS,
    today: "2026-08-04",
};

const SYS_TANDEM_STEPS: &[(&str, &[(&str, &str)])] = &[
    (
        "C1 skeleton",
        &[
            ("src/lib.rs", "// the simulator\n"),
            ("src/engine/mod.rs", "// simulator-side engine modules\n"),
            (
                "SYSEngine/crates/sys_game_bridge/src/lib.rs",
                "// the seam\n",
            ),
            (
                "SYSEngine/crates/sys_present/src/lib.rs",
                "// presentation\n",
            ),
            ("SYSEngine/crates/sys_sound/src/lib.rs", "// sound\n"),
            ("SYSEngine/docs/BUILD-STATUS.md", "# verified tree state\n"),
            ("AGENTS.md", "# rulings\n"),
            // The one legacy document this project actually holds (`MANIFEST.md` §4,
            // committed verbatim). `tandem.work.roadmap-import` tracks its disposition
            // with `source { kind legacy }`, so the migration audit checks it exists at
            // HEAD — which, until the tracking record completes and it is archived, it
            // does. The plan documents of §6 are cited but deliberately absent (§8), and
            // the tracker-anchored audit leaves those bare citations alone.
            (
                "examples/sys-tandem/legacy/2026-08-03-engine-simulator-tandem-roadmap.md",
                include_str!(
                    "../../../../examples/sys-tandem/legacy/2026-08-03-engine-simulator-tandem-roadmap.md"
                ),
            ),
        ],
    ),
    (
        "C2 engine docs",
        &[("SYSEngine/docs/RENDERING-NEXT.md", "# renderer boundary\n")],
    ),
    (
        "C3 the assessment and the rulings",
        &[
            (
                "SYSEngine/docs/2026-08-02-mid-engine-assessment.md",
                "# the gap register\n",
            ),
            ("AGENTS.md", "# rulings, revised\n"),
        ],
    ),
    (
        "C4 M1-M5 land",
        &[
            ("src/lib.rs", "// the simulator, with the day close\n"),
            (
                "SYSEngine/crates/sys_present/src/lib.rs",
                "// presentation, four more record families\n",
            ),
        ],
    ),
    (
        "C5 a bridge follow-up",
        &[(
            "SYSEngine/crates/sys_game_bridge/src/lib.rs",
            "// the seam, extended\n",
        )],
    ),
];

const SYS_TANDEM_COMMITS: &[&str] = &[
    "a3f19d0c7b5e284610f8c2a94db3e0761fc58a29",
    "4e82b6d1a09f37c5e28d4bf160a739c8b2e05d17",
    "9d40e1b7c85a326f0b1de94738ca5602f81b7d4a",
    "5cb1a4f0d2e79836ba4c17e05d3f96428a7bc10e",
    "b7e3092d6a1f48c5039be2714da86f05c93e1b6d",
];

/// The five commits of `MANIFEST.md` §4, with the files each one writes.
const SAVE_YOUR_SKIN_STEPS: &[(&str, &[(&str, &str)])] = &[
    (
        "C1 initial skeleton",
        &[
            ("sim/src/lib.rs", "// simulator\n"),
            ("sim/src/step.rs", "// fixed timestep\n"),
            ("sim/src/project/mod.rs", "// projection pass\n"),
            ("sim/src/tick/mod.rs", "// tick loop\n"),
            ("lege/src/lib.rs", "// viewer\n"),
            ("lege/src/render/mod.rs", "// render graph\n"),
            ("lege/src/light/mod.rs", "// lighting\n"),
            ("content/day/light/dawn.toml", "# dawn keys\n"),
            ("tools/audit.rs", "// asset audit\n"),
            // The pre-AKR pile. `sys.work.legacy-roadmap-import` cites it with
            // `source { kind legacy }`, and the migration audit (AKR-M022) checks it
            // exists — in the fiction it does, until step 6 of `docs/12` archives it.
            (
                "docs/legacy/ROADMAP.md",
                include_str!("../../../../examples/save-your-skin/docs/legacy/ROADMAP.md"),
            ),
        ],
    ),
    (
        "C2 projection and step",
        &[
            ("sim/src/project/mod.rs", "// projection pass, revised\n"),
            ("sim/src/step.rs", "// fixed timestep, 8 ms\n"),
        ],
    ),
    (
        "C3 renderer boundary",
        &[
            ("lege/src/lib.rs", "// viewer, frame snapshot boundary\n"),
            ("sim/src/step.rs", "// fixed timestep, accumulator\n"),
        ],
    ),
    (
        "C4 projection rework and determinism suite",
        &[
            ("sim/src/project/mod.rs", "// projection pass, reworked\n"),
            ("sim/tests/determinism.rs", "// 512-seed sweep\n"),
        ],
    ),
    (
        "C5 render graph and views",
        &[
            ("lege/src/render/mod.rs", "// render graph, extracted\n"),
            ("docs/generated/ROADMAP.md", "<!-- generated -->\n"),
        ],
    ),
];

/// The manifest's invented hashes, in table order.
pub const SAVE_YOUR_SKIN_COMMITS: &[&str] = &[
    "3f0a1c9d5b7e2648a0d4f1b8c36e9752ad014b6f",
    "7c41d0ba92e6f37518a3cd406b5e2f91d8074a63",
    "b2e58f1406c7a9d3e41b60258fa3d7c6195e0b48",
    "5d9c2a70e31f8b46c07d5924ab6e3f1074c9d285",
    "e806b3f54a2d7091c5e13b8a26f490dc7b135e64",
];

/// A materialised worked example: a repository, a workspace, and a hash translation.
pub struct Example {
    root: PathBuf,
    /// The real commit for each entry of [`MANIFEST_COMMITS`], in the same order.
    commits: Vec<String>,
    /// `(real, manifest)` substitutions applied to output before comparison.
    substitutions: Vec<(String, String)>,
    /// The frozen "today" this example is evaluated at.
    today: &'static str,
    /// A copy of the generated views, so `git checkout <view>` can be honoured.
    snapshot: PathBuf,
}

impl Example {
    /// Builds the repository, lays down the workspace, and regenerates lock and views.
    ///
    /// # Panics
    /// If git is unavailable, or if the example does not build — either is a broken
    /// checkout rather than a test failure worth reporting per-assertion.
    pub fn materialise(name: &str) -> Self {
        Self::of(&SAVE_YOUR_SKIN, name)
    }

    /// The same, for a named fixture.
    ///
    /// # Panics
    /// As [`Example::materialise`].
    pub fn of(fixture: &Fixture, name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("akr-p6-{name}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp directory");

        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.name", "AKR Test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "gc.auto", "0"]);

        let mut commits = Vec::new();
        for (day, (message, writes)) in fixture.steps.iter().enumerate() {
            for (path, contents) in *writes {
                write(&root.join(path), contents);
            }
            let stamp = format!("2026-01-{:02}T12:00:00+00:00", day + 1);
            git(&root, &["add", "-A"]);
            let output = Command::new("git")
                .args(["commit", "--quiet", "-m", message])
                .current_dir(&root)
                .env("GIT_AUTHOR_DATE", &stamp)
                .env("GIT_COMMITTER_DATE", &stamp)
                .output()
                .expect("git runs");
            assert!(output.status.success(), "git commit failed for {message}");
            commits.push(git(&root, &["rev-parse", "HEAD"]).trim().to_owned());
        }

        let source = examples_dir().join(fixture.name);
        copy_dir(&source.join(".akr"), &root.join(".akr"));
        std::fs::create_dir_all(root.join("docs/generated")).expect("views directory");
        if let Ok(views) = std::fs::read_dir(source.join("docs/generated")) {
            for entry in views {
                let entry = entry.expect("view").path();
                if entry.extension().is_some_and(|e| e == "md") {
                    let name = entry.file_name().expect("file name");
                    std::fs::copy(&entry, root.join("docs/generated").join(name))
                        .expect("copy view");
                }
            }
        }

        // Repoint every cited commit at the real one. This changes the record bytes, so
        // the seals and the source-graph hash change with them; both are regenerated
        // below rather than patched, because regenerating is what the tool does.
        let mut substitutions = Vec::new();
        for (index, invented) in fixture.commits.iter().enumerate() {
            let real = &commits[index];
            rewrite_tree(&root.join(".akr"), invented, real);
            substitutions.push((real.clone(), (*invented).to_owned()));
            substitutions.push((real[..8].to_owned(), invented[..8].to_owned()));
        }

        let example = Self {
            root,
            commits,
            substitutions,
            today: fixture.today,
            snapshot: PathBuf::new(),
        };
        example.run(&["lock"]);
        example.run(&["build"]);

        // The one derived value the substitutions above cannot cover: the source graph is
        // a hash of the rewritten bytes, so it is read back and mapped to the hash the
        // committed example carries.
        let mut example = example;
        let committed = source_graph_of(
            &std::fs::read_to_string(source.join(".akr/akr.lock")).expect("committed lock"),
        );
        let rebuilt = source_graph_of(
            &std::fs::read_to_string(example.root.join(".akr/akr.lock")).expect("rebuilt lock"),
        );
        example.substitutions.push((rebuilt, committed));

        // A snapshot of the built tree, so a transcript's `git checkout <path>` can be
        // honoured without adding a commit. Committing the workspace would move HEAD off
        // C5 and invalidate every hash the transcripts quote.
        let snapshot = example.root.with_extension("snapshot");
        let _ = std::fs::remove_dir_all(&snapshot);
        copy_dir(&example.root.join("docs/generated"), &snapshot);
        example.snapshot = snapshot;
        example
    }

    /// Runs a transcript's non-`akr` shell line: `sed`, `git checkout`, `cd`.
    ///
    /// Only the forms the transcripts actually use are supported, and anything else is a
    /// hard failure rather than a silent skip — a transcript that grew a new directive
    /// should fail loudly until this understands it.
    pub fn shell(&self, command: &str) {
        let words: Vec<&str> = command.split_whitespace().collect();
        match words.first().copied() {
            Some("cd") => {}
            // A deferred command — a write operation, or `search` — that this phase does
            // not run. Its effect on the workspace is deferred with it, which is why the
            // transcripts put every deferred command after the last checked block.
            Some("akr") => {}
            Some("git") if words.get(1) == Some(&"checkout") => {
                for path in &words[2..] {
                    let name = Path::new(path).file_name().expect("a file name");
                    std::fs::copy(self.snapshot.join(name), self.root.join(path))
                        .expect("restore from snapshot");
                }
            }
            // `echo $?` is a transcript idiom for showing the exit status; the harness
            // asserts the status directly, so running it would only print a stray 0.
            Some("echo") => {}
            Some("sed") => self.sed(command),
            _ => panic!("the transcript harness does not understand `{command}`"),
        }
    }

    /// Applies a transcript's `sed -i 's/^<from>/<to>/' <file>` line, without a shell.
    ///
    /// This used to be `sh -c`, which is not a thing every supported platform has: the
    /// suite failed on Windows with "program not found" for a substitution the harness
    /// could perfectly well do itself. The whole corpus contains one directive, of one
    /// shape -- a `^`-anchored literal replaced on the single line that starts with it --
    /// so that shape is what is implemented, and anything else fails loudly rather than
    /// being approximated.
    fn sed(&self, command: &str) {
        let rest = command
            .strip_prefix("sed -i ")
            .unwrap_or_else(|| panic!("only `sed -i` is supported: {command}"));
        let (script, file) = rest
            .rsplit_once(' ')
            .unwrap_or_else(|| panic!("a sed directive names a file: {command}"));
        let script = script
            .trim()
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or_else(|| panic!("the sed script must be single-quoted: {command}"));

        let body = script
            .strip_prefix("s/")
            .and_then(|s| s.strip_suffix('/'))
            .unwrap_or_else(|| panic!("only `s/from/to/` is supported: {command}"));
        // `/` inside the pattern is backslash-escaped, as it is in the transcript. Split
        // on the one delimiter that is not, then unescape both halves.
        let mut halves = Vec::new();
        let mut current = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => current.push(chars.next().unwrap_or('\\')),
                '/' => halves.push(std::mem::take(&mut current)),
                other => current.push(other),
            }
        }
        halves.push(current);
        assert_eq!(halves.len(), 2, "expected one `s/from/to/` pair: {command}");
        let (from, to) = (&halves[0], &halves[1]);
        let from = from
            .strip_prefix('^')
            .unwrap_or_else(|| panic!("only `^`-anchored patterns are supported: {command}"));

        let path = self.root.join(file.trim());
        let text = std::fs::read_to_string(&path).expect("the file the directive names");
        let mut replaced = 0usize;
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let (body, ending) = match line.strip_suffix('\n') {
                Some(body) => (body, "\n"),
                None => (line, ""),
            };
            if let Some(tail) = body.strip_prefix(from) {
                replaced += 1;
                out.push_str(to);
                out.push_str(tail);
            } else {
                out.push_str(body);
            }
            out.push_str(ending);
        }
        assert!(replaced > 0, "sed directive matched nothing: {command}");
        std::fs::write(&path, out).expect("rewrite the file the directive names");
    }

    /// Runs `git` in the workspace and asserts it succeeded.
    ///
    /// [`Self::shell`] deliberately understands only the handful of directives the
    /// transcripts use; the change-protocol tests need the git *index*, which no
    /// transcript exercises, so they get their own door rather than widening that one.
    pub fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Where the workspace is.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The real commit standing in for the manifest's Cn, 1-based.
    pub fn commit(&self, n: usize) -> &str {
        &self.commits[n - 1]
    }

    /// Runs `akr` in the workspace with the frozen `--today`, returning stdout, stderr and
    /// the exit status.
    ///
    /// # Panics
    /// If the binary cannot be executed.
    pub fn run(&self, args: &[&str]) -> Run {
        let mut command = Command::new(env!("CARGO_BIN_EXE_akr"));
        command.arg("--today").arg(self.today);
        command.args(args).current_dir(&self.root);
        let output = command.output().expect("the akr binary runs");
        Run {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        }
    }

    /// Runs `akr` with no executable named `git` available.
    ///
    /// Used to prove that a command's fast path is structurally Git-free rather than
    /// merely quick on the test machine.
    pub fn run_without_git(&self, args: &[&str]) -> Run {
        let empty_path = self.root.join("no-executables");
        std::fs::create_dir_all(&empty_path).expect("empty PATH directory");
        let mut command = Command::new(env!("CARGO_BIN_EXE_akr"));
        command.arg("--today").arg(self.today);
        command
            .args(args)
            .current_dir(&self.root)
            .env("PATH", empty_path);
        let output = command.output().expect("the akr binary runs");
        Run {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        }
    }

    /// A digest of every source file under `.akr`, for atomicity assertions.
    ///
    /// `docs/07-cli.md` §4 promises that a refused write leaves the working tree
    /// byte-identical. The only way to check a promise about *every* file is to hash every
    /// file, so this returns the whole map rather than one file's hash.
    pub fn sources(&self) -> Vec<(String, String)> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries {
                let path = entry.expect("entry").path();
                // The disposable cache is not a source. D-019 makes it never
                // authoritative, always rebuildable and read by nothing outside
                // `akr-core`, so a refused write that left an index rebuild or the write
                // pipeline's lock file behind has still left every *source* byte-identical
                // — which is the promise docs/07 §4 makes and this helper exists to check.
                if path.file_name().is_some_and(|name| name == "cache") && path.is_dir() {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, base, out);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    let name = path
                        .strip_prefix(base)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned();
                    out.push((name, akr_core::hash::sha256_hex(&bytes)));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root.join(".akr"), &self.root, &mut out);
        out.sort();
        out
    }

    /// Overwrites a file inside the workspace, for tests that need a specific shape.
    pub fn write_file(&self, relative: &str, contents: &str) {
        write(&self.root.join(relative), contents);
    }

    /// Reads a file inside the workspace.
    pub fn read_file(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root.join(relative)).expect("readable")
    }

    /// Rewrites real hashes back to the manifest's, so a transcript stays readable.
    pub fn normalise(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (real, invented) in &self.substitutions {
            out = out.replace(real.as_str(), invented.as_str());
        }
        out
    }

    /// Translates a transcript's manifest hash into the real one, for arguments.
    pub fn denormalise(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (real, invented) in &self.substitutions {
            // Longest first: the eight-character forms are prefixes of the full ones, and
            // the full ones are pushed first, so a plain reverse pass is enough.
            out = out.replace(invented.as_str(), real.as_str());
        }
        out
    }
}

impl Drop for Example {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.snapshot);
    }
}

/// One invocation's result.
pub struct Run {
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// The process exit status.
    pub code: i32,
}

impl Run {
    /// Everything the command printed, in the order a terminal would show it.
    pub fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Where the worked examples live.
pub fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Where the committed `save-your-skin` example lives.
pub fn example_source() -> PathBuf {
    examples_dir().join("save-your-skin")
}

/// Where the transcripts live.
pub fn transcript_dir() -> PathBuf {
    example_source().join("transcripts")
}

fn source_graph_of(lock: &str) -> String {
    lock.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("source_graph \"")
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .expect("a lock carries a source graph")
        .to_owned()
}

fn rewrite_tree(dir: &Path, from: &str, to: &str) {
    for entry in std::fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rewrite_tree(&path, from, to);
        } else if let Ok(text) = std::fs::read_to_string(&path)
            && text.contains(from)
        {
            std::fs::write(&path, text.replace(from, to)).expect("rewrite");
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("destination");
    for entry in std::fs::read_dir(from).expect("source directory") {
        let path = entry.expect("entry").path();
        let name = path.file_name().expect("file name");
        if path.is_dir() {
            copy_dir(&path, &to.join(name));
        } else {
            std::fs::copy(&path, to.join(name)).expect("copy");
        }
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent directory");
    }
    std::fs::write(path, contents).expect("write");
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
