//! JSON-RPC 2.0 over stdio: the transport `docs/08-mcp.md` §1 specifies.
//!
//! Requests in on stdin, responses out on stdout. The historical framing is one JSON
//! document per line — greppable, replayable, and what `tests/differential.rs` still
//! speaks. Official MCP stdio clients (Grok, Claude Code, the TypeScript SDK) send
//! `Content-Length` headers instead. The server accepts both and answers in the framing
//! of the request that produced the response, so a host that cannot parse NDJSON is not
//! left attached with zero tools.
//!
//! Four methods: `initialize`, `server/discover`, `tools/list`, `tools/call`.
//! Notifications — a request with no `id` — are acknowledged by producing no response, as
//! JSON-RPC requires.
//!
//! # Tool failures are results, not transport errors
//!
//! A tool that refuses returns a *successful* JSON-RPC response whose result carries
//! `isError: true` and §5's payload. The transport succeeded; the ledger said no. Conflating
//! the two would make an agent unable to tell a refusal it should read from a server it
//! should restart.

use akr_core::json::{Value, parse};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::errors::ToolError;
use crate::schema::{TOOLS, ToolId, input_schema_for, output_schema_for};
use crate::tools;

/// The legacy protocol version this server still accepts.
pub const PROTOCOL_LEGACY: &str = "2024-11-05";
/// The March 2025 MCP revision, still sent by some hosts.
pub const PROTOCOL_2025_03: &str = "2025-03-26";
/// The June 2025 MCP revision, what Grok and current SDKs send.
pub const PROTOCOL_2025_06: &str = "2025-06-18";
/// The November 2025 MCP revision, the default the rmcp 3.x clients prefer.
///
/// Omitting it did not fail loudly: a client offering `2025-11-25` found nothing to match
/// and was silently answered `2024-11-05`, so every such host ran the whole session a
/// generation behind what both ends could speak.
pub const PROTOCOL_2025_11: &str = "2025-11-25";
/// The current protocol version this server now negotiates.
pub const PROTOCOL_CURRENT: &str = "2026-07-28";
/// Supported protocol versions, in preference order.
pub const SUPPORTED_PROTOCOLS: &[&str] = &[
    PROTOCOL_CURRENT,
    PROTOCOL_2025_11,
    PROTOCOL_2025_06,
    PROTOCOL_2025_03,
    PROTOCOL_LEGACY,
];

/// `resultType` for a result that is the whole answer rather than a task or a prompt.
///
/// One of exactly three values MCP defines — `complete`, `input_required`, `task` — and
/// the only one a ledger read or write ever produces. See `MCP_RESULT_TYPES` in
/// `tests/conformance.rs`, which holds the closed set this is checked against.
const RESULT_TYPE_COMPLETE: &str = "complete";

/// Where a discovery result carries the server's identity.
///
/// `serverInfo` is a top-level field of an `initialize` result and a `_meta` entry of a
/// discovery one. The difference is not cosmetic: the discovery result is a closed schema,
/// and a client that cannot deserialise it has no way back to the legacy handshake.
const SERVER_INFO_META_KEY: &str = "io.modelcontextprotocol/serverInfo";

/// The server's own version, matching the tool version the CLI reports.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A server bound to one workspace.
pub struct Server {
    root: PathBuf,
    surface: Surface,
    accounting: Option<PathBuf>,
    /// Advertise `knowledge_search` instead of `knowledge.search`.
    ///
    /// Set by `--tool-names underscore` or when `initialize` names a host that
    /// drops dotted MCP tool names (Grok Build 1.0.25).
    host_safe_tool_names: AtomicBool,
    /// The binary this process was loaded from, to notice it being replaced.
    executable: Option<crate::skew::Executable>,
    /// The root is not inside any workspace (`AKR-C011`), so every tool takes and
    /// advertises the optional `workspace` argument (D-045).
    rootless: bool,
    /// Optional frozen evaluation date (used by deterministic differential tests).
    today: Option<akr_core::model::Date>,
}

/// Which half of the tool catalogue this server exposes.
///
/// Tool schemas are a fixed tax on every session that loads them, and an implementation
/// agent that will only ever read pays it for eight write tools it never calls. `read`
/// serves the surface that answers questions; `full` adds the ones that change the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Surface {
    /// Read tools only.
    Read,
    /// Every tool (the default).
    #[default]
    Full,
}

impl Surface {
    /// Parses `--surface`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "read" => Some(Self::Read),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Whether this surface exposes `tool`.
    #[must_use]
    pub fn exposes(self, tool: &crate::schema::Tool) -> bool {
        match self {
            Self::Read => !tool.writes,
            Self::Full => true,
        }
    }
}

impl Server {
    /// A server for the workspace at `root`, exposing every tool.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let rootless = matches!(
            akr_cli::session::locate(&root),
            Err(error) if error.code == "AKR-C011"
        );
        Self {
            root,
            surface: Surface::Full,
            accounting: None,
            host_safe_tool_names: AtomicBool::new(false),
            executable: crate::skew::Executable::current(),
            rootless,
            today: None,
        }
    }

    /// Sets an explicit evaluation date instead of reading the system clock.
    #[must_use]
    pub fn with_today(mut self, today: Option<akr_core::model::Date>) -> Self {
        self.today = today;
        self
    }

    /// Restricts the catalogue to one surface.
    #[must_use]
    pub fn with_surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }

    /// Appends one JSONL line per tool call to `path`.
    #[must_use]
    pub fn with_accounting(mut self, path: impl Into<PathBuf>) -> Self {
        self.accounting = Some(path.into());
        self
    }

    /// Advertises underscored tool names (`knowledge_search`) instead of dotted ones.
    ///
    /// `tools/call` accepts both forms either way. Use this when the host is known
    /// ahead of `initialize`, or for hosts we do not fingerprint.
    #[must_use]
    pub fn with_host_safe_tool_names(self, enabled: bool) -> Self {
        self.host_safe_tool_names.store(enabled, Ordering::Relaxed);
        self
    }

    /// The workspace root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Handles one request, returning the response, or `None` for a notification.
    #[must_use]
    pub fn handle(&self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        // A notification carries no `id` and gets no response, per JSON-RPC 2.0.
        let id = id.filter(|value| !value.is_null())?;

        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "server/discover" => Ok(self.server_discover(&params)),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => Ok(self.tools_call(&params)),
            "ping" => Ok(Value::Object(Vec::new())),
            other => Err((-32601, format!("unknown method {other:?}"))),
        };

        Some(match result {
            Ok(result) => Value::object(vec![
                ("jsonrpc", Value::string("2.0")),
                ("id", id),
                ("result", result),
            ]),
            Err((code, message)) => Value::object(vec![
                ("jsonrpc", Value::string("2.0")),
                ("id", id),
                (
                    "error",
                    Value::object(vec![
                        ("code", Value::integer(code)),
                        ("message", Value::string(message)),
                    ]),
                ),
            ]),
        })
    }

    /// The identity every handshake publishes.
    fn server_info(&self) -> Value {
        Value::object(vec![
            ("name", Value::string("akr-mcp")),
            ("version", Value::string(SERVER_VERSION)),
            // Published so a client can see a stale server before it trips over
            // one: the friction this answers was diagnosed twice as a ledger bug.
            (
                "vocabularyVersion",
                Value::string(crate::skew::SERVER_VOCABULARY),
            ),
        ])
    }

    /// The guidance every handshake carries.
    fn instructions(&self) -> Value {
        let (context, validate) = if self.host_safe_tool_names.load(Ordering::Relaxed) {
            ("knowledge_context", "knowledge_validate")
        } else {
            ("knowledge.context", "knowledge.validate")
        };
        let (secret_put, trace) = if self.host_safe_tool_names.load(Ordering::Relaxed) {
            ("knowledge_secret_put", "knowledge_trace")
        } else {
            ("knowledge.secret_put", "knowledge.trace")
        };
        // D-049: what the ledger is for, said where every session reads it.
        let purpose = format!(
            " Part of your job is to help the next agent understand what happened here:              what was asked, what you found in the code as it stood, and why that led to              what you did. Record it as you go with {trace} on the work record, in              proportion to the expected impact of the change; never restate a diff."
        );
        let privacy = format!(
            "{purpose} Keep private paths, filenames and credentials in local-only secret entries \
             using {secret_put} from a private UTF-8 file; cite the returned opaque \
             {{{{secret:<id>}}}} token in public text. Public output renders [*]. Never copy \
             resolved values into public records, sources, evidence or handoffs. Secret \
             storage (.akr/secrets/) must stay out of Git and published files. Reveal \
             is explicit and sends values to the caller; do not reveal keys to remote agents."
        );
        if self.rootless {
            return Value::string(format!(
                "No AKR ledger at {}; the workspaces are below it. Pass `workspace` (a \
                 workspace directory, absolute or relative to that directory) with every \
                 call. Call {context} before touching code, and {validate} before handing \
                 work back.{privacy}",
                std::path::absolute(&self.root)
                    .unwrap_or_else(|_| self.root.clone())
                    .display()
            ));
        }
        Value::string(format!(
            "AKR knowledge ledger at {}. Call {context} before touching \
             code, and {validate} before handing work back.{privacy}",
            self.root.display()
        ))
    }

    /// The capability set both handshakes advertise.
    fn capabilities() -> Value {
        Value::object(vec![("tools", Value::Object(Vec::new()))])
    }

    fn initialize(&self, params: &Value) -> Value {
        if client_wants_host_safe_tool_names(params) {
            self.host_safe_tool_names.store(true, Ordering::Relaxed);
        }
        let protocol_version = select_protocol(params);
        Value::object(vec![
            ("protocolVersion", Value::string(protocol_version)),
            ("capabilities", Self::capabilities()),
            ("serverInfo", self.server_info()),
            ("instructions", self.instructions()),
        ])
    }

    /// The `server/discover` result, in the shape a discovery client actually deserialises.
    ///
    /// A client that speaks discovery sends this *before* `initialize`, and retreats to the
    /// legacy handshake on one signal only: a `-32601`. Answering successfully in the wrong
    /// shape is therefore worse than not implementing the method at all — the client reads a
    /// malformed success as a broken peer rather than an old one, and gives up rather than
    /// falling back. That is not hypothetical: an earlier version of this function returned
    /// an `initialize` result under a discovery method name, and every rmcp 3.x host — Codex
    /// among them — refused the connection outright.
    ///
    /// Every field below except `instructions` and `_meta` is required by that schema.
    ///
    /// The request carries no version to select against: a discovery client sends its
    /// preferences as request metadata and chooses from `supportedVersions` itself.
    fn server_discover(&self, _params: &Value) -> Value {
        Value::object(vec![
            ("resultType", Value::string(RESULT_TYPE_COMPLETE)),
            (
                "supportedVersions",
                Value::array(
                    SUPPORTED_PROTOCOLS
                        .iter()
                        .map(|version| Value::string(*version))
                        .collect(),
                ),
            ),
            ("capabilities", Self::capabilities()),
            ("instructions", self.instructions()),
            // Not cacheable. A ledger server is cheap to ask again, and a capability set
            // held past its truth is exactly the skew this server exists to surface.
            ("ttlMs", Value::integer(0)),
            ("cacheScope", Value::string("private")),
            (
                "_meta",
                Value::object(vec![(SERVER_INFO_META_KEY, self.server_info())]),
            ),
        ])
    }

    fn tools_list(&self) -> Value {
        let host_safe = self.host_safe_tool_names.load(Ordering::Relaxed);
        Value::object(vec![(
            "tools",
            Value::array(
                TOOLS
                    .iter()
                    .filter(|tool| self.surface.exposes(tool))
                    .map(|tool| {
                        let mut fields = vec![
                            (
                                "name",
                                Value::string(if host_safe {
                                    crate::schema::host_safe_name(tool.name)
                                } else {
                                    tool.name.to_owned()
                                }),
                            ),
                            ("description", Value::string(tool.description)),
                        ];
                        if host_safe {
                            fields.push(("title", Value::string(tool.name)));
                        }
                        fields.extend([
                            ("inputSchema", {
                                // Always advertise optional `workspace` so agents working in
                                // sibling git worktrees or multi-root checkouts can target
                                // their worktree without failing client-side schema checks.
                                with_workspace_argument(input_schema_for(tool.id))
                            }),
                            ("outputSchema", output_schema_for(tool.id)),
                            (
                                "annotations",
                                Value::object(vec![
                                    ("readOnlyHint", Value::bool(!tool.writes)),
                                    // Nothing in AKR deletes knowledge
                                    // (`01-architecture.md` §9), so no tool is destructive
                                    // even when it writes.
                                    ("destructiveHint", Value::bool(false)),
                                    ("idempotentHint", Value::bool(tool.id != ToolId::Revise)),
                                ]),
                            ),
                        ]);
                        Value::object(fields)
                    })
                    .collect(),
            ),
        )])
    }

    fn tools_call(&self, params: &Value) -> Value {
        let advertised = params.get("name").and_then(Value::as_str).unwrap_or("");
        let tool_id = ToolId::from_name(advertised);
        let name = match tool_id {
            Some(id) => id.name(),
            None => advertised,
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Vec::new()));

        let started = std::time::Instant::now();
        // `workspace` picks the ledger for this call (D-045), the MCP spelling of the
        // command line's `--dir`; the tools never see it.
        let (root, outcome) = match workspace_root(&self.root, &arguments) {
            Ok((root, tool_arguments)) => {
                let outcome = guarded_tool_call(|| match tool_id {
                    Some(id) => tools::call_id_at(&root, id, &tool_arguments, self.today),
                    None => tools::call_at(&root, advertised, &tool_arguments, self.today),
                });
                (root, outcome)
            }
            Err(error) => (self.root.clone(), Err(error)),
        };
        let (text, structured, is_error) = match outcome {
            Ok(payload) => {
                // Start and context assemble to their own budget. When the caller omits
                // `budget_tokens`, still treat them as internally budgeted at the
                // assembly default: a second, smaller adapter ceiling produced a compact
                // preview that itself exceeded the advertised hard limit.
                let internally_budgeted = match tool_id {
                    Some(ToolId::Start | ToolId::HandoffSessionBegin) => {
                        Some(token_budget(&arguments, 1_400))
                    }
                    Some(ToolId::Context) => arguments
                        .get("budget_tokens")
                        .and_then(Value::as_integer)
                        .and_then(|value| usize::try_from(value).ok()),
                    _ => None,
                };
                let enforced = match tool_id {
                    Some(id) => crate::budget::enforce_id(
                        id,
                        payload.text(),
                        payload.structured(),
                        internally_budgeted,
                        &arguments,
                    ),
                    None => crate::budget::enforce(
                        name,
                        payload.text(),
                        payload.structured(),
                        internally_budgeted,
                        &arguments,
                    ),
                };
                (enforced.text, enforced.structured, false)
            }
            Err(mut error) => {
                // A failing call is where an agent meets a stale server, and a type error
                // about a slot it never wrote is the least explicable thing the surface
                // can say. Naming the skew here turns it into a one-line remedy.
                if let Some(skew) = crate::skew::detect(&root) {
                    error.diagnostics.push(skew.diagnostic());
                }
                if let Some(replaced) = self
                    .executable
                    .as_ref()
                    .and_then(crate::skew::Executable::replaced)
                {
                    error.diagnostics.push(replaced);
                }
                (None, error.to_json(), true)
            }
        };

        crate::budget::record(
            self.accounting.as_deref(),
            &crate::budget::Call {
                tool: name.to_owned(),
                input_bytes: arguments.to_pretty().len(),
                text_output_bytes: text.as_ref().map_or(0, String::len),
                structured_output_bytes: structured.to_pretty().len(),
                estimated_output_tokens: crate::budget::estimate_tokens(
                    text.as_deref().unwrap_or_default(),
                ) + crate::budget::estimate_tokens(
                    &structured.to_pretty(),
                ),
                truncated: structured.get("truncated").and_then(Value::as_bool) == Some(true),
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            },
        );

        content(text.as_deref(), &structured, is_error)
    }
}

/// Contains an implementation panic to the request that triggered it.
///
/// The ledger write pipeline performs its durable write only after validation and uses
/// atomic file replacement. A panic is nevertheless an internal bug, so the response is
/// retryable once and the long-lived stdio server stays available for diagnostics.
fn guarded_tool_call<T>(call: impl FnOnce() -> Result<T, ToolError>) -> Result<T, ToolError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(result) => result,
        Err(_) => Err(ToolError::new(
            "AKR-X099",
            "the AKR tool implementation failed unexpectedly; the server contained the failure",
        )),
    }
}

/// An MCP tool result: the payload as JSON text, plus the structured form.
///
/// Both, deliberately. `content` is what a client that only knows about text will show a
/// model; `structuredContent` is what a client that understands the schema will parse.
/// Emitting one and not the other would make the server work well with half the runtimes.
fn content(text: Option<&str>, structured: &Value, is_error: bool) -> Value {
    let text = text
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| structured.to_pretty());
    Value::object(vec![
        (
            "content",
            Value::array(vec![Value::object(vec![
                ("type", Value::string("text")),
                ("text", Value::string(text)),
            ])]),
        ),
        // `complete` — the result is the whole answer. The vocabulary is closed
        // (`complete`, `input_required`, `task`); a value outside it is not an
        // extension a client ignores, it is a result the client cannot type, and
        // an SDK that matches on this field rejects the whole response. This said
        // `"tool"` for every tool AKR has, which is why no rmcp 3.x host could
        // call one.
        ("resultType", Value::string(RESULT_TYPE_COMPLETE)),
        ("structuredContent", structured.clone()),
        ("isError", Value::bool(is_error)),
    ])
}

/// Grok Build 1.0.25 (clientInfo.name `grok`) silently drops dotted MCP tool names.
fn client_wants_host_safe_tool_names(params: &Value) -> bool {
    params
        .get("clientInfo")
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("grok"))
}

fn select_protocol(parameters: &Value) -> &'static str {
    parameters
        .get("protocolVersion")
        .and_then(Value::as_str)
        .and_then(|requested| {
            SUPPORTED_PROTOCOLS
                .iter()
                .copied()
                .find(|protocol| *protocol == requested)
        })
        .unwrap_or(PROTOCOL_LEGACY)
}

fn token_budget(arguments: &Value, default: usize) -> usize {
    arguments
        .get("budget_tokens")
        .and_then(Value::as_integer)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

/// How one stdio message was framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Ndjson,
    ContentLength,
}

/// Reads one JSON-RPC request from `input`.
///
/// NDJSON (a `{` or `[` on its own line) and official MCP `Content-Length` headers are
/// both accepted. The response is written in the same framing so a host that cannot
/// parse the other style still sees a valid handshake.
///
/// # Errors
/// Any I/O failure on either stream. A malformed frame is answered with a JSON-RPC parse
/// error rather than ending the session: one bad message should not take down a server an
/// agent is mid-task with.
pub fn serve(
    server: &Server,
    mut input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    while let Some((body, framing)) = read_frame(&mut input)? {
        let response = match body {
            FrameBody::Json(body) if body.trim().is_empty() => continue,
            FrameBody::Json(body) => match parse(body.trim()) {
                Ok(request) => server.handle(&request),
                Err(error) => Some(Value::object(vec![
                    ("jsonrpc", Value::string("2.0")),
                    ("id", Value::Null),
                    (
                        "error",
                        Value::object(vec![
                            ("code", Value::integer(-32700)),
                            ("message", Value::string(format!("parse error: {error}"))),
                        ]),
                    ),
                ])),
            },
            FrameBody::Invalid(message) => Some(Value::object(vec![
                ("jsonrpc", Value::string("2.0")),
                ("id", Value::Null),
                (
                    "error",
                    Value::object(vec![
                        ("code", Value::integer(-32700)),
                        ("message", Value::string(format!("parse error: {message}"))),
                    ]),
                ),
            ])),
        };
        if let Some(response) = response {
            write_frame(&mut output, &compact(&response), framing)?;
        }
    }
    Ok(())
}

enum FrameBody {
    Json(String),
    Invalid(String),
}

fn read_frame(input: &mut impl BufRead) -> std::io::Result<Option<(FrameBody, Framing)>> {
    loop {
        // `fill_buf` may expose only `Con` from a fragmented `Content-Length` header.
        // Read the complete line before deciding its framing so ordinary pipe writes
        // cannot turn a valid header into a malformed NDJSON request.
        let mut first = String::new();
        if input.read_line(&mut first)? == 0 {
            return Ok(None);
        }
        if first.trim().is_empty() {
            continue;
        }
        let trimmed = first.trim_start();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return Ok(Some((FrameBody::Json(first), Framing::Ndjson)));
        }

        // Official MCP headers start with `Content-Length:`. Anything else is a
        // malformed NDJSON line — treating it as a header would swallow the next
        // real request, which is how a bad line used to take the session down.
        if !starts_like_header(first.as_bytes()) {
            return Ok(Some((FrameBody::Json(first), Framing::Ndjson)));
        }

        let mut content_length = None;
        let mut header = Some(first);
        while let Some(line) = header.take() {
            if line == "\n" || line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                match value.trim().parse() {
                    Ok(length) => content_length = Some(length),
                    Err(_) => {
                        return Ok(Some((
                            FrameBody::Invalid("invalid Content-Length header".to_owned()),
                            Framing::ContentLength,
                        )));
                    }
                }
            }
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                return Ok(Some((
                    FrameBody::Invalid("unterminated Content-Length headers".to_owned()),
                    Framing::ContentLength,
                )));
            }
            header = Some(line);
        }
        let Some(len) = content_length else {
            return Ok(Some((
                FrameBody::Invalid("missing Content-Length header".to_owned()),
                Framing::ContentLength,
            )));
        };
        let mut body = vec![0_u8; len];
        input.read_exact(&mut body)?;
        let body = String::from_utf8(body)
            .map(FrameBody::Json)
            .unwrap_or_else(|_| FrameBody::Invalid("Content-Length body is not UTF-8".to_owned()));
        return Ok(Some((body, Framing::ContentLength)));
    }
}

fn starts_like_header(bytes: &[u8]) -> bool {
    let line = bytes
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .next()
        .unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    let Some((name, _)) = text.split_once(':') else {
        return false;
    };
    let name = name.trim();
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn write_frame(output: &mut impl Write, body: &str, framing: Framing) -> std::io::Result<()> {
    match framing {
        Framing::Ndjson => writeln!(output, "{body}")?,
        Framing::ContentLength => write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?,
    }
    output.flush()
}

/// One response, on one line.
///
/// [`Value::to_pretty`] is for humans reading an envelope; a transport frame is delimited
/// by newlines, so it cannot contain them.
fn compact(value: &Value) -> String {
    value
        .to_pretty()
        .lines()
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join("")
}

/// Description of the optional `workspace` argument a rootless server advertises.
const WORKSPACE_ARGUMENT: &str = "The AKR workspace for this call: a directory containing `.akr/` \
     (or inside one), absolute or relative to the server's directory. The MCP spelling of \
     the command line's `--dir`; needed because this server was started outside any \
     workspace (D-045).";

/// Adds the optional `workspace` property to a tool's input schema.
fn with_workspace_argument(schema: Value) -> Value {
    let Value::Object(mut fields) = schema else {
        return schema;
    };
    let property = Value::object(vec![
        ("type", Value::string("string")),
        ("description", Value::string(WORKSPACE_ARGUMENT)),
    ]);
    match fields.iter_mut().find(|(key, _)| key == "properties") {
        Some((_, Value::Object(properties))) => {
            properties.push(("workspace".to_owned(), property));
        }
        _ => fields.push((
            "properties".to_owned(),
            Value::Object(vec![("workspace".to_owned(), property)]),
        )),
    }
    Value::Object(fields)
}

/// The root a call runs against and the arguments the tool sees.
///
/// Without `workspace`, the server's own root and the arguments unchanged. With it, that
/// directory (relative paths resolve against the server's root) and the arguments
/// without it. Honoured whether or not the schema advertised it, so a caller that names
/// its workspace explicitly is never refused for doing so.
fn workspace_root(root: &Path, arguments: &Value) -> Result<(PathBuf, Value), ToolError> {
    let Value::Object(fields) = arguments else {
        return Ok((root.to_path_buf(), arguments.clone()));
    };
    let Some((_, value)) = fields.iter().find(|(key, _)| key == "workspace") else {
        return Ok((root.to_path_buf(), arguments.clone()));
    };
    let Some(path) = value.as_str().filter(|path| !path.trim().is_empty()) else {
        return Err(ToolError::new(
            "AKR-C004",
            "`workspace`: expected a non-empty directory path",
        ));
    };
    let path = Path::new(path);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let rest = fields
        .iter()
        .filter(|(key, _)| key != "workspace")
        .cloned()
        .collect();
    Ok((resolved, Value::Object(rest)))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Cursor, Read};

    use akr_core::json::{Value, parse};

    use super::{Server, guarded_tool_call, serve};

    struct FragmentedInput {
        bytes: Vec<u8>,
        cursor: usize,
        chunk_size: usize,
    }

    impl Read for FragmentedInput {
        fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
            let available = self.fill_buf()?;
            let count = available.len().min(into.len());
            into[..count].copy_from_slice(&available[..count]);
            self.consume(count);
            Ok(count)
        }
    }

    impl BufRead for FragmentedInput {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            let end = (self.cursor + self.chunk_size).min(self.bytes.len());
            Ok(&self.bytes[self.cursor..end])
        }

        fn consume(&mut self, amount: usize) {
            self.cursor = (self.cursor + amount).min(self.bytes.len());
        }
    }

    #[test]
    fn a_tool_panic_becomes_a_retryable_internal_error() {
        let error = guarded_tool_call::<()>(|| panic!("injected tool panic"))
            .expect_err("the panic is contained");
        let payload = error.to_json().to_pretty();
        assert!(payload.contains("\"class\": \"internal\""), "{payload}");
        assert!(payload.contains("\"retryable\": true"), "{payload}");
        assert!(payload.contains("AKR-X099"), "{payload}");
    }

    #[test]
    fn a_thousand_requests_and_a_malformed_frame_do_not_end_the_session() {
        let mut input = String::new();
        for id in 0..1_000 {
            let request = match id % 3 {
                0 => format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#),
                1 => format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#),
                _ => format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"knowledge.unknown","arguments":{{}}}}}}"#
                ),
            };
            input.push_str(&request);
            input.push('\n');
            if id == 499 {
                input.push_str("{malformed\n");
            }
        }

        let mut output = Vec::new();
        serve(
            &Server::new("/workspace-not-opened-by-this-test"),
            Cursor::new(input),
            &mut output,
        )
        .expect("the session remains available");

        let output = String::from_utf8(output).expect("responses are UTF-8");
        let responses = output.lines().collect::<Vec<_>>();
        assert_eq!(responses.len(), 1_001);
        for response in &responses {
            parse(response).expect("every response is valid JSON");
        }
        assert!(responses[500].contains("\"code\": -32700"));
        assert!(
            responses
                .last()
                .is_some_and(|line| line.contains("\"id\": 999"))
        );
    }

    #[test]
    fn content_length_initialize_echoes_the_requested_protocol_and_lists_tools() {
        let init = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"grok","version":"1"}}}"#;
        let list = br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let mut input = Vec::new();
        write_content_length(&mut input, init);
        write_content_length(&mut input, list);

        let mut output = Vec::new();
        serve(
            &Server::new("/workspace-not-opened-by-this-test"),
            Cursor::new(input),
            &mut output,
        )
        .expect("content-length session stays up");

        let (first, rest) = split_content_length(&output).expect("first frame");
        let (second, rest) = split_content_length(rest).expect("second frame");
        assert!(rest.is_empty(), "{}", String::from_utf8_lossy(rest));

        let initialize = parse(&first).expect("initialize JSON");
        assert_eq!(
            initialize
                .get("result")
                .and_then(|result| result.get("protocolVersion"))
                .and_then(Value::as_str),
            Some("2025-06-18")
        );
        let tools = parse(&second).expect("tools/list JSON");
        let listed = tools
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .expect("tools array");
        // Grok's clientInfo is why this fixture exists: that host drops dotted
        // names, so the advertised catalogue must be the underscored form.
        assert!(
            listed.iter().any(|tool| {
                tool.get("name").and_then(Value::as_str) == Some("knowledge_start")
                    && tool.get("title").and_then(Value::as_str) == Some("knowledge.start")
            }),
            "{second}"
        );
    }

    #[test]
    fn a_fragmented_content_length_header_is_not_misclassified_as_ndjson() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let mut bytes = Vec::new();
        write_content_length(&mut bytes, body);
        let mut output = Vec::new();
        serve(
            &Server::new("/workspace-not-opened-by-this-test"),
            FragmentedInput {
                bytes,
                cursor: 0,
                chunk_size: 3,
            },
            &mut output,
        )
        .expect("fragmented session stays up");

        let (response, rest) = split_content_length(&output).expect("Content-Length response");
        assert!(rest.is_empty());
        assert!(
            parse(&response)
                .expect("response is JSON")
                .get("result")
                .is_some(),
            "{response}"
        );
    }

    fn write_content_length(into: &mut Vec<u8>, body: &[u8]) {
        into.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        into.extend_from_slice(body);
    }

    fn split_content_length(bytes: &[u8]) -> Option<(String, &[u8])> {
        let text = std::str::from_utf8(bytes).ok()?;
        let (headers, rest) = text.split_once("\r\n\r\n")?;
        let length: usize = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse().ok())?;
        let body = rest.get(..length)?.to_owned();
        Some((body, &rest.as_bytes()[length..]))
    }
}
