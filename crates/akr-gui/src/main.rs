#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use akr_gui::model::{
    AcceptanceCheck, Diagnostic, GitMetadata, LoadPhase, Record, Relation, ReviewCounts,
    ReviewSnapshot, SnapshotError, WorkspaceLoader,
};
use akr_gui::ui::{AppModel, Panel, TreeMode};
use pixelkit_raster::{Painter, RasterKernel, Rect, WindowBuffer};
use pixelkit_shell::{Input, KeyInput, Modifiers, MouseButton, Scale};
use pixelkit_text::{Align, Face, FontSet, TextCache};
use pixelkit_ui::list::GUTTER as LIST_GUTTER;
use pixelkit_ui::widget::TABLE_GUTTER;
use pixelkit_ui::{
    Cell, Column, Row, ScrollState, TableOutcome, TableState, TextFieldState, Theme, Ui, navigate,
};
use pixelkit_windowing::{ImeEvent, PixelApp, Waker, WindowConfig, run_app};

// --- embedded type --------------------------------------------------------

fn latin_face() -> &'static Face {
    static FACE: OnceLock<Face> = OnceLock::new();
    FACE.get_or_init(|| {
        Face::from_bytes(include_bytes!("../fonts/NotoSans-Regular.ttf"))
            .expect("the embedded Noto Sans parses")
    })
}

fn font_set() -> &'static FontSet {
    static SET: OnceLock<FontSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = FontSet::new();
        set.add(latin_face());
        set
    })
}

fn theme() -> Theme {
    Theme {
        accent: 0x0056_b6d9,
        selection: 0x0033_5870,
        ..Theme::default()
    }
}

// --- layout (logical pixels; tests and frames share this) ------------------

const HEADER_H: i32 = 52;
const FILTER_H: i32 = 52;
const FOOTER_H: i32 = 30;
const LEFT_FRAC: f32 = 0.32;
const SPLIT_GAP: i32 = 6;

#[derive(Debug, Clone, Copy)]
struct Areas {
    header: Rect,
    filter_bar: Rect,
    left: Rect,
    right: Rect,
    footer: Rect,
}

fn layout_areas(width: i32, height: i32) -> Areas {
    let header = Rect::new(0, 0, width, HEADER_H);
    let filter_bar = Rect::new(0, HEADER_H, width, FILTER_H);
    let footer = Rect::new(0, height - FOOTER_H, width, FOOTER_H);
    let content = Rect::new(
        0,
        HEADER_H + FILTER_H,
        width,
        height - HEADER_H - FILTER_H - FOOTER_H,
    );
    let left_w = (width as f32 * LEFT_FRAC) as i32;
    let left = Rect::new(content.x, content.y, left_w, content.h);
    let right = Rect::new(
        content.x + left_w + SPLIT_GAP,
        content.y,
        (content.w - left_w - SPLIT_GAP).max(0),
        content.h,
    );
    Areas {
        header,
        filter_bar,
        left,
        right,
        footer,
    }
}

// --- workspace helpers ------------------------------------------------------

/// Resolves a typed or dropped path to the workspace to open.
///
/// A file resolves to its directory, and any directory inside a ledger resolves
/// to the nearest ancestor holding `.akr`, so dropping a source file on the
/// window opens that file's project rather than a directory with no records.
/// A directory with no `.akr` anywhere above it is still opened — the load will
/// report the real diagnostic — but a path that does not exist is refused here.
fn workspace_root(requested: &Path) -> Result<PathBuf, String> {
    let path = requested
        .canonicalize()
        .map_err(|error| format!("{}: {error}", requested.display()))?;
    let start = if path.is_dir() {
        path.as_path()
    } else {
        path.parent().unwrap_or(path.as_path())
    };
    let resolved = start
        .ancestors()
        .find(|candidate| candidate.join(".akr").is_dir())
        .unwrap_or(start);
    // Windows canonicalisation yields a \\?\ prefix that reads badly in the
    // title bar and in tab chips; strip it back to a conventional path.
    let text = resolved.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(trimmed) => PathBuf::from(trimmed),
        None => resolved.to_path_buf(),
    })
}

fn workspace_label(workspace: &Path) -> String {
    workspace
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or("workspace")
        .to_owned()
}

/// AKR projects living next to `workspace`: every sibling directory holding a
/// `.akr` ledger, with the workspace itself first. Most checkouts keep all
/// their projects in one folder, so this is the project switcher.
fn discover_projects(workspace: &Path) -> Vec<PathBuf> {
    let Some(parent) = workspace.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut projects: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(".akr").is_dir())
        .collect();
    projects.sort();
    if let Some(position) = projects.iter().position(|path| path == workspace) {
        let current = projects.remove(position);
        projects.insert(0, current);
    }
    projects
}

fn mode_label(mode: TreeMode) -> &'static str {
    match mode {
        TreeMode::Planning => "Planning",
        TreeMode::Knowledge => "Knowledge",
    }
}

fn tree_rows(model: &AppModel) -> Vec<(i32, String)> {
    let Some(snapshot) = model.snapshot() else {
        return Vec::new();
    };
    let allowed = model
        .filtered_keys()
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    match model.mode {
        TreeMode::Planning => {
            fn visit(
                node: &akr_gui::model::TreeNode,
                depth: i32,
                allowed: &std::collections::BTreeSet<String>,
                out: &mut Vec<(i32, String)>,
            ) {
                if allowed.contains(&node.key) {
                    out.push((depth, node.key.clone()));
                }
                for child in &node.children {
                    visit(child, depth + 1, allowed, out);
                }
            }
            let mut out = Vec::new();
            for root in snapshot.planning_roots() {
                visit(&root, 0, &allowed, &mut out);
            }
            out
        }
        TreeMode::Knowledge => snapshot
            .knowledge_groups()
            .into_values()
            .flatten()
            .filter(|key| allowed.contains(key))
            .map(|key| (0, key))
            .collect(),
    }
}

// --- loading ----------------------------------------------------------------

#[derive(Clone)]
struct AkrLoader;
impl WorkspaceLoader for AkrLoader {
    fn load_reporting(
        &self,
        workspace: &Path,
        progress: &(dyn Fn(LoadPhase) + Sync),
    ) -> Result<ReviewSnapshot, SnapshotError> {
        use akr_cli::review_snapshot::ReviewPhase;
        let source = akr_cli::review_snapshot::ReviewSnapshot::load_reporting(
            workspace,
            akr_cli::review_snapshot::ReviewOptions::default(),
            &|phase| {
                progress(LoadPhase {
                    label: phase.label().to_owned(),
                    step: phase.step(),
                    total: ReviewPhase::ALL.len(),
                });
            },
        )
        .map_err(|error| SnapshotError {
            message: error.to_string(),
        })?;
        Ok(ReviewSnapshot {
            workspace: source.workspace,
            project: source.project,
            source_graph: source.source_graph,
            head: source.head,
            counts: ReviewCounts {
                records: source.counts.records,
                revisions: source.counts.revisions,
                stale: source.counts.stale,
                at_risk: source.counts.at_risk,
                diagnostics: source.counts.diagnostics,
                live_planning: source.counts.live_planning,
                open_questions: source.counts.open_questions,
            },
            diagnostics: source
                .diagnostics
                .into_iter()
                .map(|diagnostic| Diagnostic {
                    severity: diagnostic.severity,
                    code: diagnostic.code,
                    message: diagnostic.message,
                })
                .collect(),
            records: source
                .records
                .into_iter()
                .map(|record| {
                    let body = record
                        .slots
                        .iter()
                        .map(|field| format!("{}: {}", field.name, field.value))
                        .chain(
                            record
                                .claims
                                .iter()
                                .map(|claim| format!("claim #{}: {}", claim.anchor, claim.text)),
                        )
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    let relations = record
                        .relations
                        .iter()
                        .filter(|relation| relation.direction == "outbound")
                        .map(|relation| Relation {
                            kind: relation.relation.clone(),
                            target: relation
                                .record
                                .split_once('/')
                                .map_or_else(|| relation.record.clone(), |(key, _)| key.to_owned()),
                        })
                        .collect();
                    let provenance = record
                        .provenance
                        .iter()
                        .map(|source| {
                            let locator = source
                                .document
                                .as_deref()
                                .or(source.path.as_deref())
                                .or(source.url.as_deref())
                                .unwrap_or("unlocated");
                            match &source.range {
                                Some(range) => format!("{} {} {range}", source.kind, locator),
                                None => format!("{} {}", source.kind, locator),
                            }
                        })
                        .collect();
                    Record {
                        key: record.key,
                        title: record.title,
                        kind: record.kind,
                        state: record.state,
                        revision: record.revision,
                        body: if body.is_empty() { record.body } else { body },
                        freshness: record.freshness.status.clone(),
                        plan_of_record: !record.plan_for.is_empty(),
                        relations,
                        acceptance: record
                            .acceptance
                            .into_iter()
                            .map(|check| AcceptanceCheck {
                                id: check.id,
                                statement: check.statement,
                                verdict: check.verdict,
                            })
                            .collect(),
                        provenance,
                        history: record
                            .history
                            .into_iter()
                            .map(|revision| revision.id)
                            .collect(),
                        git: GitMetadata {
                            defined_at: record.defined_at,
                            observed_at: record.observed_at,
                            stale_cause: record.freshness.cause,
                        },
                    }
                })
                .collect(),
        })
    }
}

/// What a worker thread sends back: stage boundaries as they pass, then the
/// finished snapshot. Both carry the generation, so a superseded load's
/// messages are discarded rather than overwriting a newer one.
enum LoadMessage {
    Phase {
        tab: usize,
        generation: u64,
        phase: LoadPhase,
    },
    Done {
        tab: usize,
        generation: u64,
        result: Box<Result<ReviewSnapshot, SnapshotError>>,
    },
}

struct ReviewApp {
    model: AppModel,
    text: TextCache,
    kernel: RasterKernel,
    input: Input,
    theme: Theme,
    tree_table: TableState,
    out_table: TableState,
    panel_scroll: ScrollState,
    tree_visible_rows: i32,
    out_visible_rows: i32,
    filter_field: TextFieldState,
    filter_focused: bool,
    path_field: TextFieldState,
    projects: Vec<PathBuf>,
    project_table: TableState,
    modal_open: bool,
    scroll_drag: Option<ScrollDrag>,
    zoom: f32,
    status: Option<String>,
    pending_title: Option<String>,
    load_started: Option<Instant>,
    tick: u64,
    waker: Option<Waker>,
    loads: mpsc::Receiver<LoadMessage>,
    sender: mpsc::Sender<LoadMessage>,
    loader: Arc<dyn WorkspaceLoader>,
}

impl ReviewApp {
    fn new(workspaces: Vec<PathBuf>) -> Self {
        let (sender, loads) = mpsc::channel();
        let mut model = AppModel::default();
        for workspace in workspaces {
            model.add_tab(workspace);
        }
        if model.tabs.is_empty() {
            model.add_tab(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        }
        Self {
            model,
            text: TextCache::new(font_set()),
            kernel: RasterKernel::new(),
            input: Input::new(),
            theme: theme(),
            tree_table: TableState::new(),
            out_table: TableState::new(),
            panel_scroll: ScrollState::new(),
            tree_visible_rows: 10,
            out_visible_rows: 10,
            filter_field: TextFieldState::new(),
            filter_focused: false,
            path_field: TextFieldState::new(),
            projects: Vec::new(),
            project_table: TableState::new(),
            modal_open: false,
            scroll_drag: None,
            zoom: 1.0,
            status: None,
            pending_title: None,
            load_started: None,
            tick: 0,
            waker: None,
            loads,
            sender,
            loader: Arc::new(AkrLoader),
        }
    }

    fn request_load(&mut self, tab: usize) {
        let Some(workspace) = self.model.tabs.get_mut(tab).map(|tab| {
            tab.load_generation += 1;
            tab.error = None;
            tab.phase = None;
            tab.workspace.clone()
        }) else {
            return;
        };
        let generation = self.model.tabs[tab].load_generation;
        let sender = self.sender.clone();
        let loader = self.loader.clone();
        let waker = self.waker.clone();
        self.load_started = Some(Instant::now());
        std::thread::spawn(move || {
            let progress = sender.clone();
            let result = loader.load_reporting(&workspace, &|phase| {
                let _ = progress.send(LoadMessage::Phase {
                    tab,
                    generation,
                    phase,
                });
            });
            let _ = sender.send(LoadMessage::Done {
                tab,
                generation,
                result: Box::new(result),
            });
            if let Some(waker) = waker {
                waker.wake();
            }
        });
    }

    fn request_active_load(&mut self) {
        if let Some(tab) = self.model.active_tab {
            self.request_load(tab);
        }
    }

    /// Opens a workspace in a tab, reusing the tab if it is already open.
    fn open_workspace(&mut self, requested: &Path) {
        let path = match workspace_root(requested) {
            Ok(path) => path,
            Err(message) => {
                self.status = Some(message);
                return;
            }
        };
        self.tree_table = TableState::new();
        self.out_table = TableState::new();
        self.panel_scroll = ScrollState::new();
        if let Some(index) = self.model.tabs.iter().position(|tab| tab.workspace == path) {
            self.model.active_tab = Some(index);
            self.status = Some(format!("already open: {}", path.display()));
            self.sync_title();
            return;
        }
        let index = self.model.add_tab(path.clone());
        self.status = Some(format!("opened {}", path.display()));
        self.sync_title();
        self.request_load(index);
    }

    /// Opens the project switcher: every sibling AKR project, clickable, plus
    /// a path field seeded with the active workspace's parent so a project
    /// anywhere else is a short edit rather than a retype.
    fn begin_open_dialog(&mut self) {
        let workspace = self
            .model
            .active_tab
            .and_then(|index| self.model.tabs.get(index))
            .map(|tab| tab.workspace.clone());
        let seed = workspace
            .as_deref()
            .and_then(|workspace| workspace.parent())
            .map(|parent| {
                let text = parent.to_string_lossy().into_owned();
                if text.ends_with(['/', '\\']) {
                    text
                } else {
                    format!("{text}{}", std::path::MAIN_SEPARATOR)
                }
            })
            .unwrap_or_default();
        self.path_field = TextFieldState::with_text(seed);
        self.path_field.end();
        self.projects = workspace
            .as_deref()
            .map(discover_projects)
            .unwrap_or_default();
        self.project_table = TableState::new();
        self.scroll_drag = None;
        self.modal_open = true;
        self.filter_focused = false;
        self.status = None;
    }

    fn submit_open_prompt(&mut self) {
        self.modal_open = false;
        let typed = self.path_field.text().trim().trim_matches('"').to_owned();
        if typed.is_empty() {
            return;
        }
        self.open_workspace(Path::new(&typed));
    }

    fn close_active_workspace(&mut self) {
        let Some(index) = self.model.active_tab else {
            return;
        };
        let label = workspace_label(self.model.tabs[index].workspace.as_path());
        self.model.close_active_tab();
        self.tree_table = TableState::new();
        self.out_table = TableState::new();
        self.panel_scroll = ScrollState::new();
        self.status = Some(format!("closed {label}"));
        self.sync_title();
    }

    /// Mirrors the active workspace into the OS title bar, so the full path is
    /// visible even when the tab chip is truncated.
    fn sync_title(&mut self) {
        self.pending_title = Some(
            match self
                .model
                .active_tab
                .and_then(|index| self.model.tabs.get(index))
            {
                Some(tab) => format!("AKR Review — {}", tab.workspace.display()),
                None => "AKR Review".to_owned(),
            },
        );
    }

    fn switch_tab(&mut self, index: usize) {
        if self.model.active_tab != Some(index) {
            self.model.active_tab = Some(index);
            self.tree_table = TableState::new();
            self.out_table = TableState::new();
            self.panel_scroll = ScrollState::new();
            self.sync_title();
        }
    }

    fn poll_loads(&mut self) {
        while let Ok(message) = self.loads.try_recv() {
            let (index, generation) = match &message {
                LoadMessage::Phase {
                    tab, generation, ..
                }
                | LoadMessage::Done {
                    tab, generation, ..
                } => (*tab, *generation),
            };
            // A tab that was closed, or a load that a newer one superseded.
            if self
                .model
                .tabs
                .get(index)
                .is_none_or(|tab| tab.load_generation != generation)
            {
                continue;
            }
            match message {
                LoadMessage::Phase { phase, .. } => {
                    if let Some(tab) = self.model.tabs.get_mut(index) {
                        tab.phase = Some(phase);
                    }
                }
                LoadMessage::Done { result, .. } => {
                    let elapsed = self.load_started.take().map(|start| start.elapsed());
                    match *result {
                        Ok(snapshot) => {
                            let records = snapshot.records.len();
                            self.model.set_snapshot(index, snapshot);
                            self.status = Some(match elapsed {
                                Some(elapsed) => format!(
                                    "loaded {records} records in {:.1}s",
                                    elapsed.as_secs_f32()
                                ),
                                None => format!("loaded {records} records"),
                            });
                        }
                        Err(error) => {
                            self.model.fail_load(index, error.message);
                            self.status = Some("load failed".to_owned());
                        }
                    }
                    if let Some(tab) = self.model.tabs.get_mut(index) {
                        tab.phase = None;
                    }
                    self.sync_title();
                }
            }
        }
    }

    /// Whether any tab is still loading, which is also what decides between a
    /// ticking and a waiting event loop.
    fn is_loading(&self) -> bool {
        self.model
            .tabs
            .iter()
            .any(|tab| tab.snapshot.is_none() && tab.error.is_none())
    }
}

// --- frames ---------------------------------------------------------------

/// Work a frame decided on that needs `&mut ReviewApp`, applied once the
/// frame's widget borrows end.
#[derive(Default)]
struct Deferred {
    switch_tab: Option<usize>,
    sync_title: bool,
    open_dialog: bool,
    open_path: Option<PathBuf>,
    submit_modal: bool,
    cancel_modal: bool,
    close_tab: bool,
    reload_tab: bool,
    request_loads: Vec<usize>,
}

/// Which scrollable region a thumb drag belongs to. One drag at a time: the
/// pointer is either dragging a thumb or it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollTarget {
    Tree,
    Outstanding,
    Panel,
    Projects,
}

/// A scrollbar thumb under the pointer: the distance from the press point to
/// the thumb's top, so the thumb follows without jumping.
#[derive(Debug, Clone, Copy)]
struct ScrollDrag {
    target: ScrollTarget,
    grab: i32,
}

/// The frame's mutable pieces, split so panels can borrow them alongside the
/// [`Ui`] without fighting over `self`.
struct FrameState<'a> {
    model: &'a mut AppModel,
    tree_table: &'a mut TableState,
    out_table: &'a mut TableState,
    panel_scroll: &'a mut ScrollState,
    tree_visible_rows: &'a mut i32,
    out_visible_rows: &'a mut i32,
    filter_field: &'a mut TextFieldState,
    filter_focused: &'a mut bool,
    path_field: &'a mut TextFieldState,
    projects: &'a mut Vec<PathBuf>,
    project_table: &'a mut TableState,
    modal_open: &'a mut bool,
    scroll_drag: &'a mut Option<ScrollDrag>,
    zoom: &'a mut f32,
    status: &'a mut Option<String>,
    tick: u64,
    deferred: &'a mut Deferred,
}

struct Line {
    text: String,
    colour: u32,
}

/// Wraps `text` into display lines at `width` physical pixels.
fn wrapped_lines(ui: &mut Ui, text: &str, width: i32, colour: u32, out: &mut Vec<Line>) {
    let style = ui.theme.body;
    for block in text.split('\n') {
        if block.trim().is_empty() {
            out.push(Line {
                text: String::new(),
                colour,
            });
            continue;
        }
        for range in ui.text.wrap(block, width, style).iter() {
            out.push(Line {
                text: block[range.clone()].to_owned(),
                colour,
            });
        }
    }
}

fn scroll_lines(
    ui: &mut Ui,
    scroll: &mut ScrollState,
    slot: &mut Option<ScrollDrag>,
    area: Rect,
    lines: &[Line],
) {
    if lines.is_empty() {
        return;
    }
    let line_height = ui.text.line_height(ui.theme.body);
    let content = line_height * lines.len() as i32;
    if let Some(geom) = list_thumb(
        area,
        ui.px(LIST_GUTTER),
        ui.px(2),
        ui.px(18),
        content,
        scroll.offset(),
    ) {
        drag_thumb(ui.input, slot, ScrollTarget::Panel, scroll, &geom);
    }
    let heights = vec![line_height; lines.len()];
    ui.scroll_list(scroll, area, &heights, |ui, index, rect| {
        if !lines[index].text.is_empty() {
            ui.label(rect, &lines[index].text, lines[index].colour, Align::Left);
        }
    });
}

fn render_header(ui: &mut Ui, state: &mut FrameState, area: Rect) {
    ui.painter.fill_rect(area, ui.theme.panel);
    ui.painter.fill_rect(
        Rect::new(area.x, area.bottom() - 1, area.w, 1),
        ui.theme.panel_edge,
    );
    let pad = ui.px(12);
    let title_w = ui.px(150);
    ui.label(
        Rect::new(area.x + pad, area.y, title_w, area.h),
        "AKR REVIEW",
        ui.theme.accent,
        Align::Left,
    );
    let open_w = ui.px(110);
    let open = Rect::new(
        area.right() - open_w - pad,
        area.y + ui.px(8),
        open_w,
        area.h - ui.px(16),
    );
    if ui.button(open, "+ Open") {
        state.deferred.open_dialog = true;
    }
    let tabs_area = Rect::new(
        area.x + pad + title_w,
        area.y + ui.px(8),
        open.x - ui.px(8) - (area.x + pad + title_w),
        area.h - ui.px(16),
    );
    if state.model.tabs.is_empty() || tabs_area.w < ui.px(60) {
        return;
    }
    let labels: Vec<String> = state
        .model
        .tabs
        .iter()
        .map(|tab| workspace_label(tab.workspace.as_path()))
        .collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let mut selected = state.model.active_tab.unwrap_or(0).min(refs.len() - 1);
    if ui.tabs(&mut selected, tabs_area, &refs) {
        state.deferred.switch_tab = Some(selected);
    }
}

fn render_filter_bar(ui: &mut Ui, state: &mut FrameState, area: Rect, pressed: &[KeyInput]) {
    ui.painter.fill_rect(area, ui.theme.panel_alt);
    ui.painter.fill_rect(
        Rect::new(area.x, area.bottom() - 1, area.w, 1),
        ui.theme.panel_edge,
    );
    let pad = ui.px(12);
    let wide = area.w > ui.px(900);
    let count = state.model.outstanding_count();
    let items: Vec<(Panel, String, i32)> = if wide {
        vec![
            (Panel::Dashboard, "Dashboard".to_owned(), ui.px(120)),
            (
                Panel::Outstanding,
                format!("Outstanding ({count})"),
                ui.px(200),
            ),
            (Panel::Inspector, "Detail".to_owned(), ui.px(100)),
            (Panel::Relations, "Relations".to_owned(), ui.px(120)),
            (Panel::Git, "Git".to_owned(), ui.px(80)),
        ]
    } else {
        vec![
            (Panel::Dashboard, "D".to_owned(), ui.px(44)),
            (Panel::Outstanding, format!("O({count})"), ui.px(72)),
            (Panel::Inspector, "I".to_owned(), ui.px(44)),
            (Panel::Relations, "L".to_owned(), ui.px(44)),
            (Panel::Git, "G".to_owned(), ui.px(44)),
        ]
    };
    let mut right = area.right() - pad;
    for (panel, label, width) in items.iter().rev() {
        let rect = Rect::new(right - width, area.y + ui.px(8), *width, area.h - ui.px(16));
        if ui.button(rect, label) {
            state.model.panel = *panel;
            *state.panel_scroll = ScrollState::new();
        }
        if state.model.panel == *panel {
            ui.painter.fill_rect(
                Rect::new(rect.x, rect.bottom() - ui.px(3), rect.w, ui.px(3)),
                ui.theme.accent,
            );
        }
        right -= width + ui.px(8);
    }
    let field = Rect::new(
        area.x + pad,
        area.y + ui.px(8),
        (right - (area.x + pad)).max(ui.px(60)),
        area.h - ui.px(16),
    );
    if ui.input.take_click(field) {
        *state.filter_focused = true;
    }
    let mut changed = false;
    if *state.filter_focused {
        changed |= state.filter_field.apply(pressed);
    }
    changed |= ui.text_field(
        state.filter_field,
        field,
        "/ filter records",
        *state.filter_focused,
        false,
    );
    if changed {
        state.model.filter.query = state.filter_field.text().to_owned();
    }
}

/// Everything a thumb drag needs: the handle, its track, and the scroll
/// range it maps onto.
struct ThumbGeom {
    thumb: Rect,
    track_top: i32,
    max: i32,
    travel: i32,
    viewport: i32,
    content: i32,
}

/// A table's scrollbar thumb, mirroring pixelkit's indicator math so the drag
/// handle sits exactly on what the eye sees, or nothing when everything fits.
fn table_thumb(
    table_area: Rect,
    row_height: i32,
    row_count: usize,
    offset: i32,
) -> Option<ThumbGeom> {
    let body_h = table_area.h - row_height;
    let content = row_count as i32 * row_height;
    let max = ScrollState::max_offset(body_h, content);
    if max <= 0 || body_h <= 0 {
        return None;
    }
    let thumb_h = (body_h * body_h / content).max(20);
    let travel = body_h - thumb_h;
    let body_y = table_area.y + row_height;
    let top = body_y + travel * offset.clamp(0, max) / max;
    Some(ThumbGeom {
        thumb: Rect::new(table_area.right() - TABLE_GUTTER + 3, top, 4, thumb_h),
        track_top: body_y,
        max,
        travel,
        viewport: body_h,
        content,
    })
}

/// A scroll list's thumb, same contract as [`table_thumb`].
fn list_thumb(
    list_area: Rect,
    gutter: i32,
    inset: i32,
    min_thumb: i32,
    content: i32,
    offset: i32,
) -> Option<ThumbGeom> {
    if content <= list_area.h || list_area.h <= 0 {
        return None;
    }
    let max = ScrollState::max_offset(list_area.h, content);
    if max <= 0 {
        return None;
    }
    let thumb_h = ((list_area.h as i64 * list_area.h as i64) / content.max(1) as i64)
        .max(min_thumb as i64) as i32;
    let travel = (list_area.h - thumb_h).max(0);
    let top = list_area.y + (travel as i64 * offset.clamp(0, max) as i64 / max as i64) as i32;
    Some(ThumbGeom {
        thumb: Rect::new(
            list_area.right() - gutter + inset,
            top,
            (gutter - inset * 2).max(1),
            thumb_h.min(list_area.h),
        ),
        track_top: list_area.y,
        max,
        travel,
        viewport: list_area.h,
        content,
    })
}

/// Runs a scrollbar thumb drag for one region. Call before the region's widget
/// draws: a press on the thumb is claimed here, so the rows underneath never
/// see it, while a press anywhere else passes through untouched.
fn drag_thumb(
    input: &mut Input,
    slot: &mut Option<ScrollDrag>,
    target: ScrollTarget,
    scroll: &mut ScrollState,
    geom: &ThumbGeom,
) {
    match slot {
        Some(drag) if drag.target == target => {
            if input.is_held() {
                if geom.travel > 0
                    && let Some((_, cursor_y)) = input.cursor()
                {
                    let wanted = (cursor_y - drag.grab - geom.track_top) * geom.max / geom.travel;
                    scroll.scroll_by(
                        wanted.clamp(0, geom.max) - scroll.offset(),
                        geom.viewport,
                        geom.content,
                    );
                }
            } else {
                *slot = None;
            }
            return;
        }
        // Another region's live drag owns the pointer; a stale one is dead.
        Some(_) if input.is_held() => return,
        _ => {}
    }
    *slot = None;
    // A press still pending is a fresh press: a live drag elsewhere consumed
    // its own press when it began, so anything pending in this thumb starts a
    // drag, and anything else passes through to the rows underneath.
    if let Some((_, press_y)) = input.press_position() {
        // `take_click` only consumes a press inside the thumb; anything else
        // passes through to the rows underneath.
        if input.take_click(geom.thumb) {
            *slot = Some(ScrollDrag {
                target,
                grab: press_y - geom.thumb.y,
            });
        }
    }
}

/// Move the keyboard selection and keep it on screen, using the viewport the
/// table reported last frame.
fn keyboard_navigate(
    table: &mut TableState,
    keys: &[String],
    visible_rows: i32,
    row_height: i32,
    pressed: &[KeyInput],
) -> bool {
    if !navigate(table, keys, visible_rows.max(1), pressed) {
        return false;
    }
    if let Some(index) = table.selected_index(keys) {
        table.scroll.reveal(
            index as i32 * row_height,
            row_height,
            visible_rows.max(1) * row_height,
            keys.len() as i32 * row_height,
        );
    }
    true
}

fn render_left(
    ui: &mut Ui,
    state: &mut FrameState,
    area: Rect,
    pressed: &[KeyInput],
    nav_active: bool,
) {
    ui.painter.fill_rect(area, ui.theme.panel);
    ui.painter.fill_rect(
        Rect::new(area.right() - 1, area.y, 1, area.h),
        ui.theme.panel_edge,
    );
    let pad = ui.px(12);
    let rows = tree_rows(state.model);
    let head = Rect::new(area.x + pad, area.y + ui.px(6), area.w - pad * 2, ui.px(36));
    ui.label(
        Rect::new(head.x, head.y, head.w - ui.px(130), head.h),
        &format!("{} records", rows.len()),
        ui.theme.text_dim,
        Align::Left,
    );
    let mode_rect = Rect::new(head.right() - ui.px(120), head.y, ui.px(120), head.h);
    if ui.button(mode_rect, mode_label(state.model.mode)) {
        state.model.mode = match state.model.mode {
            TreeMode::Planning => TreeMode::Knowledge,
            TreeMode::Knowledge => TreeMode::Planning,
        };
        *state.tree_table = TableState::new();
    }
    let table_area = Rect::new(
        area.x + ui.px(4),
        head.bottom() + ui.px(4),
        area.w - ui.px(8),
        (area.bottom() - ui.px(4)) - (head.bottom() + ui.px(4)),
    );
    if rows.is_empty() {
        let reason = match state
            .model
            .active_tab
            .and_then(|index| state.model.tabs.get(index))
        {
            None => "no workspace open",
            Some(tab) if tab.error.is_some() => "load failed",
            Some(tab) if tab.snapshot.is_none() => "loading...",
            Some(_) => "no records match",
        };
        ui.label(table_area, reason, ui.theme.text_dim, Align::Left);
        return;
    }
    let keys: Vec<String> = rows.iter().map(|(_, key)| key.clone()).collect();
    state
        .tree_table
        .select(state.model.selected().map(|record| record.key.clone()));
    if nav_active
        && keyboard_navigate(
            state.tree_table,
            &keys,
            *state.tree_visible_rows,
            ui.theme.row_height,
            pressed,
        )
        && let Some(selected) = state.tree_table.selected()
    {
        let selected = selected.to_owned();
        state.model.select_quiet(selected);
    }
    if let Some(geom) = table_thumb(
        table_area,
        ui.theme.row_height,
        keys.len(),
        state.tree_table.scroll.offset(),
    ) {
        drag_thumb(
            ui.input,
            state.scroll_drag,
            ScrollTarget::Tree,
            &mut state.tree_table.scroll,
            &geom,
        );
    }
    let snapshot = state.model.snapshot();
    let columns = [Column::flex("RECORD", 1, Align::Left)];
    let outcome: TableOutcome = ui.table(state.tree_table, table_area, &columns, &keys, |index| {
        let (depth, key) = &rows[index];
        let indent = "  ".repeat((*depth as usize).min(8));
        let text = match snapshot.and_then(|snapshot| snapshot.record(key)) {
            Some(record) => format!("{indent}{}  {}", record.kind, record.title),
            None => format!("{indent}{key}"),
        };
        Row {
            cells: vec![Cell::new(text)],
        }
    });
    *state.tree_visible_rows = outcome.visible_rows.max(1);
    if let Some(key) = outcome.activated {
        state.model.select(key);
        *state.panel_scroll = ScrollState::new();
    }
}

/// Draws the no-workspace and load-error states shared by every panel.
/// Returns true when it drew something terminal and the panel should stop.
fn render_terminal(ui: &mut Ui, state: &mut FrameState, area: Rect) -> bool {
    let pad = ui.px(12);
    let inner = Rect::new(
        area.x + pad,
        area.y + pad,
        area.w - pad * 2,
        area.h - pad * 2,
    );
    let Some(tab) = state
        .model
        .active_tab
        .and_then(|index| state.model.tabs.get(index))
    else {
        let mut lines = Vec::new();
        lines.push(Line {
            text: "No workspace open.".to_owned(),
            colour: ui.theme.text,
        });
        lines.push(Line {
            text: String::new(),
            colour: ui.theme.text_dim,
        });
        wrapped_lines(
            ui,
            "Press O and type a project directory, or click + Open in the title bar. \
             A path inside a project resolves to the directory holding its .akr folder.",
            inner.w,
            ui.theme.text_dim,
            &mut lines,
        );
        scroll_lines(ui, state.panel_scroll, state.scroll_drag, inner, &lines);
        return true;
    };
    let Some(error) = tab.error.clone() else {
        return false;
    };
    let mut lines = vec![Line {
        text: "LOAD FAILED".to_owned(),
        colour: ui.theme.alarm,
    }];
    lines.push(Line {
        text: String::new(),
        colour: ui.theme.text_dim,
    });
    wrapped_lines(ui, &error, inner.w, ui.theme.text, &mut lines);
    scroll_lines(ui, state.panel_scroll, state.scroll_drag, inner, &lines);
    true
}

fn render_loading(ui: &mut Ui, state: &mut FrameState, area: Rect, elapsed: f32) {
    const MARKERS: [&str; 4] = ["|", "/", "-", "\\"];
    let marker = MARKERS[(state.tick / 8) as usize % MARKERS.len()];
    let pad = ui.px(12);
    let mut y = area.y + pad;
    let full = Rect::new(area.x + pad, y, area.w - pad * 2, ui.px(34));
    ui.heading(full, &format!("{marker} Loading workspace"));
    y = full.bottom() + ui.px(8);
    let (path, step, total, label) = match state
        .model
        .active_tab
        .and_then(|index| state.model.tabs.get(index))
    {
        Some(tab) => {
            let (step, total, label) = match &tab.phase {
                Some(phase) => (phase.step, phase.total, phase.label.clone()),
                None => (0, 5, "starting".to_owned()),
            };
            (tab.workspace.display().to_string(), step, total, label)
        }
        None => (String::new(), 0, 5, "starting".to_owned()),
    };
    let row = Rect::new(area.x + pad, y, area.w - pad * 2, ui.px(24));
    ui.label(row, &path, ui.theme.text_dim, Align::Left);
    y = row.bottom() + ui.px(12);
    let step_row = Rect::new(area.x + pad, y, area.w - pad * 2, ui.px(24));
    ui.label(
        step_row,
        &format!("step {step} of {total}: {label}"),
        ui.theme.text,
        Align::Left,
    );
    y = step_row.bottom() + ui.px(8);
    let bar = Rect::new(
        area.x + pad,
        y,
        (area.w - pad * 2).min(ui.px(480)),
        ui.px(12),
    );
    let fraction = if total == 0 {
        0.0
    } else {
        step.min(total) as f64 / total as f64
    };
    ui.bar(bar, fraction, ui.theme.accent);
    y = bar.bottom() + ui.px(8);
    let elapsed_row = Rect::new(area.x + pad, y, area.w - pad * 2, ui.px(24));
    ui.label(
        elapsed_row,
        &format!("{elapsed:.1}s elapsed"),
        ui.theme.text_dim,
        Align::Left,
    );
    y = elapsed_row.bottom() + ui.px(12);
    let mut lines = Vec::new();
    wrapped_lines(
        ui,
        "Most of this is git: freshness asks the repository which commits touched \
         each watched path. Large histories take longer, and the answer is cached \
         for the rest of the load.",
        area.w - pad * 2,
        ui.theme.text_dim,
        &mut lines,
    );
    scroll_lines(
        ui,
        state.panel_scroll,
        state.scroll_drag,
        Rect::new(area.x + pad, y, area.w - pad * 2, area.bottom() - pad - y),
        &lines,
    );
}

fn render_dashboard(ui: &mut Ui, state: &mut FrameState, area: Rect) {
    if render_terminal(ui, state, area) {
        return;
    }
    let pad = ui.px(12);
    let mut y = area.y + pad;
    let row = |y: i32, height: i32| Rect::new(area.x + pad, y, area.w - pad * 2, height);
    let Some(snapshot) = state.model.snapshot() else {
        ui.label(
            row(y, ui.px(24)),
            "Loading review snapshot...",
            ui.theme.text_dim,
            Align::Left,
        );
        return;
    };
    let project = snapshot.project.clone();
    let counts = snapshot.counts;
    let outstanding = snapshot.outstanding_count();
    let source_graph = snapshot.source_graph.clone();
    let head = snapshot.head.clone();
    let diagnostics = snapshot.diagnostics.clone();
    ui.heading(row(y, ui.px(34)), &project);
    y += ui.px(34) + ui.px(8);
    ui.label(
        row(y, ui.px(24)),
        &format!(
            "{} records / {} revisions",
            counts.records, counts.revisions
        ),
        ui.theme.text,
        Align::Left,
    );
    y += ui.px(24);
    let jump = Rect::new(
        area.x + pad,
        y + ui.px(4),
        (area.w - pad * 2).min(ui.px(460)),
        ui.px(36),
    );
    if ui.button(jump, &format!("{outstanding} outstanding — review them")) {
        state.model.panel = Panel::Outstanding;
        *state.panel_scroll = ScrollState::new();
    }
    y = jump.bottom() + ui.px(8);
    ui.label(
        row(y, ui.px(24)),
        &format!(
            "{} live planning / {} open questions",
            counts.live_planning, counts.open_questions
        ),
        ui.theme.text,
        Align::Left,
    );
    y += ui.px(24);
    let bad = counts.stale + counts.at_risk + counts.diagnostics > 0;
    ui.label(
        row(y, ui.px(24)),
        &format!(
            "{} stale / {} at risk / {} diagnostics",
            counts.stale, counts.at_risk, counts.diagnostics
        ),
        if bad {
            ui.theme.alarm
        } else {
            ui.theme.text_dim
        },
        Align::Left,
    );
    y += ui.px(24) + ui.px(12);
    ui.label(
        row(y, ui.px(24)),
        &format!("source graph  {source_graph}"),
        ui.theme.text_dim,
        Align::Left,
    );
    y += ui.px(24);
    if let Some(head) = head {
        ui.label(
            row(y, ui.px(24)),
            &format!("git HEAD      {head}"),
            ui.theme.text_dim,
            Align::Left,
        );
        y += ui.px(24);
    }
    y += ui.px(8);
    ui.label(
        row(y, ui.px(28)),
        "DIAGNOSTICS",
        ui.theme.accent,
        Align::Left,
    );
    y += ui.px(28);
    let mut lines = Vec::new();
    if diagnostics.is_empty() {
        lines.push(Line {
            text: "none".to_owned(),
            colour: ui.theme.text_dim,
        });
    }
    for diagnostic in &diagnostics {
        wrapped_lines(
            ui,
            &format!(
                "{} {}  {}",
                diagnostic.severity, diagnostic.code, diagnostic.message
            ),
            area.w - pad * 2,
            if diagnostic.severity == "error" {
                ui.theme.alarm
            } else {
                ui.theme.text_dim
            },
            &mut lines,
        );
    }
    scroll_lines(
        ui,
        state.panel_scroll,
        state.scroll_drag,
        Rect::new(
            area.x + pad,
            y,
            area.w - pad * 2,
            (area.bottom() - pad - y).max(0),
        ),
        &lines,
    );
}

fn render_outstanding(
    ui: &mut Ui,
    state: &mut FrameState,
    area: Rect,
    pressed: &[KeyInput],
    nav_active: bool,
) {
    if render_terminal(ui, state, area) {
        return;
    }
    let pad = ui.px(12);
    let keys = state.model.outstanding_keys();
    let title = Rect::new(area.x + pad, area.y + pad, area.w - pad * 2, ui.px(28));
    if keys.is_empty() {
        ui.label(
            title,
            "Nothing outstanding — no proposed records.",
            ui.theme.good,
            Align::Left,
        );
        let mut lines = Vec::new();
        wrapped_lines(
            ui,
            "Every entry in this project has been accepted, completed, or withdrawn. \
             The filter above still applies: clear it if you expected to see more.",
            area.w - pad * 2,
            ui.theme.text_dim,
            &mut lines,
        );
        scroll_lines(
            ui,
            state.panel_scroll,
            state.scroll_drag,
            Rect::new(
                area.x + pad,
                title.bottom() + ui.px(8),
                area.w - pad * 2,
                (area.bottom() - pad - (title.bottom() + ui.px(8))).max(0),
            ),
            &lines,
        );
        return;
    }
    ui.label(
        title,
        &format!(
            "{} outstanding — proposed, awaiting review. Click one to inspect it.",
            keys.len()
        ),
        ui.theme.text,
        Align::Left,
    );
    let table_area = Rect::new(
        area.x + pad,
        title.bottom() + ui.px(8),
        area.w - pad * 2,
        (area.bottom() - pad - (title.bottom() + ui.px(8))).max(0),
    );
    state
        .out_table
        .select(state.model.selected().map(|record| record.key.clone()));
    if nav_active
        && keyboard_navigate(
            state.out_table,
            &keys,
            *state.out_visible_rows,
            ui.theme.row_height,
            pressed,
        )
        && let Some(selected) = state.out_table.selected()
    {
        let selected = selected.to_owned();
        state.model.select_quiet(selected);
    }
    if let Some(geom) = table_thumb(
        table_area,
        ui.theme.row_height,
        keys.len(),
        state.out_table.scroll.offset(),
    ) {
        drag_thumb(
            ui.input,
            state.scroll_drag,
            ScrollTarget::Outstanding,
            &mut state.out_table.scroll,
            &geom,
        );
    }
    let snapshot = state.model.snapshot();
    let columns = [
        Column::fixed("KIND", ui.px(110), Align::Left),
        Column::flex("KEY", 2, Align::Left),
        Column::flex("TITLE", 3, Align::Left),
        Column::fixed("FRESHNESS", ui.px(120), Align::Left),
    ];
    let theme = ui.theme;
    let outcome: TableOutcome = ui.table(state.out_table, table_area, &columns, &keys, |index| {
        let key = &keys[index];
        let record = snapshot.and_then(|snapshot| snapshot.record(key));
        let (kind, title, freshness) = match record {
            Some(record) => (
                record.kind.as_str(),
                record.title.as_str(),
                record.freshness.as_str(),
            ),
            None => ("", key.as_str(), ""),
        };
        let fresh_cell = match freshness {
            "current" => Cell::badge(freshness, theme.good, theme.good_soft, theme.good),
            "stale" | "at risk" | "at-risk" => {
                Cell::badge(freshness, theme.alarm, theme.alarm_soft, theme.alarm)
            }
            _ => Cell::new(freshness),
        };
        Row {
            cells: vec![
                Cell::coloured(kind, theme.text_dim),
                Cell::new(key.as_str()),
                Cell::new(title),
                fresh_cell,
            ],
        }
    });
    *state.out_visible_rows = outcome.visible_rows.max(1);
    if let Some(key) = outcome.activated {
        state.model.select(key);
        *state.panel_scroll = ScrollState::new();
    }
}

fn render_inspector(ui: &mut Ui, state: &mut FrameState, area: Rect) {
    if render_terminal(ui, state, area) {
        return;
    }
    let pad = ui.px(12);
    let inner = Rect::new(
        area.x + pad,
        area.y + pad,
        area.w - pad * 2,
        area.h - pad * 2,
    );
    let Some(record) = state.model.selected().cloned() else {
        ui.label(
            inner,
            "Select a record to inspect",
            ui.theme.text_dim,
            Align::Left,
        );
        return;
    };
    let accent = ui.theme.accent;
    let text = ui.theme.text;
    let dim = ui.theme.text_dim;
    let alarm = ui.theme.alarm;
    let mut lines = vec![
        Line {
            text: record.key.clone(),
            colour: accent,
        },
        Line {
            text: format!(
                "{} / {} / r{} / freshness: {}",
                record.kind, record.state, record.revision, record.freshness
            ),
            colour: text,
        },
        Line {
            text: String::new(),
            colour: dim,
        },
    ];
    wrapped_lines(ui, &record.title, inner.w, text, &mut lines);
    lines.push(Line {
        text: String::new(),
        colour: dim,
    });
    wrapped_lines(ui, &record.body, inner.w, dim, &mut lines);
    lines.push(Line {
        text: String::new(),
        colour: dim,
    });
    lines.push(Line {
        text: "ACCEPTANCE".to_owned(),
        colour: accent,
    });
    if record.acceptance.is_empty() {
        lines.push(Line {
            text: "none".to_owned(),
            colour: dim,
        });
    }
    for check in &record.acceptance {
        let colour = if check.verdict.starts_with("satisfied") {
            dim
        } else {
            alarm
        };
        wrapped_lines(
            ui,
            &format!("{}  {}  {}", check.id, check.verdict, check.statement),
            inner.w,
            colour,
            &mut lines,
        );
    }
    lines.push(Line {
        text: String::new(),
        colour: dim,
    });
    lines.push(Line {
        text: "REVISION HISTORY".to_owned(),
        colour: accent,
    });
    if record.history.is_empty() {
        lines.push(Line {
            text: "none".to_owned(),
            colour: dim,
        });
    }
    for revision in &record.history {
        lines.push(Line {
            text: revision.clone(),
            colour: dim,
        });
    }
    scroll_lines(ui, state.panel_scroll, state.scroll_drag, inner, &lines);
}

enum RelationRow {
    Header(String),
    Outbound {
        kind: String,
        target: String,
    },
    Edge {
        from: String,
        kind: String,
        to: String,
    },
}

fn render_relations(ui: &mut Ui, state: &mut FrameState, area: Rect) {
    if render_terminal(ui, state, area) {
        return;
    }
    let pad = ui.px(12);
    let inner = Rect::new(
        area.x + pad,
        area.y + pad,
        area.w - pad * 2,
        area.h - pad * 2,
    );
    let Some(record) = state.model.selected().cloned() else {
        ui.label(
            inner,
            "Select a record to inspect relations",
            ui.theme.text_dim,
            Align::Left,
        );
        return;
    };
    let current = record.key.clone();
    let mut rows = vec![RelationRow::Header(current.clone())];
    rows.push(RelationRow::Header("OUTBOUND RELATIONS".to_owned()));
    for relation in &record.relations {
        rows.push(RelationRow::Outbound {
            kind: relation.kind.clone(),
            target: relation.target.clone(),
        });
    }
    rows.push(RelationRow::Header(
        "LOCAL NEIGHBORHOOD (2 HOPS / 12 NODES)".to_owned(),
    ));
    let neighbors = state
        .model
        .snapshot()
        .map(|snapshot| snapshot.neighborhood(&current, 2, 12))
        .unwrap_or_default();
    for edge in &neighbors {
        rows.push(RelationRow::Edge {
            from: edge.from.clone(),
            kind: edge.kind.clone(),
            to: edge.to.clone(),
        });
    }
    let row_height = ui.theme.row_height + ui.px(6);
    let heights = vec![row_height; rows.len()];
    let content: i32 = heights.iter().sum();
    if let Some(geom) = list_thumb(
        inner,
        ui.px(LIST_GUTTER),
        ui.px(2),
        ui.px(18),
        content,
        state.panel_scroll.offset(),
    ) {
        drag_thumb(
            ui.input,
            state.scroll_drag,
            ScrollTarget::Panel,
            state.panel_scroll,
            &geom,
        );
    }
    let mut goto: Option<String> = None;
    ui.scroll_list(
        state.panel_scroll,
        inner,
        &heights,
        |ui, index, rect| match &rows[index] {
            RelationRow::Header(text) => {
                ui.label(rect, text, ui.theme.accent, Align::Left);
            }
            RelationRow::Outbound { kind, target } => {
                if ui.button(rect, &format!("{kind} -> {target}")) {
                    goto = Some(target.clone());
                }
            }
            RelationRow::Edge { from, kind, to } => {
                if ui.button(rect, &format!("{from} -{kind}-> {to}")) {
                    goto = Some(if from == &current {
                        to.clone()
                    } else {
                        from.clone()
                    });
                }
            }
        },
    );
    if let Some(key) = goto {
        let known = state
            .model
            .snapshot()
            .and_then(|snapshot| snapshot.record(&key))
            .is_some();
        if known {
            state.model.select(key);
            *state.panel_scroll = ScrollState::new();
        } else {
            *state.status = Some(format!("no such record in this workspace: {key}"));
        }
    }
}

fn render_git(ui: &mut Ui, state: &mut FrameState, area: Rect) {
    if render_terminal(ui, state, area) {
        return;
    }
    let pad = ui.px(12);
    let inner = Rect::new(
        area.x + pad,
        area.y + pad,
        area.w - pad * 2,
        area.h - pad * 2,
    );
    let Some(record) = state.model.selected().cloned() else {
        ui.label(
            inner,
            "Select a record to inspect provenance",
            ui.theme.text_dim,
            Align::Left,
        );
        return;
    };
    let accent = ui.theme.accent;
    let text = ui.theme.text;
    let dim = ui.theme.text_dim;
    let mut lines = vec![
        Line {
            text: record.key.clone(),
            colour: accent,
        },
        Line {
            text: "GIT".to_owned(),
            colour: accent,
        },
        Line {
            text: format!(
                "defined at   {}",
                record.git.defined_at.as_deref().unwrap_or("uncommitted")
            ),
            colour: text,
        },
        Line {
            text: format!(
                "observed at  {}",
                record.git.observed_at.as_deref().unwrap_or("not empirical")
            ),
            colour: text,
        },
        Line {
            text: format!(
                "stale cause  {}",
                record.git.stale_cause.as_deref().unwrap_or("none")
            ),
            colour: text,
        },
        Line {
            text: String::new(),
            colour: dim,
        },
        Line {
            text: "PROVENANCE".to_owned(),
            colour: accent,
        },
    ];
    if record.provenance.is_empty() {
        lines.push(Line {
            text: "none".to_owned(),
            colour: dim,
        });
    }
    for value in &record.provenance {
        wrapped_lines(ui, value, inner.w, dim, &mut lines);
    }
    scroll_lines(ui, state.panel_scroll, state.scroll_drag, inner, &lines);
}

fn render_footer(ui: &mut Ui, state: &mut FrameState, area: Rect, elapsed: f32) {
    ui.painter.fill_rect(area, ui.theme.panel);
    ui.painter
        .fill_rect(Rect::new(area.x, area.y, area.w, 1), ui.theme.panel_edge);
    let pad = ui.px(12);
    let zoom = format!("zoom {}%", (*state.zoom * 100.0).round() as i32);
    let zoom_w = ui.px(130);
    ui.label(
        Rect::new(area.right() - zoom_w - pad, area.y, zoom_w, area.h),
        &zoom,
        ui.theme.text_dim,
        Align::Right,
    );
    let hints = "O open   W close   Tab switch   / filter   P/K tree   D U I L G panels   R reload   +/- zoom";
    let loading_line = state
        .model
        .active_tab
        .and_then(|index| state.model.tabs.get(index))
        .filter(|tab| tab.snapshot.is_none() && tab.error.is_none())
        .map(|tab| match &tab.phase {
            Some(phase) => format!(
                "loading {}/{}  {}  {elapsed:.1}s",
                phase.step, phase.total, phase.label
            ),
            None => format!("loading  {elapsed:.1}s"),
        });
    let (text, colour) = if let Some(line) = loading_line {
        (line, ui.theme.text)
    } else if let Some(status) = state.status.clone() {
        (status, ui.theme.text)
    } else {
        (hints.to_owned(), ui.theme.text_dim)
    };
    ui.label(
        Rect::new(area.x + pad, area.y, area.w - pad * 3 - zoom_w, area.h),
        &text,
        colour,
        Align::Left,
    );
}

const DIALOG_W: i32 = 620;
const DIALOG_H: i32 = 460;

fn render_open_dialog(ui: &mut Ui, state: &mut FrameState, pressed: &[KeyInput]) {
    let full = Rect::new(0, 0, ui.painter.width(), ui.painter.height());
    let dialog = Rect::new(
        full.x + (full.w - ui.px(DIALOG_W)) / 2,
        full.y + (full.h - ui.px(DIALOG_H)) / 2,
        ui.px(DIALOG_W).min(full.w - ui.px(32)),
        ui.px(DIALOG_H).min(full.h - ui.px(32)),
    );
    // A shadow, then the dialog, so it reads as above the workbench.
    ui.painter.fill_rect(
        Rect::new(dialog.x + ui.px(6), dialog.y + ui.px(6), dialog.w, dialog.h),
        0x0000_0000,
    );
    ui.painter.rounded_rect(
        dialog,
        ui.theme.corner_radius,
        ui.theme.border_width,
        ui.theme.panel,
        ui.theme.accent,
    );
    let pad = ui.px(16);
    let mut y = dialog.y + pad;
    ui.heading(
        Rect::new(dialog.x + pad, y, dialog.w - pad * 2, ui.px(30)),
        "Open project",
    );
    y += ui.px(30) + ui.px(8);
    // The bottom block has a fixed height; the project list takes the rest.
    let bottom_h = ui.px(22) + ui.px(8) + ui.px(40) + ui.px(8) + ui.px(24) + ui.px(8) + ui.px(36);
    let list_label = Rect::new(dialog.x + pad, y, dialog.w - pad * 2, ui.px(22));
    y += ui.px(22) + ui.px(4);
    let list_area = Rect::new(
        dialog.x + pad,
        y,
        dialog.w - pad * 2,
        (dialog.bottom() - pad - bottom_h - y).max(ui.px(60)),
    );
    y = list_area.bottom() + ui.px(8);
    let field_label = Rect::new(dialog.x + pad, y, dialog.w - pad * 2, ui.px(22));
    y += ui.px(22) + ui.px(8);
    let field = Rect::new(dialog.x + pad, y, dialog.w - pad * 2, ui.px(40));
    y = field.bottom() + ui.px(8);
    let hint = Rect::new(dialog.x + pad, y, dialog.w - pad * 2, ui.px(24));
    let cancel = Rect::new(
        dialog.right() - pad - ui.px(110),
        dialog.bottom() - pad - ui.px(36),
        ui.px(110),
        ui.px(36),
    );
    let open = Rect::new(
        cancel.x - ui.px(8) - ui.px(110),
        cancel.y,
        ui.px(110),
        ui.px(36),
    );

    if state.projects.is_empty() {
        ui.label(
            list_label,
            "No AKR projects beside this one — type any path below.",
            ui.theme.text_dim,
            Align::Left,
        );
    } else {
        ui.label(
            list_label,
            "AKR projects next to this one — click one to open it.",
            ui.theme.text_dim,
            Align::Left,
        );
        let keys: Vec<String> = state
            .projects
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        if let Some(geom) = table_thumb(
            list_area,
            ui.theme.row_height,
            keys.len(),
            state.project_table.scroll.offset(),
        ) {
            drag_thumb(
                ui.input,
                state.scroll_drag,
                ScrollTarget::Projects,
                &mut state.project_table.scroll,
                &geom,
            );
        }
        let open_workspaces: Vec<PathBuf> = state
            .model
            .tabs
            .iter()
            .map(|tab| tab.workspace.clone())
            .collect();
        let projects = state.projects.clone();
        let columns = [
            Column::flex("PROJECT", 1, Align::Left),
            Column::flex("LOCATION", 2, Align::Left),
        ];
        let theme = ui.theme;
        let outcome = ui.table(state.project_table, list_area, &columns, &keys, |index| {
            let path = &projects[index];
            let name = workspace_label(path);
            let is_open = open_workspaces.iter().any(|open| open == path);
            Row {
                cells: vec![
                    if is_open {
                        Cell::coloured(format!("{name}  (open)"), theme.good)
                    } else {
                        Cell::new(name)
                    },
                    Cell::coloured(path.to_string_lossy(), theme.text_dim),
                ],
            }
        });
        if let Some(key) = outcome.activated {
            state.deferred.open_path = Some(PathBuf::from(key));
        }
    }

    ui.label(
        field_label,
        "Or open any path:",
        ui.theme.text_dim,
        Align::Left,
    );
    state.path_field.apply(pressed);
    ui.text_field(state.path_field, field, "project directory", true, false);
    ui.label(
        hint,
        "Click a project to open it · Enter opens the typed path · Esc cancels",
        ui.theme.text_dim,
        Align::Left,
    );
    if ui.button(open, "Open") {
        state.deferred.submit_modal = true;
    }
    if ui.button(cancel, "Cancel") {
        state.deferred.cancel_modal = true;
    }
    for key in pressed {
        match key {
            KeyInput::Enter => state.deferred.submit_modal = true,
            KeyInput::Escape => state.deferred.cancel_modal = true,
            _ => {}
        }
    }
}

/// Single-letter shortcuts. Runs only when no field owns the keyboard and no
/// dialog is open.
fn shortcuts(state: &mut FrameState, pressed: &[KeyInput]) {
    for key in pressed {
        match key {
            KeyInput::Character(value) if *value == '/' => *state.filter_focused = true,
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'o') => {
                state.deferred.open_dialog = true;
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'w') => {
                state.deferred.close_tab = true;
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'r') => {
                state.deferred.reload_tab = true;
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'p') => {
                state.model.mode = TreeMode::Planning;
                *state.tree_table = TableState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'k') => {
                state.model.mode = TreeMode::Knowledge;
                *state.tree_table = TableState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'d') => {
                state.model.panel = Panel::Dashboard;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'u') => {
                state.model.panel = Panel::Outstanding;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'i') => {
                state.model.panel = Panel::Inspector;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'l') => {
                state.model.panel = Panel::Relations;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Character(value) if value.eq_ignore_ascii_case(&'g') => {
                state.model.panel = Panel::Git;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Character(value) if matches!(*value, '+' | '=') => {
                *state.zoom = (*state.zoom * 1.25).clamp(0.5, 3.0);
            }
            KeyInput::Character(value) if matches!(*value, '-' | '_') => {
                *state.zoom = (*state.zoom / 1.25).clamp(0.5, 3.0);
            }
            KeyInput::Character(value) if *value == '0' => *state.zoom = 1.0,
            KeyInput::Tab => {
                state.model.cycle_tab();
                *state.tree_table = TableState::new();
                *state.out_table = TableState::new();
                *state.panel_scroll = ScrollState::new();
                state.deferred.sync_title = true;
            }
            KeyInput::Enter if state.model.panel == Panel::Outstanding => {
                state.model.panel = Panel::Inspector;
                *state.panel_scroll = ScrollState::new();
            }
            KeyInput::Escape => *state.filter_focused = false,
            _ => {}
        }
    }
}

impl ReviewApp {
    /// One frame, headless-friendly: everything it needs is a buffer and a
    /// scale, so tests drive the same code the window host does.
    fn frame(&mut self, buffer: &mut WindowBuffer, host_scale: Scale) {
        self.poll_loads();
        self.tick += 1;
        let scale = Scale(host_scale.0 * self.zoom);
        let full = Rect::new(0, 0, buffer.width as i32, buffer.height as i32);
        if self.input.modifiers().control {
            let (_, dy) = self.input.take_scroll(full);
            if dy != 0.0 {
                self.zoom *= if dy > 0.0 { 1.1 } else { 1.0 / 1.1 };
                self.zoom = self.zoom.clamp(0.5, 3.0);
            }
        }
        let mut theme = self.theme;
        theme.row_height = scale.px(theme.row_height);
        theme.padding = scale.px(theme.padding);
        theme.corner_radius = scale.px(theme.corner_radius);
        let mut deferred = Deferred::default();
        let elapsed = self
            .load_started
            .map_or(0.0, |start| start.elapsed().as_secs_f32());
        {
            let ReviewApp {
                model,
                text,
                kernel,
                input,
                tree_table,
                out_table,
                panel_scroll,
                tree_visible_rows,
                out_visible_rows,
                filter_field,
                filter_focused,
                path_field,
                projects,
                project_table,
                modal_open,
                scroll_drag,
                zoom,
                status,
                tick,
                ..
            } = self;
            let tick = *tick;
            let mut ui = Ui::new(Painter::new(buffer), text, input, theme, scale, kernel);
            ui.painter.clear(ui.theme.background);
            let logical_w = scale.logical(ui.painter.width()) as i32;
            let logical_h = scale.logical(ui.painter.height()) as i32;
            let areas = layout_areas(logical_w, logical_h);
            let physical = |rect: Rect| {
                Rect::new(
                    scale.px(rect.x),
                    scale.px(rect.y),
                    scale.px(rect.w),
                    scale.px(rect.h),
                )
            };
            let mut state = FrameState {
                model,
                tree_table,
                out_table,
                panel_scroll,
                tree_visible_rows,
                out_visible_rows,
                filter_field,
                filter_focused,
                path_field,
                projects,
                project_table,
                modal_open,
                scroll_drag,
                zoom,
                status,
                tick,
                deferred: &mut deferred,
            };
            // Fresh tabs load themselves; a failed tab waits for R.
            for (index, tab) in state.model.tabs.iter().enumerate() {
                if tab.snapshot.is_none() && tab.error.is_none() && tab.load_generation == 0 {
                    state.deferred.request_loads.push(index);
                }
            }
            let pressed: Vec<KeyInput> = ui.input.keys().to_vec();
            if *state.modal_open {
                // The dialog owns the frame: nothing behind it draws, so no
                // background widget can claim the dialog's presses.
                render_open_dialog(&mut ui, &mut state, &pressed);
            } else if *state.filter_focused {
                let mut exit = false;
                for key in &pressed {
                    match key {
                        KeyInput::Enter | KeyInput::Escape => exit = true,
                        _ => {}
                    }
                }
                render_header(&mut ui, &mut state, physical(areas.header));
                render_filter_bar(&mut ui, &mut state, physical(areas.filter_bar), &pressed);
                render_left(&mut ui, &mut state, physical(areas.left), &[], false);
                render_right(&mut ui, &mut state, physical(areas.right), elapsed);
                render_footer(&mut ui, &mut state, physical(areas.footer), elapsed);
                if exit {
                    *state.filter_focused = false;
                }
            } else {
                shortcuts(&mut state, &pressed);
                let nav_outstanding = state.model.panel == Panel::Outstanding;
                render_header(&mut ui, &mut state, physical(areas.header));
                render_filter_bar(&mut ui, &mut state, physical(areas.filter_bar), &[]);
                render_left(
                    &mut ui,
                    &mut state,
                    physical(areas.left),
                    &pressed,
                    !nav_outstanding,
                );
                render_right_outstanding(
                    &mut ui,
                    &mut state,
                    physical(areas.right),
                    &pressed,
                    nav_outstanding,
                    elapsed,
                );
                render_footer(&mut ui, &mut state, physical(areas.footer), elapsed);
            }
            ui.input.end_frame();
        }
        if let Some(index) = deferred.switch_tab {
            self.switch_tab(index);
        }
        if deferred.close_tab {
            self.close_active_workspace();
        }
        if deferred.reload_tab {
            self.request_active_load();
        }
        if deferred.open_dialog {
            self.begin_open_dialog();
        }
        if let Some(path) = deferred.open_path {
            self.modal_open = false;
            self.open_workspace(&path);
        }
        if deferred.submit_modal {
            self.submit_open_prompt();
        }
        if deferred.cancel_modal {
            self.modal_open = false;
        }
        for index in deferred.request_loads {
            self.request_load(index);
        }
        if deferred.sync_title {
            self.sync_title();
        }
    }
}

fn render_right(ui: &mut Ui, state: &mut FrameState, area: Rect, elapsed: f32) {
    let loading = state
        .model
        .active_tab
        .and_then(|index| state.model.tabs.get(index))
        .is_some_and(|tab| tab.snapshot.is_none() && tab.error.is_none());
    if loading {
        render_loading(ui, state, area, elapsed);
        return;
    }
    match state.model.panel {
        Panel::Dashboard => render_dashboard(ui, state, area),
        Panel::Outstanding => render_outstanding(ui, state, area, &[], false),
        Panel::Relations => render_relations(ui, state, area),
        Panel::Git => render_git(ui, state, area),
        Panel::Tree | Panel::Inspector => render_inspector(ui, state, area),
    }
}

fn render_right_outstanding(
    ui: &mut Ui,
    state: &mut FrameState,
    area: Rect,
    pressed: &[KeyInput],
    nav_active: bool,
    elapsed: f32,
) {
    let loading = state
        .model
        .active_tab
        .and_then(|index| state.model.tabs.get(index))
        .is_some_and(|tab| tab.snapshot.is_none() && tab.error.is_none());
    if loading {
        render_loading(ui, state, area, elapsed);
        return;
    }
    match state.model.panel {
        Panel::Dashboard => render_dashboard(ui, state, area),
        Panel::Outstanding => render_outstanding(ui, state, area, pressed, nav_active),
        Panel::Relations => render_relations(ui, state, area),
        Panel::Git => render_git(ui, state, area),
        Panel::Tree | Panel::Inspector => render_inspector(ui, state, area),
    }
}

impl PixelApp for ReviewApp {
    fn render(&mut self, buffer: &mut WindowBuffer, scale: Scale) {
        self.frame(buffer, scale);
    }

    fn attach(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn animation_interval(&self) -> Option<Duration> {
        // Tick while anything is in flight: a fresh tab with no snapshot, or
        // a reload behind an old one.
        if self.is_loading() || self.load_started.is_some() {
            Some(Duration::from_millis(120))
        } else {
            None
        }
    }

    fn tick(&mut self) {
        self.poll_loads();
    }

    fn poll_title(&mut self) -> Option<String> {
        self.pending_title.take()
    }

    fn on_key(&mut self, key: &KeyInput) {
        self.input.key(key.clone());
    }

    fn on_modifiers(&mut self, modifiers: Modifiers) {
        self.input.modifiers_changed(modifiers);
    }

    fn on_cursor(&mut self, x: f32, y: f32) {
        self.input.cursor_moved(x, y);
    }

    fn on_cursor_left(&mut self) {
        self.input.cursor_left();
    }

    fn on_mouse(&mut self, button: MouseButton, pressed: bool) {
        self.input.mouse(button, pressed);
    }

    fn on_scroll(&mut self, dx: f32, dy: f32) {
        self.input.scrolled(dx, dy);
    }

    fn on_ime(&mut self, ime: &ImeEvent) {
        if let ImeEvent::Commit(text) = ime {
            if self.modal_open {
                self.path_field.insert_str(text);
            } else if self.filter_focused {
                self.filter_field.insert_str(text);
                self.model.filter.query = self.filter_field.text().to_owned();
            }
        }
    }
}

fn main() {
    let workspaces = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let mut app = ReviewApp::new(workspaces);
    app.sync_title();
    run_app(
        app,
        WindowConfig::new("AKR Review", 1280.0, 800.0).min_size(640.0, 420.0),
    )
    .expect("run the AKR review workbench");
}

#[cfg(test)]
mod tests {
    use super::*;
    use akr_gui::model::demo_snapshot;

    fn demo_app() -> ReviewApp {
        let mut app = ReviewApp::new(vec![PathBuf::from("/demo")]);
        app.model.set_snapshot(0, demo_snapshot("/demo"));
        app
    }

    fn frame(app: &mut ReviewApp) -> WindowBuffer {
        let mut buffer = WindowBuffer::new(1280, 800);
        app.frame(&mut buffer, Scale::new(1.0));
        buffer
    }

    fn click(app: &mut ReviewApp, x: f32, y: f32) {
        app.input.cursor_moved(x, y);
        app.input.mouse(MouseButton::Left, true);
        frame(app);
        app.input.mouse(MouseButton::Left, false);
        frame(app);
    }

    #[test]
    fn layout_tiles_the_window_without_overlap() {
        for (width, height) in [(640, 420), (1280, 800), (2560, 1440)] {
            let areas = layout_areas(width, height);
            assert_eq!(areas.header.h, HEADER_H);
            assert_eq!(areas.filter_bar.y, HEADER_H);
            assert_eq!(areas.left.y, HEADER_H + FILTER_H);
            assert_eq!(areas.left.y + areas.left.h, height - FOOTER_H);
            assert_eq!(areas.right.y, areas.left.y);
            assert_eq!(areas.right.h, areas.left.h);
            assert_eq!(areas.footer.y + areas.footer.h, height);
            assert_eq!(areas.left.x + areas.left.w + SPLIT_GAP, areas.right.x);
            assert_eq!(areas.right.x + areas.right.w, width);
            assert!(areas.left.w > 0 && areas.right.w > 0);
        }
    }

    #[test]
    fn embedded_font_covers_everything_the_chrome_uses() {
        assert_eq!(font_set().len(), 1);
        for character in ['A', 'g', '0', '/', '—', '·', '|'] {
            assert!(
                latin_face().covers(character),
                "the embedded face must cover {character:?}"
            );
        }
    }

    #[test]
    fn shortcut_keys_reach_the_outstanding_panel_and_the_filter() {
        let mut app = demo_app();
        app.input.key(KeyInput::Character('u'));
        frame(&mut app);
        assert_eq!(app.model.panel, Panel::Outstanding);
        app.input.key(KeyInput::Character('/'));
        frame(&mut app);
        assert!(app.filter_focused);
        app.input.key(KeyInput::Escape);
        frame(&mut app);
        assert!(!app.filter_focused);
    }

    #[test]
    fn clicking_the_dashboard_outstanding_button_jumps_to_outstanding() {
        let mut app = demo_app();
        assert_eq!(app.model.panel, Panel::Dashboard);
        // The jump button in render_dashboard at 1280x800, scale 1:
        // right.x + 12, right.y + 12 + 34 + 8 + 24 + 4, 460 wide, 36 tall.
        let areas = layout_areas(1280, 800);
        let x = (areas.right.x + 12 + 230) as f32;
        let y = (areas.right.y + 12 + 34 + 8 + 24 + 4 + 18) as f32;
        click(&mut app, x, y);
        assert_eq!(app.model.panel, Panel::Outstanding);
    }

    #[test]
    fn clicking_an_outstanding_row_opens_it_in_the_inspector() {
        let mut app = demo_app();
        app.model.panel = Panel::Outstanding;
        app.model.select_quiet("akr.work.review-snapshot");
        // The first table row in render_outstanding at 1280x800, scale 1:
        // title (28) + gap (8) + table header (30), then row 0 (30 tall).
        let areas = layout_areas(1280, 800);
        let x = (areas.right.x + 12 + 60) as f32;
        let y = (areas.right.y + 12 + 28 + 8 + 30 + 15) as f32;
        click(&mut app, x, y);
        assert_eq!(app.model.panel, Panel::Inspector);
        assert_eq!(
            app.model.selected().map(|record| record.key.as_str()),
            Some("akr.milestone.human-record-review")
        );
    }

    #[test]
    fn outstanding_screen_paints_real_content_headless() {
        let mut app = demo_app();
        app.model.panel = Panel::Outstanding;
        let buffer = frame(&mut app);
        let background = theme().background;
        let ink = buffer
            .pixels
            .iter()
            .filter(|pixel| **pixel != background)
            .count();
        assert!(
            ink > 50_000,
            "the outstanding screen should paint real content, got {ink} non-background pixels"
        );
        if let Ok(dir) = std::env::var("AKR_GUI_PNG_DIR") {
            pixelkit_raster::png::write(format!("{dir}/akr-gui-outstanding.png"), &buffer)
                .expect("write the outstanding screen");
            app.model.panel = Panel::Dashboard;
            let buffer = frame(&mut app);
            pixelkit_raster::png::write(format!("{dir}/akr-gui-dashboard.png"), &buffer)
                .expect("write the dashboard screen");
            app.projects = [
                "/home/dk/rust-projects/AKR",
                "/home/dk/rust-projects/pixelkit",
                "/home/dk/rust-projects/Kitchen-Concept",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect();
            app.modal_open = true;
            let buffer = frame(&mut app);
            pixelkit_raster::png::write(format!("{dir}/akr-gui-dialog.png"), &buffer)
                .expect("write the dialog screen");
        }
    }

    #[test]
    fn a_path_resolves_to_the_directory_holding_the_ledger() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        assert!(repo.join(".akr").is_dir(), "fixture assumption");
        // A file inside the project resolves to the project itself.
        let from_file = workspace_root(&repo.join("crates/akr-gui/src/main.rs")).unwrap();
        assert_eq!(from_file, workspace_root(repo).unwrap());
        // A nested directory does too.
        let from_dir = workspace_root(&repo.join("crates/akr-gui")).unwrap();
        assert_eq!(from_dir, from_file);
        assert!(!from_file.to_string_lossy().starts_with(r"\\?\"));
        assert!(workspace_root(Path::new("no/such/place/anywhere")).is_err());
    }

    #[test]
    fn table_thumb_matches_the_indicator_geometry() {
        let area = Rect::new(0, 0, 300, 330);
        let geom = table_thumb(area, 30, 20, 0).expect("a scrollbar");
        assert_eq!(geom.thumb, Rect::new(293, 30, 4, 150));
        assert_eq!((geom.max, geom.travel), (300, 150));
        let moved = table_thumb(area, 30, 20, 150).expect("a scrollbar");
        assert_eq!(moved.thumb.y, 105);
        assert!(
            table_thumb(area, 30, 5, 0).is_none(),
            "a list that fits has no thumb"
        );
    }

    #[test]
    fn dragging_the_outstanding_thumb_scrolls_without_selecting() {
        let mut app = demo_app();
        let mut snapshot = demo_snapshot("/demo");
        for index in 0..60 {
            snapshot.records.push(Record {
                key: format!("akr.work.load-{index:03}"),
                title: format!("Load record {index}"),
                kind: "work".into(),
                state: "proposed".into(),
                ..Record::default()
            });
        }
        app.model.set_snapshot(0, snapshot);
        app.model.panel = Panel::Outstanding;
        let areas = layout_areas(1280, 800);
        let table = Rect::new(
            areas.right.x + 12,
            areas.right.y + 12 + 28 + 8,
            areas.right.w - 24,
            areas.right.bottom() - 12 - (areas.right.y + 12 + 28 + 8),
        );
        let keys = app.model.outstanding_keys();
        assert_eq!(keys.len(), 63);
        let geom = table_thumb(table, 30, keys.len(), 0).expect("a scrollbar for 63 rows");
        let thumb = geom.thumb;
        let cx = (thumb.x + thumb.w / 2) as f32;
        let cy = (thumb.y + thumb.h / 2) as f32;
        app.input.cursor_moved(cx, cy);
        app.input.mouse(MouseButton::Left, true);
        frame(&mut app);
        assert!(
            app.scroll_drag.is_some(),
            "pressing the thumb starts a drag"
        );
        assert_eq!(
            app.model.panel,
            Panel::Outstanding,
            "the drag must not activate the row beneath"
        );
        app.input.cursor_moved(cx, cy + 120.0);
        frame(&mut app);
        let offset = app.out_table.scroll.offset();
        assert!(
            offset > 100,
            "the thumb drag should scroll, got offset {offset}"
        );
        app.input.mouse(MouseButton::Left, false);
        frame(&mut app);
        assert!(app.scroll_drag.is_none(), "releasing ends the drag");
    }

    #[test]
    fn discover_projects_lists_sibling_ledgers() {
        let root = std::env::temp_dir().join(format!("akr-gui-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let alpha = root.join("alpha");
        let beta = root.join("beta");
        std::fs::create_dir_all(alpha.join(".akr")).unwrap();
        std::fs::create_dir_all(beta.join(".akr")).unwrap();
        std::fs::create_dir_all(root.join("gamma-no-ledger")).unwrap();
        let found = discover_projects(&alpha);
        assert_eq!(found, vec![alpha.clone(), beta.clone()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clicking_a_project_row_switches_to_it() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .canonicalize()
            .expect("canonical workspace root");
        let mut app = demo_app();
        let other = app.model.add_tab(repo.clone());
        // A seeded error keeps the frame from spawning a real load for it.
        app.model.fail_load(other, "seeded");
        app.model.active_tab = Some(0);
        app.projects = vec![repo.clone()];
        app.modal_open = true;
        // The dialog's first project row at 1280x800, scale 1.
        let dialog_x = (1280 - DIALOG_W) / 2;
        let dialog_y = (800 - DIALOG_H) / 2;
        let table_y = dialog_y + 16 + 30 + 8 + 22 + 4;
        click(
            &mut app,
            (dialog_x + 16 + 60) as f32,
            (table_y + 30 + 15) as f32,
        );
        assert!(!app.modal_open, "opening a project closes the dialog");
        assert_eq!(app.model.active_tab, Some(other));
        assert!(
            app.status
                .as_deref()
                .unwrap_or("")
                .starts_with("already open"),
            "unexpected status: {:?}",
            app.status
        );
    }

    #[test]
    fn the_dialogs_open_button_opens_the_typed_path() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .canonicalize()
            .expect("canonical workspace root");
        let mut app = demo_app();
        let other = app.model.add_tab(repo.clone());
        app.model.fail_load(other, "seeded");
        app.model.active_tab = Some(0);
        app.projects = Vec::new();
        app.path_field = TextFieldState::with_text(repo.to_string_lossy());
        app.modal_open = true;
        let dialog_x = (1280 - DIALOG_W) / 2;
        let dialog_y = (800 - DIALOG_H) / 2;
        let open_x = dialog_x + DIALOG_W - 16 - 110 - 8 - 110;
        let open_y = dialog_y + DIALOG_H - 16 - 36;
        click(&mut app, (open_x + 55) as f32, (open_y + 18) as f32);
        assert!(!app.modal_open);
        assert_eq!(app.model.active_tab, Some(other));
    }

    #[test]
    fn clicks_behind_the_open_dialog_do_nothing() {
        let mut app = demo_app();
        app.modal_open = true;
        app.projects = Vec::new();
        // Where the dashboard's jump button would be, behind the dialog.
        let areas = layout_areas(1280, 800);
        let x = (areas.right.x + 12 + 230) as f32;
        let y = (areas.right.y + 12 + 34 + 8 + 24 + 4 + 18) as f32;
        click(&mut app, x, y);
        assert_eq!(app.model.panel, Panel::Dashboard);
        assert!(app.modal_open, "the dialog stays open");
    }

    #[test]
    fn planning_rows_follow_the_relation_hierarchy() {
        let mut model = AppModel::default();
        let tab = model.add_tab("/demo".into());
        model.set_snapshot(tab, demo_snapshot("/demo"));
        assert_eq!(
            tree_rows(&model)
                .iter()
                .map(|(_, key)| key.as_str())
                .collect::<Vec<_>>(),
            [
                "akr.milestone.human-record-review",
                "akr.work.desktop-review-gui",
                "akr.work.review-snapshot"
            ]
        );
    }
}
