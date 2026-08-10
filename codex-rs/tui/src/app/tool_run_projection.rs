//! Semantic lifecycle markers for quiet tool-run projection.
//!
//! Completed tool calls remain ordinary chronological source cells while a run is open. A
//! zero-height [`ToolRunSealCell`] records the first semantic boundary after the run. Retained and
//! inline renderers may only replace source rows with a Work summary when that marker is present.

use ratatui::text::Line;

use super::compact_tool_groups;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::SelectionContribution;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct SourceCellId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct RunId(u64);

impl RunId {
    pub(super) fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
pub(crate) struct ToolRunSealCell {
    run_id: RunId,
    first_source_id: SourceCellId,
    last_source_id: SourceCellId,
    action_count: usize,
}

impl ToolRunSealCell {
    pub(super) fn run_id(&self) -> RunId {
        self.run_id
    }

    pub(super) fn action_count(&self) -> usize {
        self.action_count
    }

    pub(super) fn source_count(&self) -> usize {
        let count = self
            .last_source_id
            .0
            .saturating_sub(self.first_source_id.0)
            .saturating_add(1);
        usize::try_from(count).unwrap_or(usize::MAX)
    }

    #[cfg(test)]
    pub(super) fn source_range(&self) -> (SourceCellId, SourceCellId) {
        (self.first_source_id, self.last_source_id)
    }

    #[cfg(test)]
    pub(crate) fn with_test_counts(
        run_id: u64,
        first_source_id: u64,
        last_source_id: u64,
        action_count: usize,
    ) -> Self {
        Self {
            run_id: RunId(run_id),
            first_source_id: SourceCellId(first_source_id),
            last_source_id: SourceCellId(last_source_id),
            action_count,
        }
    }
}

impl HistoryCell for ToolRunSealCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        Vec::new()
    }

    fn selection_contribution(
        &self,
        _width: u16,
        _mode: HistoryRenderMode,
    ) -> SelectionContribution {
        SelectionContribution::Transparent
    }
}

#[derive(Debug)]
struct OpenToolRun {
    run_id: RunId,
    first_source_id: SourceCellId,
    last_source_id: SourceCellId,
    action_count: usize,
    turn_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceClass {
    Action(usize),
    Transparent,
    Barrier,
}

#[derive(Debug)]
pub(super) struct ToolRunLifecycle {
    next_source_id: u64,
    next_run_id: u64,
    current_turn_id: Option<String>,
    open: Option<OpenToolRun>,
}

impl Default for ToolRunLifecycle {
    fn default() -> Self {
        Self {
            next_source_id: 0,
            // Zero is the explicit "no hover" sentinel in CompactToolGroupHover.
            next_run_id: 1,
            current_turn_id: None,
            open: None,
        }
    }
}

impl ToolRunLifecycle {
    /// Records one source append and returns the seal that must be inserted before it, if any.
    pub(super) fn before_source_append(
        &mut self,
        cell: &dyn HistoryCell,
    ) -> Option<ToolRunSealCell> {
        let class = if let Some(item) = compact_tool_groups::tool_group_item(cell) {
            SourceClass::Action(item.summary.actions)
        } else if compact_tool_groups::is_transparent_tool_group_cell(cell) {
            SourceClass::Transparent
        } else {
            SourceClass::Barrier
        };
        self.before_classified_source_append(class)
    }

    fn before_classified_source_append(&mut self, class: SourceClass) -> Option<ToolRunSealCell> {
        let source_id = SourceCellId(self.next_source_id);
        assert_ne!(
            self.next_source_id,
            u64::MAX,
            "tool-run source id exhausted"
        );
        self.next_source_id += 1;

        if let SourceClass::Action(actions) = class {
            let run = self.open.get_or_insert_with(|| {
                let run_id = RunId(self.next_run_id);
                assert_ne!(self.next_run_id, u64::MAX, "tool-run id exhausted");
                self.next_run_id += 1;
                OpenToolRun {
                    run_id,
                    first_source_id: source_id,
                    last_source_id: source_id,
                    action_count: 0,
                    turn_id: self.current_turn_id.clone(),
                }
            });
            run.last_source_id = source_id;
            run.action_count = run.action_count.saturating_add(actions);
            return None;
        }

        if class == SourceClass::Transparent {
            if let Some(run) = self.open.as_mut() {
                run.last_source_id = source_id;
            }
            return None;
        }

        self.seal_open_run()
    }

    pub(super) fn seal_open_run(&mut self) -> Option<ToolRunSealCell> {
        let run = self.open.take()?;
        Some(ToolRunSealCell {
            run_id: run.run_id,
            first_source_id: run.first_source_id,
            last_source_id: run.last_source_id,
            action_count: run.action_count,
        })
    }

    pub(super) fn is_active(&self) -> bool {
        self.open.is_some()
    }

    /// Starts a protocol turn, sealing any stale run from an earlier turn first.
    pub(super) fn begin_turn(&mut self, turn_id: String) -> Option<ToolRunSealCell> {
        if self.current_turn_id.as_deref() == Some(turn_id.as_str()) {
            return None;
        }
        let seal = self.seal_open_run();
        self.current_turn_id = Some(turn_id);
        seal
    }

    /// Seals only the run owned by `turn_id`; a late completion cannot close a newer run.
    pub(super) fn seal_turn(&mut self, turn_id: &str) -> Option<ToolRunSealCell> {
        let run = self.open.as_ref()?;
        if run.turn_id.as_deref() != Some(turn_id) {
            return None;
        }
        self.seal_open_run()
    }

    pub(super) fn reset_open_run(&mut self) {
        self.open = None;
        self.current_turn_id = None;
    }
}

pub(crate) fn seal_cell(cell: &dyn HistoryCell) -> Option<&ToolRunSealCell> {
    cell.as_any().downcast_ref::<ToolRunSealCell>()
}

#[cfg(test)]
#[path = "tool_run_projection_tests.rs"]
mod tests;
