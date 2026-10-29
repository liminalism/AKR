use std::path::PathBuf;
use std::sync::Arc;

use crate::model::{ReviewSnapshot, WorkspaceTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeMode {
    Planning,
    Knowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Dashboard,
    Outstanding,
    Tree,
    Inspector,
    Relations,
    Git,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub query: String,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub freshness: Option<String>,
}

impl Filter {
    pub fn matches(&self, record: &crate::model::Record) -> bool {
        self.matches_query(record)
            && self.kind.as_ref().is_none_or(|kind| kind == &record.kind)
            && self
                .state
                .as_ref()
                .is_none_or(|state| state == &record.state)
            && self
                .freshness
                .as_ref()
                .is_none_or(|freshness| freshness == &record.freshness)
    }

    /// The Outstanding view: the record must be `proposed` no matter what
    /// state preset is set; the text query and the kind/freshness facets
    /// still apply.
    pub fn matches_outstanding(&self, record: &crate::model::Record) -> bool {
        record.state == "proposed"
            && self.matches_query(record)
            && self.kind.as_ref().is_none_or(|kind| kind == &record.kind)
            && self
                .freshness
                .as_ref()
                .is_none_or(|freshness| freshness == &record.freshness)
    }

    fn matches_query(&self, record: &crate::model::Record) -> bool {
        let query = self.query.trim().to_ascii_lowercase();
        query.is_empty()
            || [
                record.key.as_str(),
                record.title.as_str(),
                record.body.as_str(),
            ]
            .iter()
            .any(|value| value.to_ascii_lowercase().contains(&query))
    }
}

/// Pure state for one native window. Rendering and event-loop wiring live in
/// `main`, keeping navigation and filtering deterministic and testable.
#[derive(Debug, Clone)]
pub struct AppModel {
    pub tabs: Vec<WorkspaceTab>,
    pub active_tab: Option<usize>,
    pub mode: TreeMode,
    pub panel: Panel,
    pub filter: Filter,
    pub expanded: std::collections::BTreeSet<String>,
}

impl Default for AppModel {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: None,
            mode: TreeMode::Planning,
            panel: Panel::Dashboard,
            filter: Filter::default(),
            expanded: Default::default(),
        }
    }
}

impl AppModel {
    pub fn add_tab(&mut self, workspace: PathBuf) -> usize {
        self.tabs.push(WorkspaceTab::loading(workspace));
        let index = self.tabs.len() - 1;
        self.active_tab = Some(index);
        index
    }
    pub fn set_snapshot(&mut self, index: usize, snapshot: ReviewSnapshot) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.snapshot = Some(Arc::new(snapshot));
        tab.error = None;
        if tab.selected_key.is_none() {
            tab.selected_key = tab.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .sorted_records()
                    .first()
                    .map(|record| record.key.clone())
            });
        }
    }
    pub fn fail_load(&mut self, index: usize, message: impl Into<String>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.error = Some(message.into());
        }
    }
    pub fn snapshot(&self) -> Option<&ReviewSnapshot> {
        self.active_tab
            .and_then(|index| self.tabs.get(index))
            .and_then(|tab| tab.snapshot.as_deref())
    }
    pub fn selected(&self) -> Option<&crate::model::Record> {
        let tab = self.active_tab.and_then(|index| self.tabs.get(index))?;
        tab.selected_key
            .as_deref()
            .and_then(|key| tab.snapshot.as_ref()?.record(key))
    }
    pub fn select(&mut self, key: impl Into<String>) {
        if let Some(tab) = self.active_tab.and_then(|index| self.tabs.get_mut(index)) {
            tab.selected_key = Some(key.into());
            self.panel = Panel::Inspector;
        }
    }
    /// Changes the selection without leaving the current panel, for arrow-key
    /// movement through a list.
    pub fn select_quiet(&mut self, key: impl Into<String>) {
        if let Some(tab) = self.active_tab.and_then(|index| self.tabs.get_mut(index)) {
            tab.selected_key = Some(key.into());
        }
    }
    pub fn filtered_keys(&self) -> Vec<String> {
        self.snapshot()
            .map(|snapshot| {
                snapshot
                    .sorted_records()
                    .into_iter()
                    .filter(|record| self.filter.matches(record))
                    .map(|record| record.key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Keys for the Outstanding panel: proposed records matching the text
    /// query, sorted by key.
    pub fn outstanding_keys(&self) -> Vec<String> {
        self.snapshot()
            .map(|snapshot| {
                snapshot
                    .sorted_records()
                    .into_iter()
                    .filter(|record| self.filter.matches_outstanding(record))
                    .map(|record| record.key.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn outstanding_count(&self) -> usize {
        self.snapshot()
            .map(|snapshot| snapshot.outstanding_count())
            .unwrap_or(0)
    }
    pub fn cycle_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.active_tab = Some(
            self.active_tab
                .map_or(0, |index| (index + 1) % self.tabs.len()),
        );
    }
    pub fn close_active_tab(&mut self) {
        let Some(index) = self.active_tab else {
            return;
        };
        self.tabs.remove(index);
        self.active_tab = (!self.tabs.is_empty()).then_some(index.min(self.tabs.len() - 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo_snapshot;
    #[test]
    fn filters_and_selection_are_deterministic() {
        let mut app = AppModel::default();
        let tab = app.add_tab("/demo".into());
        app.set_snapshot(tab, demo_snapshot("/demo"));
        app.filter.query = "snapshot".into();
        assert_eq!(app.filtered_keys(), ["akr.work.review-snapshot"]);
        app.select("akr.work.review-snapshot");
        assert_eq!(app.selected().unwrap().title, "Review snapshot API");
    }
    #[test]
    fn outstanding_keys_ignore_state_presets_but_keep_the_query() {
        let mut app = AppModel::default();
        let tab = app.add_tab("/demo".into());
        app.set_snapshot(tab, demo_snapshot("/demo"));
        assert_eq!(app.outstanding_count(), 3);
        // A state preset for something else must not empty the Outstanding view.
        app.filter.state = Some("active".into());
        assert_eq!(app.outstanding_keys().len(), 3);
        assert!(app.filtered_keys().is_empty());
        // The text query still narrows it.
        app.filter.query = "snapshot".into();
        assert_eq!(app.outstanding_keys(), ["akr.work.review-snapshot"]);
    }
    #[test]
    fn tabs_keep_independent_snapshots() {
        let mut app = AppModel::default();
        let first = app.add_tab("/one".into());
        app.set_snapshot(first, demo_snapshot("/one"));
        let second = app.add_tab("/two".into());
        app.set_snapshot(second, demo_snapshot("/two"));
        assert_eq!(app.snapshot().unwrap().workspace, PathBuf::from("/two"));
        app.cycle_tab();
        assert_eq!(app.snapshot().unwrap().workspace, PathBuf::from("/one"));
    }
}
