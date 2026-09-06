//! The tool declarations of `docs/08-mcp.md` §2, with their JSON schemas.
//!
//! The list is **closed for 0.1** — no `knowledge.query`, no `knowledge.build`, no
//! `knowledge.delete`. §2 gives a reason for each absence and they are all the same
//! reason: a tool is a contract, and every tool that exists is one an agent will lean on.
//! The escape valve is `knowledge.search`, which returns records rather than rows (§6).

use akr_core::json::Value;

/// One tool, as `tools/list` reports it.
pub struct Tool {
    /// The tool name.
    pub name: &'static str,
    /// One line, shown to the agent.
    pub description: &'static str,
    /// Whether the tool changes the workspace.
    ///
    /// Usually that means `.akr/records/`. It also covers the advisor-packet store
    /// under `.agent/handoffs/` (D-040), which is not the ledger but is still a file
    /// this tool creates: `readOnlyHint` and `--surface read` both read this field, and
    /// both would be lying if a tool that wrote a packet reported itself read-only.
    pub writes: bool,
}

/// The catalogue, in §2's order.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "knowledge.search",
        description: "Search the ledger. Ranks; never authorises — nothing enters a context \
                      bundle because it matched a query. A missing or stale disposable index \
                      is refreshed from the loaded ledger before results are returned.",
        writes: false,
    },
    Tool {
        name: "knowledge.start",
        description: "Read the validated project handoff, then orient a new task to nearby planning records.",
        writes: false,
    },
    Tool {
        name: "knowledge.explain",
        description: "Explain a diagnostic code, rule identifier, or record kind.",
        writes: false,
    },
    Tool {
        name: "knowledge.get",
        description: "Retrieve one record by reference. `detail` controls the size: \
                      `summary`, `body` (default: includes acceptance checks), or \
                      `canonical` for the raw AKR source text. Ask for canonical only \
                      when you need the syntax itself; if it truncates, retry with body.",
        writes: false,
    },
    Tool {
        name: "knowledge.context",
        description: "Assemble the deterministic context bundle for a goal: what governs \
                      this work, what it rests on, and what is questionable.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_list",
        description: "List registered source documents and their retention state.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_add",
        description: "Register an immutable source document from a local path.",
        writes: true,
    },
    Tool {
        name: "knowledge.source_search",
        description: "Search the immutable source library — registered outside advice, \
                      audits and reports. Results are NON-AUTHORITATIVE: they say where a \
                      passage is, never that the project adopted it. Punctuation is safe \
                      here; `mode: \"literal\"` verifies an exact substring.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_get",
        description: "Read a passage of a registered source. NON-AUTHORITATIVE. The \
                      default detail is `section`, not the whole document; ask for \
                      `whole` only when the entire report is genuinely wanted.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_verify",
        description: "Verify hashes and retained fragments of every registered source.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_supersede",
        description: "Replace a registered source with a new immutable version.",
        writes: true,
    },
    Tool {
        name: "knowledge.source_status",
        description: "Show one source's availability, references, and retained fragments.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_dependents",
        description: "List exact and lineage record references to a source.",
        writes: false,
    },
    Tool {
        name: "knowledge.source_finalize",
        description: "Retain cited fragments or metadata, then optionally remove the full source.",
        writes: true,
    },
    Tool {
        name: "knowledge.impact",
        description: "What rests on a record, or what a commit range would invalidate. Call \
                      before proposing a supersession.",
        writes: false,
    },
    Tool {
        name: "knowledge.validate",
        description: "Run stages A-D over the ledger as it stands on disk. Call after a \
                      batch of writes and before handing work back to a human.",
        writes: false,
    },
    Tool {
        name: "knowledge.propose",
        description: "Create revision 1 of a new key. An existing key is an error: a \
                      proposal is never silently turned into a revision. Pass `acceptance` \
                      to author its checks — required for a milestone (V-008).",
        writes: true,
    },
    Tool {
        name: "knowledge.revise",
        description: "Create the next revision of an existing key. `base_rev` must equal \
                      the current head, or the call fails with a conflict. An explicit \
                      state lands on the successor; changed sealed content without one \
                      starts proposed for re-acceptance.",
        writes: true,
    },
    Tool {
        name: "knowledge.supersede",
        description: "Replace a record, disposing of every unfinished part_of child. A \
                      different new_key must already be a proposed same-kind record; \
                      propose its body first. Missing children are listed in the error.",
        writes: true,
    },
    Tool {
        name: "knowledge.complete",
        description: "Move a milestone or work record to completed. Every acceptance check \
                      must be satisfied by passing evidence — create it first with \
                      knowledge.evidence_add, then cite it here as a D-009 reference, e.g. \
                      {\"checks\": {\"no-placeholder-assets\": \
                      \"@sys.evidence.asset-audit/1\"}}.",
        writes: true,
    },
    Tool {
        name: "knowledge.evidence_add",
        description: "Record what was observed: result, method, and the commit it was \
                      observed at (defaults to HEAD). Deliberately has no field for what \
                      the evidence verifies (D-016) — cite it from a check's verified_by \
                      or from knowledge.complete.",
        writes: true,
    },
    Tool {
        name: "knowledge.evidence_add_many",
        description: "Record up to 100 evidence observations atomically. The workspace is \
                      opened once and the resulting ledger is validated and written once; \
                      if any item is invalid, none are written.",
        writes: true,
    },
    Tool {
        name: "knowledge.handoff_create",
        description: "Prepare an advisor packet: hand a task to a second model without handing over\
                      the judgement it is being brought in to make. `task` is the user's request\
                      VERBATIM — never a restatement, because a restatement substitutes your\
                      conclusion for the problem. `search_envelope` defaults to the whole project;\
                      narrow it only when the user did. Your own findings go in `worker_notes`,\
                      which the advisor cannot see until knowledge.handoff_reveal.",
        writes: true,
    },
    Tool {
        name: "knowledge.handoff_list",
        description: "List the advisor packets this workspace holds.",
        writes: false,
    },
    Tool {
        name: "knowledge.handoff_open",
        description: "Open an advisor packet: the task verbatim, the workspace and whether it has\
                      drifted, the independent search scope, the session head, governing records,\
                      commands and baselines. Worker notes are withheld — review independently\
                      first, then knowledge.handoff_reveal.",
        writes: false,
    },
    Tool {
        name: "knowledge.handoff_expand",
        description: "Read one section of a packet: task, workspace, project, governing, envelope or\
                      execution. Use it instead of re-opening the whole packet.",
        writes: false,
    },
    Tool {
        name: "knowledge.handoff_reveal",
        description: "Release the packet's worker notes, and record that it happened. Call it after\
                      an independent pass, then compare: what did either side miss?",
        writes: true,
    },
    Tool {
        name: "knowledge.handoff_verify",
        description: "Does the packet still describe this tree? Reports exact or drifted, and names\
                      what moved.",
        writes: false,
    },
    Tool {
        name: "knowledge.handoff_discard",
        description: "Delete an advisor packet. Packets are disposable; nothing durable is lost.",
        writes: true,
    },
    Tool {
        name: "knowledge.papercut",
        description: "Log a small friction hit while working — a tool call that missed \
                      and had to be retried, a confusing setup step, a flaky command, a \
                      stale cache, a misleading error, a non-obvious gotcha. One or two \
                      sentences: what you were doing, what got in the way (a guess at the \
                      cause/fix is a bonus). Do this proactively, in the moment, even \
                      though none of these are blocking — logged together they show where \
                      the project needs sanding down (D-027).",
        writes: true,
    },
];

/// The input schema for a tool, or `None` if the name is unknown.
#[must_use]
pub fn input_schema(name: &str) -> Option<Value> {
    let schema = match name {
        "knowledge.search" => object(
            vec![
                (
                    "query",
                    string(
                        "Natural-language terms to search for. Ordinary multi-word \
                         queries match any term and rank records matching more first.",
                    ),
                ),
                ("kinds", string_array("Restrict to these record kinds.")),
                ("states", string_array("Restrict to these states.")),
                (
                    "limit",
                    integer("Maximum results. Default 20, maximum 100."),
                ),
                (
                    "offset",
                    integer("Zero-based ranked-result offset. Use `next_offset` to continue."),
                ),
            ],
            &["query"],
        ),
        "knowledge.start" => object(
            vec![
                ("task", string("What the agent should start working on.")),
                ("paths", string_array("Path globs the work will touch.")),
                ("budget_tokens", integer("Approximate token budget.")),
            ],
            &["task"],
        ),
        "knowledge.explain" => object(
            vec![(
                "subject",
                string("Diagnostic code, rule id, or record kind to explain."),
            )],
            &["subject"],
        ),
        "knowledge.get" => object(
            vec![
                (
                    "ref",
                    string("A reference in any of the four forms of D-009."),
                ),
                ("history", boolean("Include every revision of the key.")),
                (
                    "relations",
                    boolean("Include inbound and outbound relations."),
                ),
                (
                    "detail",
                    string(
                        "`summary` (identity, state, scope, relation counts, freshness, \
                         source locators), `body` (the default: adds slots, claims, \
                         acceptance checks and full relations) or `canonical` (adds the \
                         raw AKR source text). Ask for `canonical` only when you need \
                         the syntax itself; if it truncates, retry with `body` — \
                         `summary` drops the acceptance block.",
                    ),
                ),
            ],
            &["ref"],
        ),
        "knowledge.context" => object(
            vec![
                (
                    "goal",
                    string(
                        "A live milestone, work item or track. A bare key, @key, or a \
                         pin to its current head is accepted; historical pins, anchors, \
                         and terminal records are retrieval-only.",
                    ),
                ),
                ("paths", string_array("Path globs the work will touch.")),
                ("budget_tokens", integer("Approximate token budget.")),
            ],
            &["goal"],
        ),
        "knowledge.source_list" => object(
            vec![(
                "all_versions",
                boolean("Include superseded source registrations."),
            )],
            &[],
        ),
        "knowledge.source_add" => object(
            vec![
                ("path", string("Local Markdown file to register.")),
                ("id", string("Optional stable source id.")),
                ("title", string("Optional human-readable title.")),
                ("origin", string("external or internal-reference.")),
                ("observed_at", string("Optional observed Git commit.")),
                ("scope", string("Optional project path glob.")),
            ],
            &["path"],
        ),
        "knowledge.source_search" => object(
            vec![
                ("query", string("What to look for. Punctuation is safe.")),
                (
                    "mode",
                    string(
                        "`text` (default) escapes punctuation into ordinary terms, \
                         `literal` verifies an exact substring against the registered \
                         bytes, `fts` passes a raw FTS5 expression through.",
                    ),
                ),
                (
                    "documents",
                    string_array("Restrict to these registered source ids."),
                ),
                (
                    "all_versions",
                    boolean("Include documents a later registration supersedes."),
                ),
                (
                    "limit",
                    integer("Maximum results. Default 10, maximum 100."),
                ),
                (
                    "offset",
                    integer("Zero-based ranked-result offset. Use `next_offset` to continue."),
                ),
            ],
            &["query"],
        ),
        "knowledge.source_get" => object(
            vec![
                (
                    "chunk",
                    string("A chunk id from knowledge.source_search. Exclusive with `id`."),
                ),
                (
                    "id",
                    string("A registered source id. Exclusive with `chunk`."),
                ),
                (
                    "detail",
                    string(
                        "`snippet` (the chunk alone), `section` (the chunk and its \
                         neighbours, the default) or `whole` (the entire document).",
                    ),
                ),
                (
                    "lines",
                    string("A line range `a:b` within the document, with `id`."),
                ),
            ],
            &[],
        ),
        "knowledge.source_verify" => object(Vec::new(), &[]),
        "knowledge.source_supersede" => object(
            vec![
                ("old_id", string("Existing source id.")),
                ("new_path", string("Local replacement Markdown file.")),
                ("new_id", string("Optional replacement source id.")),
            ],
            &["old_id", "new_path"],
        ),
        "knowledge.source_status" => object(vec![("id", string("Source id."))], &["id"]),
        "knowledge.source_dependents" => object(vec![("id", string("Source id."))], &["id"]),
        "knowledge.source_finalize" => object(
            vec![
                ("id", string("Source id.")),
                ("retain", string("cited (default) or metadata.")),
                ("context", string("exact or block (default).")),
                (
                    "remove_file",
                    boolean("Remove the full source after durable retention."),
                ),
                (
                    "dry_run",
                    boolean("Report the plan without changing files."),
                ),
            ],
            &["id"],
        ),
        "knowledge.impact" => object(
            vec![
                (
                    "ref",
                    string("A record reference. Exclusive with git_diff."),
                ),
                (
                    "git_diff",
                    string("A commit range `A..B`, full 40-hex on both ends."),
                ),
                (
                    "depth",
                    integer("Maximum propagation depth. Default unbounded."),
                ),
            ],
            &[],
        ),
        "knowledge.validate" => object(
            vec![
                (
                    "review_clean",
                    boolean("Also fail when the review queue is not empty."),
                ),
                (
                    "limit",
                    integer(
                        "Maximum diagnostics to return. Defaults to 5; use with offset to page.",
                    ),
                ),
                (
                    "offset",
                    integer("Zero-based diagnostic offset. Defaults to 0."),
                ),
            ],
            &[],
        ),
        "knowledge.propose" => object(
            vec![
                (
                    "key",
                    string(
                        "The new logical key, dot-delimited: namespace.topic.slug. The \
                         first segment must be a namespace declared in .akr/project.akr.",
                    ),
                ),
                ("kind", kind_schema()),
                ("title", string("The one-line label.")),
                ("state", string("Override the class's initial state.")),
                ("scope", scope_schema()),
                (
                    "topic",
                    string(
                        "The exclusivity handle, normative kinds only. A topic is one \
                         lowercase ASCII segment containing letters, digits or internal \
                         hyphens; it is not a dot-delimited key.",
                    ),
                ),
                ("slots", slots_schema()),
                ("claims", claims_schema()),
                ("relations", relations_schema()),
                (
                    "acceptance",
                    acceptance_schema("Required to propose a milestone (V-008)."),
                ),
                ("sources", sources_schema()),
                ("acknowledged", acknowledged_schema()),
            ],
            &["key", "kind", "title"],
        ),
        "knowledge.revise" => object(
            vec![
                ("key", string("The key to revise.")),
                ("title", string("Replace the title.")),
                ("state", string("Move along the class's lifecycle.")),
                ("scope", scope_schema()),
                (
                    "topic",
                    string(
                        "Replace the exclusivity handle, normative kinds only. Omit to \
                         keep the head's.",
                    ),
                ),
                ("slots", slots_schema()),
                ("claims", claims_schema()),
                (
                    "retired_claims",
                    string_array("Anchors this revision drops (D-011)."),
                ),
                ("relations", relations_schema()),
                ("dispositions", dispositions_schema()),
                (
                    "acceptance",
                    acceptance_schema(
                        "Replaces the acceptance block. Omit to keep the head's checks.",
                    ),
                ),
                ("sources", sources_schema()),
                ("acknowledged", acknowledged_schema()),
                (
                    "base_rev",
                    integer(
                        "The revision the edit was made against. Must equal the current \
                         head, or the call fails with a conflict.",
                    ),
                ),
            ],
            &["key", "base_rev"],
        ),
        "knowledge.supersede" => object(
            vec![
                ("old_key", string("The key whose head is retired.")),
                (
                    "new_key",
                    string(
                        "The superseding key. Defaults to old_key. A different key must \
                         already exist as a proposed record of the same kind.",
                    ),
                ),
                ("dispositions", dispositions_schema()),
            ],
            &["old_key"],
        ),
        "knowledge.complete" => object(
            vec![
                ("key", string("The milestone or work record to complete.")),
                (
                    "checks",
                    Value::object(vec![
                        ("type", Value::string("object")),
                        (
                            "description",
                            Value::string(
                                "check id -> evidence reference, in a D-009 form: \
                                 {\"no-placeholder-assets\": \
                                 \"@sys.evidence.asset-audit/1\"}. Create the evidence \
                                 with knowledge.evidence_add first.",
                            ),
                        ),
                        (
                            "additionalProperties",
                            Value::object(vec![("type", Value::string("string"))]),
                        ),
                    ]),
                ),
            ],
            &["key"],
        ),
        "knowledge.evidence_add" => evidence_schema(),
        "knowledge.evidence_add_many" => object(
            vec![(
                "evidence",
                Value::object(vec![
                    ("type", Value::string("array")),
                    (
                        "description",
                        Value::string("Evidence records to create in one atomic transaction."),
                    ),
                    ("items", evidence_schema()),
                    ("minItems", Value::integer(1)),
                    ("maxItems", Value::integer(100)),
                ]),
            )],
            &["evidence"],
        ),
        "knowledge.handoff_create" => object(
            vec![
                (
                    "task",
                    string(
                        "The user's request, VERBATIM. Not a restatement, not your \
                         reading of it: an advisor hired for what you did not notice \
                         cannot be handed your noticing as the definition of the task.",
                    ),
                ),
                (
                    "question",
                    string(
                        "What the advisor is specifically asked, when that is narrower than the task.",
                    ),
                ),
                (
                    "by",
                    string("Who prepared the packet: a model or harness name."),
                ),
                (
                    "goal",
                    string("The governing planning key, if the work has one."),
                ),
                (
                    "governing",
                    string_array("Records that govern or constrain the work."),
                ),
                (
                    "search_envelope",
                    string_array(
                        "The advisor's independent search scope, as path globs. Defaults \
                         to the whole project. Narrowing it to what you examined hands \
                         the advisor your blind spot as a boundary.",
                    ),
                ),
                (
                    "commands",
                    string_array("Commands the advisor can run, as `command` or `command=result`."),
                ),
                (
                    "baselines",
                    string_array("Measurements already established."),
                ),
                (
                    "constraints",
                    string_array("Constraints the answer must respect."),
                ),
                (
                    "evidence",
                    string_array("Evidence records already written."),
                ),
                (
                    "artifacts",
                    string_array("Artefacts already produced, by repository path."),
                ),
                (
                    "worker_notes",
                    object(
                        vec![
                            (
                                "hypotheses",
                                string_array("Where you suspect the answer lies."),
                            ),
                            ("examined", string_array("What you read.")),
                            (
                                "not_examined",
                                string_array(
                                    "What you did not read. The half of coverage usually lost.",
                                ),
                            ),
                            ("approaches", string_array("Approaches you would propose.")),
                            (
                                "searches",
                                string_array(
                                    "Searches you ran, so a query that found nothing is not repeated.",
                                ),
                            ),
                        ],
                        &[],
                    ),
                ),
                (
                    "budget_tokens",
                    integer("Approximate token budget for the embedded session head."),
                ),
            ],
            &["task"],
        ),
        "knowledge.handoff_list" => object(Vec::new(), &[]),
        "knowledge.handoff_open" => object(
            vec![("packet", string("The packet id, e.g. ap-7f29d3a10b44."))],
            &["packet"],
        ),
        "knowledge.handoff_expand" => object(
            vec![
                ("packet", string("The packet id.")),
                (
                    "section",
                    string("One of: task, workspace, project, governing, envelope, execution."),
                ),
            ],
            &["packet", "section"],
        ),
        "knowledge.handoff_reveal" => {
            object(vec![("packet", string("The packet id."))], &["packet"])
        }
        "knowledge.handoff_verify" => {
            object(vec![("packet", string("The packet id."))], &["packet"])
        }
        "knowledge.handoff_discard" => {
            object(vec![("packet", string("The packet id."))], &["packet"])
        }
        "knowledge.papercut" => object(
            vec![
                (
                    "message",
                    string(
                        "One or two sentences: what you were doing, what got in the way, \
                         and optionally a guess at the cause or fix.",
                    ),
                ),
                (
                    "agent",
                    string("Who hit it: your model or harness name, e.g. \"claude\"."),
                ),
                (
                    "namespace",
                    string(
                        "Namespace for the key. Never required: the default is where \
                         this project's papercuts already go, falling back to the \
                         namespace carrying the most records.",
                    ),
                ),
                (
                    "about",
                    string(
                        "What the friction was WITH, when that is not this project — a \
                         tool name such as \"akr\". Leave it out for this project's own \
                         code or setup. Use it whenever the thing that got in your way \
                         was the tooling rather than the repository you are working in: \
                         that is how the report reaches whoever maintains the tool.",
                    ),
                ),
            ],
            &["message", "agent"],
        ),
        _ => return None,
    };
    Some(schema)
}

/// The output schema for each tool, or `None` for a name that is not one.
///
/// Every tool answers with a JSON object, so this is one shape rather than twenty-nine.
/// It used to be a `match` naming each tool and returning that same shape from every arm
/// — a list carrying no information, and one that a new tool silently fell off: the seven
/// `knowledge.handoff_*` tools were declared, implemented and schema'd, and still failed
/// `no_tool_can_reach_the_sqlite_cache` for want of a line here. Deriving it from the
/// catalogue removes the drift rather than adding seven more lines to it. `input_schema`
/// stays a per-tool `match`, because there the arms actually differ.
#[must_use]
pub fn output_schema(name: &str) -> Option<Value> {
    TOOLS
        .iter()
        .any(|tool| tool.name == name)
        .then(|| Value::object(vec![("type", Value::string("object"))]))
}

fn evidence_schema() -> Value {
    object(
        vec![
            (
                "key",
                string(
                    "The new evidence key, dot-delimited: namespace.topic.slug, e.g. \
                     sys.evidence.asset-audit.",
                ),
            ),
            (
                "result",
                enumeration("What was observed.", &["pass", "fail", "inconclusive"]),
            ),
            (
                "method",
                enumeration("How it was observed.", &evidence_method_values()),
            ),
            (
                "command",
                string("The exact command that was run, for method command."),
            ),
            (
                "artifact",
                string("A repository path to the artefact, if one exists."),
            ),
            ("summary", string("One line on what was observed.")),
            (
                "observed_at",
                string("The full 40-hex commit the observation was made at. Defaults to HEAD."),
            ),
            (
                "title",
                string("The one-line label. Defaults to the summary or the key."),
            ),
        ],
        &["key", "result", "method"],
    )
}

/// `kind`: the closed enumeration of D-001, with each kind's typed contract in
/// the description — generated from the same tables the type-checker reads, so an agent
/// learns what a kind needs before the first `AKR-T001` rather than from it.
fn kind_schema() -> Value {
    let mut description = String::from("The record kind. Contracts per kind: ");
    for (index, kind) in akr_core::model::Kind::ALL.iter().enumerate() {
        if index > 0 {
            description.push_str("; ");
        }
        let slots: Vec<String> = kind
            .content_slots()
            .iter()
            .map(|spec| {
                format!(
                    "{}: {} {}",
                    spec.slot.name(),
                    spec.slot.value_type(),
                    if spec.required {
                        "required"
                    } else {
                        "optional"
                    }
                )
            })
            .collect();
        description.push_str(kind.name());
        description.push_str(" slots [");
        description.push_str(&slots.join(", "));
        if kind.requires_acceptance() {
            description.push_str(", acceptance required");
        }
        description.push_str("]; relations [");
        description.push_str(
            &akr_core::model::Relation::ALL
                .iter()
                .filter(|relation| relation.domain().accepts(*kind))
                .map(|relation| relation.name())
                .collect::<Vec<_>>()
                .join(", "),
        );
        description.push(']');
    }
    description.push('.');
    Value::object(vec![
        ("type", Value::string("string")),
        ("description", Value::string(description)),
        (
            "enum",
            Value::array(
                akr_core::model::Kind::ALL
                    .iter()
                    .map(|kind| Value::string(kind.name()))
                    .collect(),
            ),
        ),
    ])
}

fn object(properties: Vec<(&str, Value)>, required: &[&str]) -> Value {
    Value::object(vec![
        ("type", Value::string("object")),
        (
            "properties",
            Value::Object(
                properties
                    .into_iter()
                    .map(|(name, schema)| (name.to_owned(), schema))
                    .collect(),
            ),
        ),
        (
            "required",
            Value::array(required.iter().map(|name| Value::string(*name)).collect()),
        ),
        ("additionalProperties", Value::bool(false)),
    ])
}

fn string(description: &str) -> Value {
    Value::object(vec![
        ("type", Value::string("string")),
        ("description", Value::string(description)),
    ])
}

fn integer(description: &str) -> Value {
    Value::object(vec![
        ("type", Value::string("integer")),
        ("description", Value::string(description)),
    ])
}

fn boolean(description: &str) -> Value {
    Value::object(vec![
        ("type", Value::string("boolean")),
        ("description", Value::string(description)),
    ])
}

fn enumeration(description: &str, values: &[&str]) -> Value {
    Value::object(vec![
        ("type", Value::string("string")),
        ("description", Value::string(description)),
        (
            "enum",
            Value::array(values.iter().map(|v| Value::string(*v)).collect()),
        ),
    ])
}

/// The `check.method` values (D-016), taken from `CheckMethod::ALL` rather than
/// duplicated here, so the tool schema can never drift from the ledger's own enum.
fn check_method_values() -> Vec<&'static str> {
    akr_core::model::CheckMethod::ALL
        .iter()
        .map(|method| method.name())
        .collect()
}

/// The `evidence.method` values (D-016), taken from the same per-kind enum table the
/// type-checker reads (`Kind::content_enum_values`), so this can never drift from
/// `spec/tables/vocabulary.json`.
fn evidence_method_values() -> Vec<&'static str> {
    // Nothing in this crate may panic: a panic here would take the schema down with it,
    // and `tools/list` is the first thing a client asks for — a dead server on the
    // handshake is the one failure an agent cannot diagnose. The fallback is the same
    // list, and `evidence_method_values_track_the_ledger` fails loudly if they diverge.
    akr_core::model::Kind::Evidence
        .content_enum_values(akr_core::model::ContentSlot::Method)
        .map_or_else(
            || vec!["manual", "command", "observation"],
            <[&'static str]>::to_vec,
        )
}

/// The `disposition.outcome` values, taken from `Outcome::ALL` — the same table
/// `tools.rs` parses the argument against, so the schema cannot advertise a value the
/// dispatcher would then reject.
fn outcome_values() -> Vec<&'static str> {
    akr_core::model::Outcome::ALL
        .iter()
        .map(|outcome| outcome.name())
        .collect()
}

fn string_array(description: &str) -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        ("description", Value::string(description)),
        (
            "items",
            Value::object(vec![("type", Value::string("string"))]),
        ),
    ])
}

/// `slots` accepts any slot of the record's kind, including `note` on planning kinds
/// (D-026). The kind's own table is what validates it, so the schema stays open here and
/// the refusal — `AKR-T002` for a slot the kind does not have — comes from the grammar.
fn slots_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("object")),
        (
            "description",
            Value::string(format!(
                "Content slots for the selected kind. Exact contracts: {}. Topic is a \
                 separate top-level field and is only valid on normative kinds. Planning \
                 kinds accept optional `note` (D-026): operator commentary, rendered in \
                 views for terminal records. Closed slots: {}.",
                slot_contract_summary(),
                enum_slot_summary(),
            )),
        ),
        ("additionalProperties", Value::bool(true)),
    ])
}

/// Every kind's slots, generated from the vocabulary table used by the type-checker.
fn slot_contract_summary() -> String {
    akr_core::model::Kind::ALL
        .iter()
        .map(|kind| {
            let slots = kind
                .content_slots()
                .iter()
                .map(|spec| {
                    format!(
                        "{}:{}:{}",
                        spec.slot.name(),
                        spec.slot.value_type(),
                        if spec.required {
                            "required"
                        } else {
                            "optional"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} [{}]", kind.name(), slots)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The `acknowledged` marker, on both write tools.
fn acknowledged_schema() -> Value {
    boolean(
        "Marks a declared `contradicts` edge as knowingly tolerated (D-023). Two live \
         records that contradict each other fail V-023 (AKR-R041) unless one of them sets \
         this; acknowledging is a legitimate ledger state, and dropping the relation \
         instead loses the contradiction. Carried forward by knowledge.revise unless the \
         call names it.",
    )
}

/// Every enum content slot in the vocabulary, with its members, as one clause.
///
/// `slots` is one free-form object across thirteen kinds, so the members of a closed slot
/// had nowhere to be advertised and were discovered by having a call refused: `method` on
/// an observation reads as an obvious place for `measurement`, and the allowed words only
/// appeared in the error (`akr.papercut.collated-19-papercuts-from-evidence-intake`, from
/// openarc). There are four such slots in the whole language, so naming all four costs a
/// line and takes the guess out of it.
fn enum_slot_summary() -> String {
    let mut clauses: Vec<String> = Vec::new();
    for kind in akr_core::model::Kind::ALL {
        for spec in kind.content_slots() {
            if let Some(values) = kind.content_enum_values(spec.slot) {
                clauses.push(format!(
                    "{}.{} is one of {}",
                    kind.name(),
                    spec.slot.name(),
                    values.join("|")
                ));
            }
        }
    }
    clauses.join("; ")
}

fn scope_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        (
            "description",
            Value::string(
                "Scope terms. Compact strings are \"all\", a bare path glob such as \
                 \"src/**\", or an \"@key\" reference. Typed objects use exactly \
                 {form:\"all\"}, {form:\"path\",glob:\"src/**\"}, or \
                 {form:\"ref\",ref:\"@key\"}.",
            ),
        ),
        (
            "items",
            Value::object(vec![(
                "oneOf",
                Value::array(vec![
                    string("Compact scope term: all, a bare path glob, or @key."),
                    object(
                        vec![("form", enumeration("All project paths.", &["all"]))],
                        &["form"],
                    ),
                    object(
                        vec![
                            ("form", enumeration("Path-glob scope.", &["path"])),
                            ("glob", string("Repository path glob, for example src/**.")),
                        ],
                        &["form", "glob"],
                    ),
                    object(
                        vec![
                            ("form", enumeration("Record-reference scope.", &["ref"])),
                            (
                                "ref",
                                string("Record reference, for example @sys.track.ui."),
                            ),
                        ],
                        &["form", "ref"],
                    ),
                ]),
            )]),
        ),
    ])
}

fn claims_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        (
            "description",
            Value::string("Addressable claims, each with an anchor and text."),
        ),
        (
            "items",
            object(
                vec![
                    ("anchor", string("The anchor.")),
                    ("text", string("The claim.")),
                ],
                &["anchor", "text"],
            ),
        ),
    ])
}

fn relations_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("object")),
        (
            "description",
            Value::string(
                "relation name -> array of references. Target kinds are constrained \
                 (V-005): `implements` accepts requirement, policy, constraint or \
                 decision, not a work record. Use depends_on for a prerequisite; a \
                 completed planning prerequisite remains satisfied. Use derived_from \
                 for provenance rather than prerequisite ordering. A `contradicts` edge \
                 between two live records needs `acknowledged: true` on one of them, or \
                 one side superseded, or it fails V-023 (AKR-R041). Pin a revision \
                 (`@key/2`) only to cite that revision for good: an unpinned `@key` \
                 follows the head, and a pinned reference to a revision that a later one \
                 supersedes fails V-006 (AKR-L021).",
            ),
        ),
        ("additionalProperties", string_array("References.")),
    ])
}

fn source_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("object")),
        (
            "description",
            Value::string("One source attribution for the record."),
        ),
        (
            "properties",
            Value::Object(
                vec![
                    (
                        "kind".to_owned(),
                        enumeration(
                            "legacy, external or internal",
                            &["legacy", "external", "internal"],
                        ),
                    ),
                    (
                        "path".to_owned(),
                        string("Path to the source file this record was authored from."),
                    ),
                    ("url".to_owned(), string("URL to the source material.")),
                    (
                        "excerpt".to_owned(),
                        string("Excerpt of the source material associated with this record."),
                    ),
                    (
                        "document".to_owned(),
                        string("Registered source document id for an exact citation."),
                    ),
                    (
                        "role".to_owned(),
                        enumeration(
                            "How the source contributes to this record.",
                            &["origin", "rationale", "evidence", "constraint", "example"],
                        ),
                    ),
                    (
                        "start_byte".to_owned(),
                        integer("First cited byte, inclusive."),
                    ),
                    (
                        "end_byte".to_owned(),
                        integer("First byte after the cited passage."),
                    ),
                    (
                        "start_line".to_owned(),
                        integer("First cited line, one-based."),
                    ),
                    (
                        "end_line".to_owned(),
                        integer("Last cited line, one-based."),
                    ),
                    (
                        "excerpt_hash".to_owned(),
                        string("Optional sha256 hash of the cited bytes."),
                    ),
                    (
                        "use".to_owned(),
                        string("What the project adopted or retained from this source."),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "required",
            Value::array(vec![Value::string("kind".to_owned())]),
        ),
        ("additionalProperties", Value::bool(false)),
    ])
}

fn sources_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        (
            "description",
            Value::string(
                "Source attributions for the record. A citation given by line \
                 (start_line/end_line with no bytes) also needs `document`, the id of a \
                 registered source: the byte offsets are read off the registered bytes, \
                 and `path` is not a substitute for it. Give start_byte and end_byte \
                 instead if the document is not in the library.",
            ),
        ),
        ("items", source_schema()),
    ])
}

/// `acceptance`: the checks a milestone or work record must satisfy to complete (D-016).
/// Milestones require a non-empty acceptance block to exist at all (V-008), so this is
/// what lets `knowledge.propose` create one instead of forcing `akr propose --from`.
fn acceptance_schema(description: &str) -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        ("description", Value::string(description)),
        (
            "items",
            object(
                vec![
                    (
                        "id",
                        string("The check identifier, unique within the record."),
                    ),
                    ("statement", string("The observable outcome.")),
                    (
                        "method",
                        enumeration("How the check is carried out.", &check_method_values()),
                    ),
                    ("command", string("The exact command, for method command.")),
                    (
                        "verified_by",
                        string_array("Evidence references that already satisfy this check."),
                    ),
                ],
                &["id", "statement", "method"],
            ),
        ),
    ])
}

fn dispositions_schema() -> Value {
    Value::object(vec![
        ("type", Value::string("array")),
        (
            "description",
            Value::string(
                "One entry per unfinished part_of child (D-017). A missing one is refused \
                 and the children are listed in the error payload.",
            ),
        ),
        (
            "items",
            object(
                vec![
                    ("child", string("The child's key.")),
                    (
                        "outcome",
                        enumeration("What became of the child.", &outcome_values()),
                    ),
                    (
                        "into",
                        string("Where it went, where the outcome needs one."),
                    ),
                    ("note", string("Why.")),
                ],
                &["child", "outcome"],
            ),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `evidence_method_values` falls back rather than panicking, because a panic on the
    /// `tools/list` path would kill the server during the handshake. That makes the
    /// fallback a silent-drift risk, so the divergence is caught here instead.
    #[test]
    fn evidence_method_values_track_the_ledger() {
        let table = akr_core::model::Kind::Evidence
            .content_enum_values(akr_core::model::ContentSlot::Method)
            .expect("evidence.method is an enum content slot");
        assert_eq!(evidence_method_values(), table.to_vec());
    }

    /// The schema must never advertise a value the dispatcher rejects, nor omit one it
    /// accepts.
    #[test]
    fn schema_enums_match_their_authoritative_tables() {
        assert_eq!(
            check_method_values(),
            akr_core::model::CheckMethod::ALL
                .iter()
                .map(|m| m.name())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            outcome_values(),
            akr_core::model::Outcome::ALL
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn authoring_schema_names_scope_forms_and_exact_kind_slots() {
        let schema = input_schema("knowledge.propose").expect("propose schema");
        let rendered = schema.to_pretty();
        for required in [
            "oneOf",
            "glob",
            "Compact scope term",
            "decision [decision:text:required",
            "work [intent:text:required",
            "Topic is a separate top-level field",
        ] {
            assert!(
                rendered.contains(required),
                "missing {required:?}: {rendered}"
            );
        }
    }
}
