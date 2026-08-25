//! CST to model: turns parsed text into [`crate::model`] types.
//!
//! This is stage B. Slot names, kinds, enum members and value shapes are checked here
//! against `spec/tables/vocabulary.json`, which is why the parser does not check them —
//! "slot `cadence` takes a string" beats "unexpected token" every time.

use super::cst::{Block, BodyItem, File, Item, Slot, Value};
use crate::diagnostics::{Code, Diagnostic, Label, Severity, SlotRef, Span, Subject, codes as c};
use crate::model::{
    Acceptance, Check, CheckMethod, Claim, Commit, ContentSlot, ContentValue, Date, Disposition,
    Glob, Kind, Ledger, LogicalKey, Outcome, Project, Record, Reference, Relation, RevisionId,
    ScopeTerm, Segment, Source, SourceKind, SourceRole, State,
};
use std::collections::BTreeMap;

/// The outcome of lowering.
#[derive(Debug, Default)]
pub struct Lowered {
    /// Records that lowered successfully.
    pub records: Vec<Record>,
    /// The project declaration, when the file was a project file.
    pub project: Option<Project>,
    /// Everything that went wrong.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lowers a parsed file, tagging every record with `path` for V-003.
#[must_use]
pub fn lower_file(file: &File, path: &str) -> Lowered {
    let mut out = Lowered::default();
    let mut namespaces = Vec::new();
    for item in &file.items {
        match item {
            Item::Record(record) => {
                let mut ctx = Ctx {
                    diagnostics: Vec::new(),
                    id: None,
                };
                if let Some(lowered) = ctx.record(record, path) {
                    out.records.push(lowered);
                }
                out.diagnostics.extend(ctx.diagnostics);
            }
            Item::Namespace(namespace) => namespaces.push(namespace.name.clone()),
            Item::Block(_) => {}
        }
    }
    if !namespaces.is_empty() {
        let refs: Vec<&str> = namespaces.iter().map(String::as_str).collect();
        out.project = Some(Project::new(&file.project, &refs));
    }
    out
}

/// Lowers several files into one ledger.
#[must_use]
pub fn lower_all(files: &[(String, File)]) -> (Ledger, Vec<Diagnostic>) {
    let mut project = Project::default();
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    for (path, file) in files {
        let lowered = lower_file(file, path);
        if let Some(declared) = lowered.project {
            project = declared;
        }
        records.extend(lowered.records);
        diagnostics.extend(lowered.diagnostics);
    }
    let mut ledger = Ledger::new(project);
    ledger.extend(records);
    (ledger, diagnostics)
}

struct Ctx {
    diagnostics: Vec<Diagnostic>,
    id: Option<RevisionId>,
}

impl Ctx {
    fn error(&mut self, code: Code, span: Span, message: impl Into<String>, slot: Option<SlotRef>) {
        let subject = match (&self.id, slot) {
            (Some(id), Some(slot)) => Subject::Slot(id.clone(), slot),
            (Some(id), None) => Subject::Revision(id.clone()),
            _ => Subject::Ledger,
        };
        self.diagnostics.push(Diagnostic {
            code,
            severity: Severity::Error,
            rule: None,
            message: message.into(),
            primary: Label {
                subject,
                span: Some(span),
                message: None,
            },
            notes: Vec::new(),
            help: None,
        });
    }

    /// The same diagnostic with a remedy attached.
    fn error_with_help(
        &mut self,
        code: Code,
        span: Span,
        message: impl Into<String>,
        slot: Option<SlotRef>,
        help: impl Into<String>,
    ) {
        self.error(code, span, message, slot);
        if let Some(last) = self.diagnostics.last_mut() {
            last.help = Some(help.into());
        }
    }

    fn record(&mut self, node: &super::cst::Record, path: &str) -> Option<Record> {
        let key = match LogicalKey::parse(&node.key) {
            Ok(key) => key,
            Err(error) => {
                self.error(c::P042, node.key_span, error.to_string(), None);
                return None;
            }
        };
        let id = RevisionId::new(key, node.revision);
        self.id = Some(id.clone());

        let Some(kind) = Kind::from_name(&node.kind) else {
            self.error(
                c::T003,
                node.kind_span,
                format!("{} is not a record kind", node.kind),
                None,
            );
            return None;
        };

        let mut record = Record {
            id,
            kind,
            title: String::new(),
            state: kind.class().initial()[0],
            scope: Vec::new(),
            topic: None,
            content: BTreeMap::new(),
            claims: Vec::new(),
            retired_claims: Vec::new(),
            acceptance: None,
            dispositions: Vec::new(),
            relations: BTreeMap::new(),
            acknowledged: false,
            author: None,
            created_at: None,
            sources: Vec::new(),
            file: Some(path.to_owned()),
        };
        // Duplicate slots and block heads are caught at parse (AKR-P031, AKR-P032).
        for item in &node.body {
            match item {
                BodyItem::Slot(slot) => self.slot(&mut record, slot, kind),
                BodyItem::Block(block) => self.block(&mut record, block),
            }
        }
        Some(record)
    }

    fn slot(&mut self, record: &mut Record, slot: &Slot, kind: Kind) {
        let span = slot.value.span();
        match slot.name.as_str() {
            "title" => {
                if let Some(text) = self.text(&slot.value, "title") {
                    record.title = text;
                }
            }
            // Naming the kind's own states is the whole remedy: the word an author reaches
            // for is usually a real lifecycle word from somewhere else — `accepted` for a
            // decision the user settled — and the refusal by itself sends them to
            // `knowledge.explain` for a list the diagnostic already knows
            // (`saveyourskin.papercut.i-tried-to-record-a-user-settled-decision-with`).
            // A word that is a real state but wrong for this kind is V-007's to refuse,
            // and it already names the legal set. This arm is the other case — a word
            // that is no state at all — and it used to name nothing, so an author who
            // reached for `accepted` on a decision was sent to `knowledge.explain` for a
            // list this diagnostic already had
            // (`saveyourskin.papercut.i-tried-to-record-a-user-settled-decision-with`).
            "state" => match State::from_name(&self.word(&slot.value)) {
                Some(state) => record.state = state,
                None => self.error_with_help(
                    c::T012,
                    span,
                    format!("{} is not a lifecycle state", slot.value.render_inline()),
                    Some(SlotRef::State),
                    format!("a {} is one of {}", kind.name(), state_list(kind)),
                ),
            },
            "scope" => record.scope = self.scope(&slot.value),
            "topic" => match Segment::new(&self.word(&slot.value)) {
                Ok(segment) => record.topic = Some(segment),
                Err(error) => self.error(c::P041, span, error.to_string(), Some(SlotRef::Topic)),
            },
            "acknowledged" => record.acknowledged = self.word(&slot.value) == "true",
            "author" => record.author = self.text(&slot.value, "author"),
            "created_at" => record.created_at = self.date(&slot.value),
            "retired_claims" => {
                record.retired_claims = self
                    .array(&slot.value)
                    .iter()
                    .filter_map(|v| Segment::new(&self.word(v)).ok())
                    .collect();
            }
            name => {
                if let Some(relation) = Relation::from_name(name) {
                    let targets: Vec<Reference> = self
                        .array(&slot.value)
                        .iter()
                        .filter_map(|v| self.reference(v))
                        .collect();
                    record.relations.insert(relation, targets);
                } else if let Some(content) = ContentSlot::from_name(name) {
                    if let Some(value) = self.content_value(kind, content, &slot.value) {
                        record.content.insert(content, value);
                    }
                } else {
                    self.error(
                        c::T002,
                        slot.name_span,
                        format!("{kind} has no slot `{name}`"),
                        None,
                    );
                }
            }
        }
    }

    fn block(&mut self, record: &mut Record, block: &Block) {
        match block.name.as_str() {
            "claim" => {
                let Some(anchor) = self.head_segment(block) else {
                    return;
                };
                let text = self.block_text(block, "text").unwrap_or_default();
                let supported_by = self
                    .block_value(block, "supported_by")
                    .map(|v| {
                        self.array(v)
                            .iter()
                            .filter_map(|x| self.reference(x))
                            .collect()
                    })
                    .unwrap_or_default();
                record.claims.push(Claim {
                    anchor,
                    text,
                    supported_by,
                });
            }
            "acceptance" => {
                let mut acceptance = Acceptance::default();
                for item in &block.body {
                    if let BodyItem::Block(check) = item
                        && check.name == "check"
                    {
                        if let Some(check) = self.check(check) {
                            acceptance.checks.push(check);
                        }
                    } else {
                        self.error(
                            c::T007,
                            item.span(),
                            "`check` blocks appear only inside `acceptance`",
                            None,
                        );
                    }
                }
                record.acceptance = Some(acceptance);
            }
            "source" => {
                let kind = match self.block_text(block, "kind").as_deref() {
                    Some("legacy") => SourceKind::Legacy,
                    Some("external") => SourceKind::External,
                    Some("internal") => SourceKind::Internal,
                    _ => {
                        self.error(
                            c::T012,
                            block.span,
                            "`source.kind` must be legacy, external or internal",
                            None,
                        );
                        return;
                    }
                };
                // A citation into the registered library needs all four coordinates or
                // none: a half-written range would resolve to a passage nobody chose.
                let start_byte = self.block_integer(block, "start_byte");
                let end_byte = self.block_integer(block, "end_byte");
                let start_line = self.block_integer(block, "start_line");
                let end_line = self.block_integer(block, "end_line");
                let range = match (start_byte, end_byte, start_line, end_line) {
                    (Some(start_byte), Some(end_byte), Some(start_line), Some(end_line)) => {
                        Some(crate::model::SourceRange {
                            start_byte,
                            end_byte,
                            start_line: u32::try_from(start_line).unwrap_or(0),
                            end_line: u32::try_from(end_line).unwrap_or(0),
                            excerpt_hash: self.block_text(block, "excerpt_hash"),
                        })
                    }
                    (None, None, None, None) => None,
                    _ => {
                        self.error(
                            c::T012,
                            block.span,
                            "a `source` range needs start_byte, end_byte, start_line and \
                             end_line together, or none of them",
                            None,
                        );
                        None
                    }
                };
                record.sources.push(Source {
                    kind,
                    role: match self.block_text(block, "role").as_deref() {
                        None => None,
                        Some(value) => match SourceRole::from_str(value) {
                            Some(role) => Some(role),
                            None => {
                                self.error(
                                    c::T012,
                                    block.span,
                                    "`role` must be origin, rationale, evidence, constraint or example",
                                    None,
                                );
                                None
                            }
                        },
                    },
                    path: self.block_text(block, "path"),
                    url: self.block_text(block, "url"),
                    excerpt: self.block_text(block, "excerpt"),
                    document: self.block_text(block, "document"),
                    range,
                    use_note: self.block_text(block, "use"),
                });
            }
            "disposition" => {
                let Some(head) = block.head.as_ref().and_then(|h| self.reference(h)) else {
                    self.error(
                        c::P034,
                        block.span,
                        "`disposition` requires a reference",
                        None,
                    );
                    return;
                };
                let outcome = match self.block_text(block, "outcome").as_deref() {
                    Some("carried_forward") => Outcome::CarriedForward,
                    Some("completed_elsewhere") => Outcome::CompletedElsewhere,
                    Some("intentionally_dropped") => Outcome::IntentionallyDropped,
                    Some("still_required_separately") => Outcome::StillRequiredSeparately,
                    _ => {
                        self.error(
                            c::T012,
                            block.span,
                            "`outcome` must be one of the four disposition outcomes",
                            None,
                        );
                        return;
                    }
                };
                let into = self
                    .block_value(block, "into")
                    .and_then(|v| self.reference(v));
                record.dispositions.push(Disposition {
                    target: head,
                    outcome,
                    into,
                    note: self.block_text(block, "note"),
                });
            }
            other => self.error(
                c::T004,
                block.name_span,
                format!("{other} is not a block"),
                None,
            ),
        }
    }

    fn check(&mut self, block: &Block) -> Option<Check> {
        let id = self.head_segment(block)?;
        let method = match self.block_text(block, "method").as_deref() {
            Some("manual") => CheckMethod::Manual,
            Some("command") => CheckMethod::Command,
            Some("observation") => CheckMethod::Observation,
            _ => {
                self.error(
                    c::T012,
                    block.span,
                    "`method` must be manual, command or observation",
                    None,
                );
                return None;
            }
        };
        Some(Check {
            id,
            statement: self.block_text(block, "statement").unwrap_or_default(),
            method,
            command: self.block_text(block, "command"),
            verified_by: self
                .block_value(block, "verified_by")
                .map(|v| {
                    self.array(v)
                        .iter()
                        .filter_map(|x| self.reference(x))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    // -- value helpers ------------------------------------------------------------

    fn head_segment(&mut self, block: &Block) -> Option<Segment> {
        let head = block.head.as_ref()?;
        match Segment::new(&self.word(head)) {
            Ok(segment) => Some(segment),
            Err(error) => {
                self.error(c::P041, head.span(), error.to_string(), None);
                None
            }
        }
    }

    fn block_value<'b>(&self, block: &'b Block, name: &str) -> Option<&'b Value> {
        block.body.iter().find_map(|item| match item {
            BodyItem::Slot(slot) if slot.name == name => Some(&slot.value),
            _ => None,
        })
    }

    /// A non-negative integer slot of a block, or `None` when it is absent.
    fn block_integer(&mut self, block: &Block, name: &str) -> Option<u64> {
        let value = self.block_value(block, name)?;
        let rendered = match value {
            Value::Str(text, _) | Value::Word(text, _) | Value::Scalar(text, _) => text.clone(),
            other => other.render_inline(),
        };
        match rendered.parse::<u64>() {
            Ok(number) => Some(number),
            Err(_) => {
                self.error(
                    c::T013,
                    value.span(),
                    format!("slot `{name}` expects a non-negative integer, found {rendered}"),
                    None,
                );
                None
            }
        }
    }

    fn block_text(&mut self, block: &Block, name: &str) -> Option<String> {
        let value = self.block_value(block, name)?;
        match value {
            Value::Str(text, _) | Value::Prose(text, _) | Value::Word(text, _) => {
                Some(text.clone())
            }
            other => Some(other.render_inline()),
        }
    }

    fn word(&self, value: &Value) -> String {
        match value {
            Value::Word(w, _) | Value::Scalar(w, _) => w.clone(),
            other => other.render_inline(),
        }
    }

    fn text(&mut self, value: &Value, slot: &str) -> Option<String> {
        match value {
            Value::Str(text, _) | Value::Prose(text, _) => Some(text.clone()),
            other => {
                self.error(
                    c::T013,
                    other.span(),
                    format!(
                        "slot `{slot}` expects a string, found {}",
                        other.render_inline()
                    ),
                    None,
                );
                None
            }
        }
    }

    fn date(&mut self, value: &Value) -> Option<Date> {
        match value {
            Value::Scalar(text, span) => match Date::parse(text) {
                Ok(date) => Some(date),
                Err(error) => {
                    self.error(c::P022, *span, error.to_string(), None);
                    None
                }
            },
            other => {
                self.error(
                    c::T013,
                    other.span(),
                    format!("expected a date, found {}", other.render_inline()),
                    None,
                );
                None
            }
        }
    }

    fn reference(&mut self, value: &Value) -> Option<Reference> {
        match value {
            Value::Ref(body, span) => match Reference::parse(body) {
                Ok(reference) => Some(reference),
                Err(error) => {
                    self.error(c::P043, *span, error.to_string(), None);
                    None
                }
            },
            other => {
                self.error(
                    c::T013,
                    other.span(),
                    format!("expected a reference, found {}", other.render_inline()),
                    None,
                );
                None
            }
        }
    }

    fn array<'b>(&self, value: &'b Value) -> Vec<&'b Value> {
        match value {
            Value::Array(items, _) => items.iter().collect(),
            single => vec![single],
        }
    }

    fn scope(&mut self, value: &Value) -> Vec<ScopeTerm> {
        self.array(value)
            .iter()
            .filter_map(|term| match term {
                Value::Word(w, _) if w == "all" => Some(ScopeTerm::All),
                Value::Prefixed(word, inner, span) => match word.as_str() {
                    "ref" => self.reference(inner).map(ScopeTerm::Ref),
                    "path" => match inner.as_ref() {
                        Value::Str(glob, _) => Some(ScopeTerm::Path(Glob::new(glob))),
                        _ => None,
                    },
                    _ => {
                        self.error(
                            c::T032,
                            *span,
                            "scope term must be `all`, `ref @key`, or `path \"glob\"`",
                            Some(SlotRef::Scope),
                        );
                        None
                    }
                },
                other => {
                    self.error(
                        c::T032,
                        other.span(),
                        "scope term must be `all`, `ref @key`, or `path \"glob\"`",
                        Some(SlotRef::Scope),
                    );
                    None
                }
            })
            .collect()
    }

    fn content_value(
        &mut self,
        kind: Kind,
        slot: ContentSlot,
        value: &Value,
    ) -> Option<ContentValue> {
        let span = value.span();
        Some(match slot {
            ContentSlot::ObservedAt | ContentSlot::AsOf => match value {
                Value::Commit(hex, _) => match Commit::new(hex) {
                    Ok(commit) => ContentValue::Commit(commit),
                    Err(error) => {
                        self.error(c::P021, span, error.to_string(), None);
                        return None;
                    }
                },
                other => {
                    self.error(
                        c::T013,
                        span,
                        format!(
                            "slot `{slot}` expects a commit, found {}",
                            other.render_inline()
                        ),
                        Some(SlotRef::Content(slot)),
                    );
                    return None;
                }
            },
            ContentSlot::ReviewAfter | ContentSlot::Target => ContentValue::Date(self.date(value)?),
            ContentSlot::Watches => ContentValue::Globs(
                self.array(value)
                    .iter()
                    .filter_map(|v| match v {
                        Value::Str(glob, _) => Some(Glob::new(glob)),
                        _ => None,
                    })
                    .collect(),
            ),
            ContentSlot::Exceptions => ContentValue::Refs(
                self.array(value)
                    .iter()
                    .filter_map(|v| self.reference(v))
                    .collect(),
            ),
            ContentSlot::Aliases | ContentSlot::Collated => ContentValue::Strings(
                self.array(value)
                    .iter()
                    .filter_map(|v| match v {
                        Value::Str(text, _) => Some(text.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            ContentSlot::Method | ContentSlot::Result | ContentSlot::Confidence => {
                let candidate = self.word(value);
                match Segment::new(&candidate) {
                    Ok(segment)
                        if kind
                            .content_enum_values(slot)
                            .is_none_or(|values| values.contains(&segment.as_str())) =>
                    {
                        ContentValue::Enum(segment)
                    }
                    Ok(_) => {
                        let expected = kind.content_enum_values(slot).unwrap_or_default();
                        let record = self
                            .id
                            .as_ref()
                            .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
                        self.error(
                            c::T012,
                            span,
                            format!(
                                "record {record} has invalid `{slot}` value `{candidate}`; expected one of: {}",
                                expected.join(", ")
                            ),
                            Some(SlotRef::Content(slot)),
                        );
                        return None;
                    }
                    Err(error) => {
                        let record = self
                            .id
                            .as_ref()
                            .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
                        self.error(
                            c::T012,
                            span,
                            format!(
                                "record {record} has invalid `{slot}` value `{candidate}`: {error}"
                            ),
                            Some(SlotRef::Content(slot)),
                        );
                        return None;
                    }
                }
            }
            _ => match value {
                Value::Prose(text, _) => ContentValue::Prose(text.clone()),
                Value::Str(text, _) => ContentValue::Text(text.clone()),
                other => ContentValue::Text(other.render_inline()),
            },
        })
    }
}

/// A kind's lifecycle states, as the `state` refusal names them.
fn state_list(kind: Kind) -> String {
    kind.class()
        .states()
        .iter()
        .map(|state| state.name().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}
