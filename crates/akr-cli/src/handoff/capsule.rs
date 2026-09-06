//! The two inherited levels: what is true of the repository, and of the session.
//!
//! # Why a project capsule is derived rather than written
//!
//! Everything in it is already visible in the checkout — the manifests say the toolchain,
//! the directory layout is the module map, `project.akr` declares the namespaces. An agent
//! discovering it runs `rg --files`, reads `Cargo.toml`, finds `tests/`, works out where
//! the benchmarks are, and reads the instruction file. That is a deterministic question
//! with a deterministic answer, so it is derived once and inherited, and the derivation is
//! keyed by a digest so it changes exactly when the project's shape does.
//!
//! It is deliberately shallow. A capsule that tried to summarise what the code *does*
//! would be a model's judgement wearing a fact's clothes, and the whole subsystem is built
//! on keeping those apart. What it holds is where things are and how to run them.
//!
//! # Why a session capsule holds the request verbatim
//!
//! A parent narrowing a job for a subagent is legitimate — that is what delegation is
//! for. Silently *replacing* the user's request with the parent's reading of it is not,
//! and the two look identical from inside the child. So the session holds the original
//! request, every packet inherits it, and a narrowed assignment renders underneath it:
//! the narrowing stays visible to the agent it was done to.

use super::{PreparedCommand, block, commands_json, list, optional, strings, text};
use crate::session::Session;
use akr_core::hash::Sha256;
use akr_core::json::Value;
use std::path::Path;

/// What is true of this repository, whoever is working in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// `pc-` plus twelve hex characters of a digest over the derived content.
    pub id: String,
    /// The project name from `project.akr`.
    pub name: String,
    /// Detected toolchains, most specific first.
    pub toolchain: Vec<String>,
    /// Top-level areas and what each is for.
    pub module_map: Vec<String>,
    /// How to build, test, lint and benchmark.
    pub commands: Vec<PreparedCommand>,
    /// Declared AKR namespaces.
    pub namespaces: Vec<String>,
    /// Paths that are authority.
    pub authoritative: Vec<String>,
    /// Paths that are build output and must never be hand-edited.
    pub generated: Vec<String>,
    /// Instruction files an agent in this repository is expected to have read.
    pub instructions: Vec<String>,
    /// Architectural boundaries, supplied rather than derived.
    pub boundaries: Vec<String>,
}

/// What is true of this session, whoever it delegates to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCapsule {
    /// `sx-` plus twelve hex characters.
    pub id: String,
    /// The date it was opened.
    pub opened_at: String,
    /// The project capsule it was opened against.
    pub project: String,
    /// **The user's request, verbatim.** Inherited by every packet, never replaced.
    pub request: String,
    /// The governing planning key, if the work has one.
    pub goal: Option<String>,
    /// Records that govern the work.
    pub governing: Vec<String>,
    /// The workspace as it stood when the session opened.
    pub workspace: super::snapshot::Fingerprint,
    /// The session head, rendered once so no child re-derives it.
    pub session_head: String,
    /// Constraints the work must respect.
    pub constraints: Vec<String>,
    /// Commands run this session, with what they produced.
    pub commands: Vec<PreparedCommand>,
    /// Measurements established this session.
    pub baselines: Vec<String>,
    /// Evidence records already written.
    pub evidence: Vec<String>,
    /// Artefacts already produced.
    pub artifacts: Vec<String>,
}

// ---------------------------------------------------------------------------------------
// deriving the project capsule
// ---------------------------------------------------------------------------------------

/// Manifests, the toolchain each implies, and the commands that go with it.
///
/// A table rather than a plugin point. Getting this wrong costs an agent one wrong command
/// and a correction; getting it configurable costs every project a configuration file.
const TOOLCHAINS: &[(&str, &str, &[&str])] = &[
    (
        "Cargo.toml",
        "rust (cargo)",
        &[
            "cargo build",
            "cargo test",
            "cargo clippy --all-targets",
            "cargo fmt --all",
        ],
    ),
    (
        "package.json",
        "node (npm)",
        &["npm install", "npm test", "npm run build"],
    ),
    ("pyproject.toml", "python", &["pytest"]),
    ("setup.py", "python", &["pytest"]),
    ("go.mod", "go", &["go build ./...", "go test ./..."]),
    ("CMakeLists.txt", "c/c++ (cmake)", &["cmake --build build"]),
    ("Makefile", "make", &["make", "make test"]),
];

/// Directories whose name says what they hold.
const ROLES: &[(&str, &str)] = &[
    ("src", "source"),
    ("lib", "source"),
    ("crates", "source (workspace members)"),
    ("packages", "source (workspace members)"),
    ("apps", "source (workspace members)"),
    ("tests", "tests"),
    ("test", "tests"),
    ("benches", "benchmarks"),
    ("bench", "benchmarks"),
    ("examples", "worked examples"),
    ("fixtures", "test fixtures"),
    ("docs", "documentation"),
    ("doc", "documentation"),
    ("spec", "specification"),
    ("sources", "registered source library (immutable)"),
    ("scripts", "tooling"),
    ("tools", "tooling"),
];

/// Directories that are never part of a module map.
const NOISE: &[&str] = &[
    "target",
    "node_modules",
    "build",
    "dist",
    "vendor",
    "__pycache__",
    "venv",
    ".venv",
];

fn is_noise(name: &str) -> bool {
    name.starts_with('.') || NOISE.contains(&name)
}

/// Reads the workspace and answers the questions every agent otherwise asks itself.
#[must_use]
pub fn derive_project(session: &Session) -> Project {
    let root = &session.root;

    let mut toolchain = Vec::new();
    let mut commands = Vec::new();
    for (manifest, name, defaults) in TOOLCHAINS {
        if !root.join(manifest).is_file() {
            continue;
        }
        let label = if *manifest == "Cargo.toml" {
            edition_of(&root.join(manifest))
                .map_or_else(|| (*name).to_owned(), |e| format!("{name}, edition {e}"))
        } else {
            (*name).to_owned()
        };
        if !toolchain.contains(&label) {
            toolchain.push(label);
        }
        // Only the first-detected toolchain contributes commands. A repository with both
        // a Cargo.toml and a Makefile has one of them in charge, and offering an agent
        // eight commands from two build systems is worse than offering four from one.
        if commands.is_empty() {
            commands.extend(defaults.iter().map(|c| PreparedCommand::parse(c)));
        }
    }

    let mut module_map = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !is_noise(name))
            .collect();
        names.sort();
        for name in names {
            let role = ROLES
                .iter()
                .find(|(directory, _)| *directory == name)
                .map(|(_, role)| *role);
            // A workspace-member directory is worth one level of descent: "crates" tells
            // an agent nothing, "crates/akr-core, akr-cli, akr-mcp" tells it the shape.
            let members = matches!(name.as_str(), "crates" | "packages" | "apps")
                .then(|| children_of(&root.join(&name)))
                .filter(|members| !members.is_empty());
            match (role, members) {
                (Some(role), Some(members)) => {
                    module_map.push(format!("{name}/  {role}: {}", members.join(", ")));
                }
                (Some(role), None) => module_map.push(format!("{name}/  {role}")),
                (None, _) => module_map.push(format!("{name}/")),
            }
        }
    }

    let namespaces: Vec<String> = session
        .ledger
        .project
        .namespaces
        .iter()
        .map(ToString::to_string)
        .collect();

    let present = |candidates: &[&str]| -> Vec<String> {
        candidates
            .iter()
            .filter(|path| {
                // A candidate may carry a parenthetical note for the reader, so the
                // existence check takes the path off the front rather than the whole
                // string: "sources/** (immutable)" is a real directory plus a caption.
                let trimmed = path
                    .split_whitespace()
                    .next()
                    .unwrap_or(path)
                    .trim_end_matches("/**")
                    .trim_end_matches('/');
                root.join(trimmed).exists()
            })
            .map(|path| (*path).to_owned())
            .collect()
    };

    let mut capsule = Project {
        id: String::new(),
        name: session.ledger.project.name.clone(),
        toolchain,
        module_map,
        commands,
        namespaces,
        authoritative: present(&[".akr/records/**", "spec/**", "sources/** (immutable)"]),
        generated: present(&["docs/generated/**", ".akr/cache/**", ".akr/akr.lock"]),
        instructions: present(&["AGENTS.md", "CLAUDE.md", "GEMINI.md", ".cursorrules"]),
        boundaries: Vec::new(),
    };
    capsule.id = format!("pc-{}", &capsule.digest()[..12]);
    capsule
}

/// The `edition = "…"` of a Cargo manifest, by line scan.
///
/// A line scan rather than a TOML parser because the workspace takes no dependencies
/// (`docs/13` §4) and this is the only field wanted. A manifest that does not yield one
/// simply reports the toolchain without an edition.
fn edition_of(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("edition"))
        .and_then(|rest| rest.trim_start().strip_prefix('='))
        .and_then(|rest| {
            let value = rest.trim().trim_matches('"');
            (!value.is_empty() && value.chars().all(|c| c.is_ascii_digit()))
                .then(|| value.to_owned())
        })
}

fn children_of(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !is_noise(name))
        .collect();
    names.sort();
    names
}

impl Project {
    /// A digest over everything derived, so the id changes exactly when the shape does.
    fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.name.as_bytes());
        for group in [
            &self.toolchain,
            &self.module_map,
            &self.namespaces,
            &self.authoritative,
            &self.generated,
            &self.instructions,
            &self.boundaries,
        ] {
            for item in group {
                hasher.update(item.as_bytes());
                hasher.update(b"\x1e");
            }
            hasher.update(b"\x1d");
        }
        for command in &self.commands {
            hasher.update(command.command.as_bytes());
            hasher.update(b"\x1e");
        }
        hasher.finish().to_hex()
    }

    /// The rendering a packet embeds.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!("PROJECT {} ({})\n", self.name, self.id);
        if !self.toolchain.is_empty() {
            text.push_str(&format!("  toolchain    {}\n", self.toolchain.join(", ")));
        }
        if !self.namespaces.is_empty() {
            text.push_str(&format!("  namespaces   {}\n", self.namespaces.join(", ")));
        }
        text.push_str(&block("  layout", &self.module_map, "    "));
        if !self.commands.is_empty() {
            text.push_str("  commands\n");
            for command in &self.commands {
                text.push_str(&format!("    {}\n", command.line()));
            }
        }
        text.push_str(&block("  authoritative", &self.authoritative, "    "));
        text.push_str(&block(
            "  generated (never hand-edit)",
            &self.generated,
            "    ",
        ));
        text.push_str(&block("  instructions", &self.instructions, "    "));
        text.push_str(&block("  boundaries", &self.boundaries, "    "));
        text
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("capsule", Value::string(self.id.clone())),
            ("kind", Value::string("project")),
            ("name", Value::string(self.name.clone())),
            ("toolchain", strings(&self.toolchain)),
            ("module_map", strings(&self.module_map)),
            ("commands", commands_json(&self.commands)),
            ("namespaces", strings(&self.namespaces)),
            ("authoritative", strings(&self.authoritative)),
            ("generated", strings(&self.generated)),
            ("instructions", strings(&self.instructions)),
            ("boundaries", strings(&self.boundaries)),
        ])
    }

    /// Reads one back.
    ///
    /// # Errors
    /// A message when the document is not a project capsule.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let id = value
            .get("capsule")
            .and_then(Value::as_str)
            .ok_or("no `capsule` field")?
            .to_owned();
        Ok(Self {
            id,
            name: text(value, "name"),
            toolchain: list(Some(value), "toolchain"),
            module_map: list(Some(value), "module_map"),
            commands: PreparedCommand::list_from_json(Some(value), "commands"),
            namespaces: list(Some(value), "namespaces"),
            authoritative: list(Some(value), "authoritative"),
            generated: list(Some(value), "generated"),
            instructions: list(Some(value), "instructions"),
            boundaries: list(Some(value), "boundaries"),
        })
    }
}

impl SessionCapsule {
    /// Opens a session against `project`, for `request`.
    #[must_use]
    pub fn new(
        request: &str,
        project: &Project,
        workspace: super::snapshot::Fingerprint,
        opened_at: akr_core::model::Date,
        ordinal: usize,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(request.as_bytes());
        hasher.update(project.id.as_bytes());
        hasher.update(workspace.head.as_deref().unwrap_or("").as_bytes());
        hasher.update(opened_at.to_string().as_bytes());
        hasher.update(ordinal.to_string().as_bytes());
        Self {
            id: format!("sx-{}", &hasher.finish().to_hex()[..12]),
            opened_at: opened_at.to_string(),
            project: project.id.clone(),
            request: request.to_owned(),
            goal: None,
            governing: Vec::new(),
            workspace,
            session_head: String::new(),
            constraints: Vec::new(),
            commands: Vec::new(),
            baselines: Vec::new(),
            evidence: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    /// Whether anything has been established beyond opening the session.
    #[must_use]
    pub fn has_established(&self) -> bool {
        !self.commands.is_empty()
            || !self.baselines.is_empty()
            || !self.evidence.is_empty()
            || !self.artifacts.is_empty()
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("capsule", Value::string(self.id.clone())),
            ("kind", Value::string("session")),
            ("opened_at", Value::string(self.opened_at.clone())),
            ("project", Value::string(self.project.clone())),
            ("request", Value::string(self.request.clone())),
            (
                "goal",
                self.goal
                    .as_ref()
                    .map_or(Value::Null, |g| Value::string(g.clone())),
            ),
            ("governing", strings(&self.governing)),
            ("workspace", self.workspace.to_json()),
            ("session_head", Value::string(self.session_head.clone())),
            ("constraints", strings(&self.constraints)),
            ("commands", commands_json(&self.commands)),
            ("baselines", strings(&self.baselines)),
            ("evidence", strings(&self.evidence)),
            ("artifacts", strings(&self.artifacts)),
        ])
    }

    /// Reads one back.
    ///
    /// # Errors
    /// A message when the document is not a session capsule.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let id = value
            .get("capsule")
            .and_then(Value::as_str)
            .ok_or("no `capsule` field")?
            .to_owned();
        Ok(Self {
            id,
            opened_at: text(value, "opened_at"),
            project: text(value, "project"),
            request: text(value, "request"),
            goal: optional(value, "goal"),
            governing: list(Some(value), "governing"),
            workspace: super::snapshot::Fingerprint::from_json(
                value.get("workspace").unwrap_or(&Value::Null),
            ),
            session_head: text(value, "session_head"),
            constraints: list(Some(value), "constraints"),
            commands: PreparedCommand::list_from_json(Some(value), "commands"),
            baselines: strings_of(value, "baselines"),
            evidence: strings_of(value, "evidence"),
            artifacts: strings_of(value, "artifacts"),
        })
    }
}

fn strings_of(value: &Value, field: &str) -> Vec<String> {
    list(Some(value), field)
}
