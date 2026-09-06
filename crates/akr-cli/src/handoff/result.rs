//! What a child hands back, and what a parent learns from several of them.
//!
//! # Why a result is a packet too
//!
//! A parent does not need the child's transcript. "I started by looking at…, then I ran…,
//! the command produced…, I also checked…" is administrative narration with almost no
//! value upstream, and it arrives thousands of tokens at a time. What the parent needs is
//! findings, evidence, what changed, what is still uncertain, and — the part everyone
//! forgets — what the child actually looked at.
//!
//! # Coverage is the field that pays for itself
//!
//! An agent spends ten minutes understanding a subsystem, returns three paragraphs, and
//! its orientation evaporates. The next agent starts over. Worse, five agents sent to
//! "review the project" tend to converge on the same obvious doorway, so the fifth costs
//! as much as the first and adds nothing.
//!
//! [`Coverage`] is the cheap fix: every child says what it read, searched, ran and
//! deliberately did not open, and [`aggregate`] rolls those up across a session. The
//! parent then delegates the next wave against evidence — *nobody has opened
//! `rescale.rs`* — rather than against a hunch about what has been done.
//!
//! `not_examined` is as load-bearing as `read`. A file nobody names is ambiguous: it might
//! have been ruled out in a second or never noticed. A file a child names as unexamined is
//! a fact, and a fact is delegable.

use super::{PreparedCommand, block, commands_json, list, optional, strings, text};
use akr_core::hash::Sha256;
use akr_core::json::Value;
use akr_core::model::Date;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// What one agent actually looked at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Files read, optionally with a line range: `src/chroma.rs:1-470`.
    pub read: Vec<String>,
    /// Queries run, so the next agent does not repeat one that found nothing.
    pub searched: Vec<String>,
    /// Commands, benchmarks or suites run.
    pub tested: Vec<String>,
    /// What was in scope and deliberately not opened.
    pub not_examined: Vec<String>,
}

impl Coverage {
    /// Whether anything was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.read.is_empty()
            && self.searched.is_empty()
            && self.tested.is_empty()
            && self.not_examined.is_empty()
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("read", strings(&self.read)),
            ("searched", strings(&self.searched)),
            ("tested", strings(&self.tested)),
            ("not_examined", strings(&self.not_examined)),
        ])
    }

    /// Reads it back.
    #[must_use]
    pub fn from_json(value: Option<&Value>) -> Self {
        Self {
            read: list(value, "read"),
            searched: list(value, "searched"),
            tested: list(value, "tested"),
            not_examined: list(value, "not_examined"),
        }
    }

    /// The rendering.
    #[must_use]
    pub fn render(&self, indent: &str) -> String {
        let inner = format!("{indent}  ");
        let mut text = String::new();
        text.push_str(&block(&format!("{indent}read"), &self.read, &inner));
        text.push_str(&block(&format!("{indent}searched"), &self.searched, &inner));
        text.push_str(&block(&format!("{indent}tested"), &self.tested, &inner));
        text.push_str(&block(
            &format!("{indent}not examined"),
            &self.not_examined,
            &inner,
        ));
        text
    }
}

/// What a child hands back for one assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// `rs-` plus twelve hex characters.
    pub id: String,
    /// The packet this answers.
    pub packet: String,
    /// The date it was filed.
    pub filed_at: String,
    /// Who filed it.
    pub filed_by: Option<String>,
    /// Findings, most significant first. The child decides the order.
    pub findings: Vec<String>,
    /// Evidence for them: source locations, records, artefacts.
    pub evidence: Vec<String>,
    /// What the child changed, if anything. Empty means it changed nothing.
    pub changes: Vec<String>,
    /// What it is not sure about. Absent uncertainty is itself a claim.
    pub uncertainties: Vec<String>,
    /// What it would do next, or would have somebody else do.
    pub follow_up: Vec<String>,
    /// Commands it ran, with what they produced.
    pub commands: Vec<PreparedCommand>,
    /// What it looked at.
    pub coverage: Coverage,
}

impl Report {
    /// A new report answering `packet`.
    #[must_use]
    pub fn new(packet: &str, filed_at: Date, ordinal: usize) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(packet.as_bytes());
        hasher.update(filed_at.to_string().as_bytes());
        hasher.update(ordinal.to_string().as_bytes());
        Self {
            id: format!("rs-{}", &hasher.finish().to_hex()[..12]),
            packet: packet.to_owned(),
            filed_at: filed_at.to_string(),
            filed_by: None,
            findings: Vec::new(),
            evidence: Vec::new(),
            changes: Vec::new(),
            uncertainties: Vec::new(),
            follow_up: Vec::new(),
            commands: Vec::new(),
            coverage: Coverage::default(),
        }
    }

    /// Whether an id names a report.
    #[must_use]
    pub fn is_report_id(id: &str) -> bool {
        id.starts_with("rs-")
    }

    /// The rendering a parent reads.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!("RESULT {} for {}\n", self.id, self.packet);
        text.push_str(&format!(
            "filed        {}{}\n",
            self.filed_at,
            self.filed_by
                .as_ref()
                .map_or_else(String::new, |by| format!(" by {by}"))
        ));
        text.push('\n');
        text.push_str(&block("FINDINGS", &self.findings, "  "));
        text.push_str(&block("EVIDENCE", &self.evidence, "  "));
        // "none" rather than an omitted section: an agent that changed nothing and one
        // that forgot to say are different reports, and only one of them is reassuring.
        if self.changes.is_empty() {
            text.push_str("CHANGES\n  none\n");
        } else {
            text.push_str(&block("CHANGES", &self.changes, "  "));
        }
        text.push_str(&block("UNCERTAINTIES", &self.uncertainties, "  "));
        text.push_str(&block("FOLLOW-UP", &self.follow_up, "  "));
        if !self.commands.is_empty() {
            text.push_str("COMMANDS\n");
            for command in &self.commands {
                text.push_str(&format!("  {}\n", command.line()));
            }
        }
        if self.coverage.is_empty() {
            // Said rather than left blank: a result with no coverage is the one that makes
            // the next delegation a guess, and the child is the only one who can fix it.
            text.push_str(
                "COVERAGE\n  none recorded — the next wave cannot see what you already \
                 looked at\n",
            );
        } else {
            text.push_str("COVERAGE\n");
            text.push_str(&self.coverage.render("  "));
        }
        text
    }

    /// The stored form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::object(vec![
            ("format", Value::string(super::packet::FORMAT)),
            ("result", Value::string(self.id.clone())),
            ("packet", Value::string(self.packet.clone())),
            ("filed_at", Value::string(self.filed_at.clone())),
            (
                "filed_by",
                self.filed_by
                    .as_ref()
                    .map_or(Value::Null, |by| Value::string(by.clone())),
            ),
            ("findings", strings(&self.findings)),
            ("evidence", strings(&self.evidence)),
            ("changes", strings(&self.changes)),
            ("uncertainties", strings(&self.uncertainties)),
            ("follow_up", strings(&self.follow_up)),
            ("commands", commands_json(&self.commands)),
            ("coverage", self.coverage.to_json()),
        ])
    }

    /// Reads one back.
    ///
    /// # Errors
    /// A message when the document is not a result.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let id = value
            .get("result")
            .and_then(Value::as_str)
            .ok_or("no `result` field")?
            .to_owned();
        Ok(Self {
            id,
            packet: value
                .get("packet")
                .and_then(Value::as_str)
                .ok_or("no `packet` field")?
                .to_owned(),
            filed_at: text(value, "filed_at"),
            filed_by: optional(value, "filed_by"),
            findings: list(Some(value), "findings"),
            evidence: list(Some(value), "evidence"),
            changes: list(Some(value), "changes"),
            uncertainties: list(Some(value), "uncertainties"),
            follow_up: list(Some(value), "follow_up"),
            commands: PreparedCommand::list_from_json(Some(value), "commands"),
            coverage: Coverage::from_json(value.get("coverage")),
        })
    }
}

/// Every report filed in this workspace, oldest id first.
#[must_use]
pub fn all(root: &Path) -> Vec<Report> {
    super::packet::ids_with(root, Report::is_report_id)
        .into_iter()
        .filter_map(|id| {
            super::packet::read_json(root, &id)
                .ok()
                .and_then(|value| Report::from_json(&value).ok())
        })
        .collect()
}

/// What a set of results, taken together, has and has not covered.
#[derive(Debug, Clone, Default)]
pub struct Aggregate {
    /// Path (with any range stripped) -> the packets that read it, and how.
    pub read: BTreeMap<String, Vec<String>>,
    /// Query -> the packets that ran it.
    pub searched: BTreeMap<String, Vec<String>>,
    /// Command or suite -> the packets that ran it.
    pub tested: BTreeMap<String, Vec<String>>,
    /// Paths a child named as deliberately unopened, and which nobody else opened.
    pub untouched: BTreeSet<String>,
    /// Assigned scopes across the session, so a reader knows what the universe was.
    pub scopes: BTreeSet<String>,
    /// Packets in the session that have filed no result.
    pub outstanding: Vec<String>,
}

/// Rolls up coverage for a set of packets and the results answering them.
///
/// `untouched` is the useful half and the subtle one: a path counts as untouched when some
/// child said it did not open it and *no* child says it read it. A path nobody mentions at
/// all is not reported — silence is not evidence of absence, and claiming it would make the
/// aggregate lie in exactly the direction that costs a wasted agent.
#[must_use]
pub fn aggregate(packets: &[super::packet::Packet], reports: &[Report]) -> Aggregate {
    let mut out = Aggregate::default();
    for packet in packets {
        for glob in &packet.scope {
            out.scopes.insert(glob.clone());
        }
        if !reports.iter().any(|report| report.packet == packet.id) {
            out.outstanding.push(packet.id.clone());
        }
    }

    let mut named_unexamined: BTreeSet<String> = BTreeSet::new();
    for report in reports {
        let by = report.packet.clone();
        for entry in &report.coverage.read {
            let path = entry.split(':').next().unwrap_or(entry).to_owned();
            out.read.entry(path).or_default().push(by.clone());
        }
        for entry in &report.coverage.searched {
            out.searched
                .entry(entry.clone())
                .or_default()
                .push(by.clone());
        }
        for entry in &report.coverage.tested {
            out.tested
                .entry(entry.clone())
                .or_default()
                .push(by.clone());
        }
        for entry in &report.coverage.not_examined {
            named_unexamined.insert(entry.split(':').next().unwrap_or(entry).to_owned());
        }
    }
    out.untouched = named_unexamined
        .into_iter()
        .filter(|path| !out.read.contains_key(path))
        .collect();
    out
}

impl Aggregate {
    /// The rendering.
    #[must_use]
    pub fn render(&self) -> String {
        let mut text = String::new();
        if !self.scopes.is_empty() {
            text.push_str("assigned scope\n");
            for glob in &self.scopes {
                text.push_str(&format!("  {glob}\n"));
            }
        }
        if self.read.is_empty() && self.searched.is_empty() && self.tested.is_empty() {
            text.push_str("\nnothing has reported coverage yet\n");
        }
        for (label, rows) in [
            ("read", &self.read),
            ("searched", &self.searched),
            ("tested", &self.tested),
        ] {
            if rows.is_empty() {
                continue;
            }
            text.push_str(&format!("\n{label}\n"));
            for (item, by) in rows {
                text.push_str(&format!("  {item}  ({})\n", by.join(", ")));
            }
        }
        if !self.untouched.is_empty() {
            text.push_str("\nnobody has examined\n");
            for path in &self.untouched {
                text.push_str(&format!("  {path}\n"));
            }
        }
        if !self.outstanding.is_empty() {
            text.push_str("\nno result filed yet\n");
            for id in &self.outstanding {
                text.push_str(&format!("  {id}\n"));
            }
        }
        text
    }

    /// The structured form.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let rows = |map: &BTreeMap<String, Vec<String>>| {
            Value::array(
                map.iter()
                    .map(|(item, by)| {
                        Value::object(vec![
                            ("item", Value::string(item.clone())),
                            (
                                "by",
                                Value::array(by.iter().cloned().map(Value::string).collect()),
                            ),
                        ])
                    })
                    .collect(),
            )
        };
        Value::object(vec![
            (
                "scope",
                Value::array(self.scopes.iter().cloned().map(Value::string).collect()),
            ),
            ("read", rows(&self.read)),
            ("searched", rows(&self.searched)),
            ("tested", rows(&self.tested)),
            (
                "untouched",
                Value::array(self.untouched.iter().cloned().map(Value::string).collect()),
            ),
            (
                "outstanding",
                Value::array(
                    self.outstanding
                        .iter()
                        .cloned()
                        .map(Value::string)
                        .collect(),
                ),
            ),
        ])
    }
}
