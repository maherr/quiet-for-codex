//! Application-owned alternate-screen rendering.
//!
//! The owned mode keeps committed conversation cells in a retained viewport and reserves the
//! bottom of every frame for the composer. Inline mode continues to use terminal scrollback.

use std::collections::HashSet;

use crossterm::cursor::SetCursorStyle;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::owned_screen_resize::OwnedScreenLayout;
use super::*;
use crate::AltScreenBehavior;
use crate::history_cell::HistoryRenderMode;
use crate::key_hint::is_plain_text_key_event;
use crate::tui::MouseMoveEvent;
use crate::tui::MousePrimaryEvent;
use crate::tui::MousePrimaryEventKind;
use crate::tui::MouseScrollEvent;

const PANE_HEADER_HEIGHT: u16 = 1;

pub(super) struct OwnedScreen {
    viewport: ConversationViewport,
    source_cells: Vec<Arc<dyn HistoryCell>>,
    compact_tool_groups: bool,
    expanded_tool_groups: HashSet<compact_tool_groups::CompactToolGroupId>,
    tool_group_hover: Arc<compact_tool_groups::CompactToolGroupHover>,
    pending_tool_group_click: Option<compact_tool_groups::CompactToolGroupId>,
    pending_tool_group_seals: HashSet<compact_tool_groups::CompactToolGroupId>,
    last_clicked_tool_group: Option<compact_tool_groups::CompactToolGroupId>,
    replay_in_progress: bool,
    last_pane_area: Rect,
    last_conversation_area: Rect,
    last_bottom_area: Rect,
    last_rendered_conversation_area: Rect,
    last_selection_viewport_area: Option<Rect>,
    last_hover_position: Option<Position>,
}

struct RenderedOwnedScreen {
    cursor: Option<(u16, u16)>,
    cursor_style: SetCursorStyle,
    selection_autoscroll_active: bool,
}

impl OwnedScreen {
    fn new(chat_widget: &ChatWidget, keymap: crate::keymap::PagerKeymap) -> Self {
        Self {
            viewport: ConversationViewport::new(
                Vec::new(),
                chat_widget.history_render_mode(),
                keymap,
            ),
            source_cells: Vec::new(),
            compact_tool_groups: chat_widget.history_render_mode() == HistoryRenderMode::Rich,
            expanded_tool_groups: HashSet::new(),
            tool_group_hover: Arc::new(compact_tool_groups::CompactToolGroupHover::default()),
            pending_tool_group_click: None,
            pending_tool_group_seals: HashSet::new(),
            last_clicked_tool_group: None,
            replay_in_progress: false,
            last_pane_area: Rect::default(),
            last_conversation_area: Rect::default(),
            last_bottom_area: Rect::default(),
            last_rendered_conversation_area: Rect::default(),
            last_selection_viewport_area: None,
            last_hover_position: None,
        }
    }

    fn replace_source_cells(
        &mut self,
        cells: Vec<Arc<dyn HistoryCell>>,
        compact_tool_groups: bool,
        reason: crate::quiet_metrics::QuietMetric,
    ) -> HashSet<compact_tool_groups::CompactToolGroupId> {
        crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::FullProjection);
        crate::quiet_metrics::bump(reason);
        // Reprojection can move a retained header while the pointer remains stationary. Clear the
        // cue until the terminal reports another coordinate instead of highlighting stale geometry.
        self.clear_tool_group_hover();
        let valid_groups = compact_tool_groups::compact_tool_group_ids(&cells);
        self.expanded_tool_groups
            .retain(|group| valid_groups.contains(group));
        self.pending_tool_group_seals
            .retain(|group| valid_groups.contains(group));
        if self
            .last_clicked_tool_group
            .is_some_and(|group| !valid_groups.contains(&group))
        {
            self.last_clicked_tool_group = None;
        }
        if !compact_tool_groups {
            self.pending_tool_group_click = None;
        }
        let projected = compact_tool_groups::project_owned_cells_with_expanded(
            &cells,
            compact_tool_groups,
            &self.expanded_tool_groups,
            &self.tool_group_hover,
        );
        self.source_cells = cells;
        self.compact_tool_groups = compact_tool_groups;
        self.viewport.replace_cells(projected);
        self.pending_tool_group_seals.clear();
        valid_groups
    }

    fn source_cells_match(&self, cells: &[Arc<dyn HistoryCell>]) -> bool {
        self.source_cells.len() == cells.len()
            && self
                .source_cells
                .iter()
                .zip(cells)
                .all(|(left, right)| Arc::ptr_eq(left, right))
    }

    fn projected_group_position(
        &self,
        group: compact_tool_groups::CompactToolGroupId,
    ) -> Option<usize> {
        (0..self.viewport.committed_cell_count()).find(|&index| {
            self.viewport.committed_cell(index).is_some_and(|cell| {
                compact_tool_groups::compact_tool_group_header_id(
                    cell.as_ref(),
                    self.last_conversation_area.width.max(1),
                    /*row_within_cell*/ 0,
                ) == Some(group)
            })
        })
    }

    fn apply_tool_group_projection(
        &mut self,
        group: compact_tool_groups::CompactToolGroupId,
        expanded: bool,
        respect_anchor_guard: bool,
        reason: crate::quiet_metrics::RangeReplacementReason,
    ) -> bool {
        let Some(projection) = compact_tool_groups::compact_tool_group_projection_by_id(
            &self.source_cells,
            group,
            expanded,
            &self.tool_group_hover,
        ) else {
            self.pending_tool_group_seals.remove(&group);
            return false;
        };
        let width = self.last_conversation_area.width.max(1);
        let projected_position = self.projected_group_position(group);
        let source_position = self
            .source_cells
            .get(projection.source_start)
            .and_then(|first_source| self.viewport.committed_cell_position(first_source));
        let (projected_start, remove_count) = if let Some(projected_start) = projected_position {
            let currently_expanded = self.expanded_tool_groups.contains(&group);
            (
                projected_start,
                1usize.saturating_add(if currently_expanded {
                    projection.source_cells
                } else {
                    0
                }),
            )
        } else if let Some(source_position) = source_position {
            if respect_anchor_guard
                && (self.viewport.selection_is_active()
                    || !self.viewport.is_following_bottom()
                        && self.viewport.committed_range_intersects_viewport(
                            width,
                            source_position,
                            projection.source_cells,
                            self.last_conversation_area,
                        ))
            {
                self.pending_tool_group_seals.insert(group);
                return false;
            }
            (source_position, projection.source_cells)
        } else {
            if respect_anchor_guard && self.viewport.selection_is_active() {
                self.pending_tool_group_seals.insert(group);
            }
            return false;
        };
        crate::quiet_metrics::bump_range_replacement(reason);
        self.viewport
            .replace_range(width, projected_start, remove_count, projection.cells);
        self.pending_tool_group_seals.remove(&group);
        true
    }

    fn resolve_pending_tool_group_seals(&mut self) {
        let pending = self
            .pending_tool_group_seals
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for group in pending {
            self.apply_tool_group_projection(
                group,
                /*expanded*/ false,
                /*respect_anchor_guard*/ true,
                crate::quiet_metrics::RangeReplacementReason::Seal,
            );
        }
    }

    fn set_compact_tool_groups(&mut self, compact_tool_groups: bool) {
        if self.compact_tool_groups == compact_tool_groups {
            return;
        }
        let width = self.last_conversation_area.width.max(1);
        let ids = compact_tool_groups::compact_tool_group_ids_in_order(&self.source_cells);
        if compact_tool_groups {
            self.compact_tool_groups = true;
            for group in ids {
                let expanded = self.expanded_tool_groups.contains(&group);
                self.apply_tool_group_projection(
                    group,
                    expanded,
                    /*respect_anchor_guard*/ false,
                    crate::quiet_metrics::RangeReplacementReason::UserToggle,
                );
            }
        } else {
            for group in ids.into_iter().rev() {
                let Some(projected_start) = self.projected_group_position(group) else {
                    continue;
                };
                let Some(source_cells) =
                    compact_tool_groups::compact_tool_group_cells_by_id(&self.source_cells, group)
                else {
                    continue;
                };
                let remove_count =
                    1usize.saturating_add(if self.expanded_tool_groups.contains(&group) {
                        source_cells.len()
                    } else {
                        0
                    });
                crate::quiet_metrics::bump_range_replacement(
                    crate::quiet_metrics::RangeReplacementReason::UserToggle,
                );
                self.viewport
                    .replace_range(width, projected_start, remove_count, source_cells);
            }
            self.compact_tool_groups = false;
            self.pending_tool_group_seals.clear();
            self.pending_tool_group_click = None;
            self.clear_tool_group_hover();
        }
    }

    fn push_source_cell(&mut self, cell: Arc<dyn HistoryCell>, compact_tool_groups: bool) {
        self.clear_tool_group_hover();
        if self.compact_tool_groups != compact_tool_groups {
            self.set_compact_tool_groups(compact_tool_groups);
        }

        if !compact_tool_groups {
            self.source_cells.push(cell.clone());
            if tool_run_projection::seal_cell(cell.as_ref()).is_none() {
                self.viewport.push_cell(cell);
            }
            return;
        }

        self.source_cells.push(Arc::clone(&cell));
        if tool_run_projection::seal_cell(cell.as_ref()).is_none() {
            self.viewport.push_cell(cell);
            return;
        }

        let seal_index = self.source_cells.len().saturating_sub(1);
        let Some(group) = compact_tool_groups::compact_tool_group_before_seal(
            &self.source_cells,
            seal_index,
            /*expanded*/ false,
            &self.tool_group_hover,
        ) else {
            return;
        };
        self.apply_tool_group_projection(
            group.id,
            /*expanded*/ false,
            /*respect_anchor_guard*/ true,
            crate::quiet_metrics::RangeReplacementReason::Seal,
        );
        self.expanded_tool_groups.remove(&group.id);
    }

    fn replace_source_cell(
        &mut self,
        source_index: usize,
        replacement: Arc<dyn HistoryCell>,
        compact_tool_groups: bool,
    ) -> bool {
        if self.compact_tool_groups != compact_tool_groups {
            return false;
        }
        let Some(old_source) = self.source_cells.get(source_index).cloned() else {
            return false;
        };
        let old_group = compact_tool_groups::compact_tool_group_containing_source_index(
            &self.source_cells,
            source_index,
        );
        let width = self.last_conversation_area.width.max(1);
        let (projected_start, remove_count) = if let Some(group) = old_group {
            if let Some(projected_start) = self.projected_group_position(group.id) {
                (
                    projected_start,
                    1usize.saturating_add(if self.expanded_tool_groups.contains(&group.id) {
                        group.source_cells
                    } else {
                        0
                    }),
                )
            } else {
                let Some(source_start) = self
                    .source_cells
                    .get(group.source_start)
                    .and_then(|cell| self.viewport.committed_cell_position(cell))
                else {
                    return false;
                };
                (source_start, group.source_cells)
            }
        } else {
            let Some(projected_start) = self.viewport.committed_cell_position(&old_source) else {
                return false;
            };
            (projected_start, 1)
        };

        self.source_cells[source_index] = replacement.clone();
        let projected = if let Some(group) = old_group {
            let source_end = group
                .source_start
                .saturating_add(group.consumed_cells)
                .min(self.source_cells.len());
            let affected = &self.source_cells[group.source_start..source_end];
            let valid_groups = compact_tool_groups::compact_tool_group_ids(affected);
            let group_survives = valid_groups.contains(&group.id);
            if !group_survives {
                self.expanded_tool_groups.remove(&group.id);
                self.pending_tool_group_seals.remove(&group.id);
                if self.pending_tool_group_click == Some(group.id) {
                    self.pending_tool_group_click = None;
                }
                if self.last_clicked_tool_group == Some(group.id) {
                    self.last_clicked_tool_group = None;
                }
                if self.tool_group_hover.hovered() == Some(group.id) {
                    self.last_hover_position = None;
                    self.tool_group_hover.update(None);
                }
            }
            let visually_guarded = group_survives
                && self.pending_tool_group_seals.contains(&group.id)
                && self.projected_group_position(group.id).is_none();
            compact_tool_groups::project_owned_cells_with_expanded(
                affected,
                compact_tool_groups && !visually_guarded,
                &self.expanded_tool_groups,
                &self.tool_group_hover,
            )
        } else if tool_run_projection::seal_cell(replacement.as_ref()).is_some() {
            Vec::new()
        } else {
            vec![replacement]
        };
        crate::quiet_metrics::bump_range_replacement(
            crate::quiet_metrics::RangeReplacementReason::Lifecycle,
        );
        self.viewport
            .replace_range(width, projected_start, remove_count, projected);
        true
    }

    /// Apply a structural source splice without rebuilding unrelated projection wrappers.
    ///
    /// Boundaries must fall between projected cells or complete Work segments. A splice that
    /// would cut through a sealed segment fails closed so the caller can use the conservative
    /// source-backed reconciliation path instead.
    fn splice_source_cells(
        &mut self,
        source_index: usize,
        remove_count: usize,
        replacements: Vec<Arc<dyn HistoryCell>>,
        compact_tool_groups: bool,
    ) -> bool {
        if self.compact_tool_groups != compact_tool_groups {
            return false;
        }
        let Some(source_end) = source_index.checked_add(remove_count) else {
            return false;
        };
        if source_end > self.source_cells.len() {
            return false;
        }
        let Some(projected_start) = self.projected_source_boundary(source_index) else {
            return false;
        };
        let Some(projected_end) = self.projected_source_boundary(source_end) else {
            return false;
        };
        if projected_end < projected_start {
            return false;
        }

        let projected = compact_tool_groups::project_owned_cells_with_expanded(
            &replacements,
            compact_tool_groups,
            &self.expanded_tool_groups,
            &self.tool_group_hover,
        );
        self.source_cells
            .splice(source_index..source_end, replacements);

        let valid_groups = compact_tool_groups::compact_tool_group_ids(&self.source_cells);
        self.expanded_tool_groups
            .retain(|group| valid_groups.contains(group));
        self.pending_tool_group_seals
            .retain(|group| valid_groups.contains(group));
        if self
            .pending_tool_group_click
            .is_some_and(|group| !valid_groups.contains(&group))
        {
            self.pending_tool_group_click = None;
        }
        if self
            .last_clicked_tool_group
            .is_some_and(|group| !valid_groups.contains(&group))
        {
            self.last_clicked_tool_group = None;
        }
        if self
            .tool_group_hover
            .hovered()
            .is_some_and(|group| !valid_groups.contains(&group))
        {
            self.last_hover_position = None;
            self.tool_group_hover.update(None);
        }

        let width = self.last_conversation_area.width.max(1);
        crate::quiet_metrics::bump_range_replacement(
            crate::quiet_metrics::RangeReplacementReason::Structural,
        );
        self.viewport.replace_range(
            width,
            projected_start,
            projected_end.saturating_sub(projected_start),
            projected,
        );
        true
    }

    fn projected_source_boundary(&self, source_index: usize) -> Option<usize> {
        if source_index > self.source_cells.len() {
            return None;
        }
        if source_index == self.source_cells.len() {
            return Some(self.viewport.committed_cell_count());
        }
        let source = self.source_cells.get(source_index)?;
        if tool_run_projection::seal_cell(source.as_ref()).is_some() {
            return None;
        }
        if let Some(group) = compact_tool_groups::compact_tool_group_containing_source_index(
            &self.source_cells,
            source_index,
        ) {
            if group.source_start != source_index {
                return None;
            }
            return self.projected_group_position(group.id).or_else(|| {
                self.source_cells
                    .get(group.source_start)
                    .and_then(|cell| self.viewport.committed_cell_position(cell))
            });
        }
        self.viewport.committed_cell_position(source)
    }

    fn render(
        &mut self,
        chat_widget: &ChatWidget,
        area: Rect,
        buffer: &mut Buffer,
    ) -> RenderedOwnedScreen {
        Clear.render(area, buffer);
        if !chat_widget.no_modal_or_popup_active() {
            self.clear_tool_group_hover();
        }

        let bottom_pane =
            chat_widget.bottom_pane_renderable_with_newer_hint(/*show_newer_hint*/ false);
        let bottom_height = bottom_pane.desired_height(area.width).min(area.height);
        let conversation_height = area.height.saturating_sub(bottom_height);
        let conversation_area = Rect::new(
            area.x,
            area.y,
            chat_widget.history_wrap_width(area.width),
            conversation_height,
        );
        let bottom_area = Rect::new(
            area.x,
            area.y.saturating_add(conversation_height),
            area.width,
            bottom_height,
        );
        self.last_bottom_area = bottom_area;
        if self.last_rendered_conversation_area != conversation_area {
            self.clear_tool_group_hover();
        }
        self.last_rendered_conversation_area = conversation_area;
        self.last_conversation_area = conversation_area;
        self.resolve_pending_tool_group_seals();

        self.viewport
            .set_render_mode(chat_widget.history_render_mode());
        chat_widget.observe_live_tail_frame(conversation_area.width);
        let active_key = chat_widget.active_cell_render_key();
        let scroll_offset_before_live_tail = self.viewport.scroll_offset();
        self.viewport
            .sync_live_tail(conversation_area.width, active_key, |width, lane| {
                chat_widget.active_cell_display_snapshots_for_lane(width, lane)
            });
        if self.viewport.scroll_offset() != scroll_offset_before_live_tail {
            self.clear_tool_group_hover();
        }
        let now = Instant::now();
        let selection_autoscroll_active =
            if self.last_selection_viewport_area.replace(conversation_area)
                == Some(conversation_area)
            {
                self.viewport
                    .advance_selection_autoscroll(conversation_area, now)
            } else {
                self.viewport
                    .prepare_selection_autoscroll(conversation_area, now)
            };
        self.viewport.render(conversation_area, buffer);
        let bottom_pane = chat_widget.bottom_pane_renderable_with_newer_hint(
            /*show_newer_hint*/ !self.viewport.is_following_bottom(),
        );
        bottom_pane.render(bottom_area, buffer);

        RenderedOwnedScreen {
            cursor: bottom_pane.cursor_pos(bottom_area),
            cursor_style: bottom_pane.cursor_style(bottom_area),
            selection_autoscroll_active,
        }
    }

    fn render_activity_bottom_pane(
        &self,
        chat_widget: &ChatWidget,
        buffer: &mut Buffer,
    ) -> Option<RenderedOwnedScreen> {
        let area = self.last_bottom_area;
        if area.is_empty() {
            return None;
        }
        Clear.render(area, buffer);
        let bottom_pane = chat_widget.bottom_pane_renderable_with_newer_hint(
            /*show_newer_hint*/ !self.viewport.is_following_bottom(),
        );
        bottom_pane.render(area, buffer);
        Some(RenderedOwnedScreen {
            cursor: bottom_pane.cursor_pos(area),
            cursor_style: bottom_pane.cursor_style(area),
            selection_autoscroll_active: false,
        })
    }

    fn handle_navigation_key(&mut self, key_event: KeyEvent) -> bool {
        // Composer history and cursor movement own arrows and Home. While the composer is empty,
        // End is an explicit transcript jump matching the visible newer-history footer cue.
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            || is_plain_text_key_event(key_event)
            || matches!(key_event.code, KeyCode::Up | KeyCode::Down | KeyCode::Home)
        {
            return false;
        }
        let handled = self
            .viewport
            .handle_navigation_key(self.last_conversation_area, key_event);
        if handled {
            self.clear_tool_group_hover();
        }
        handled
    }

    fn set_keymap(&mut self, keymap: crate::keymap::PagerKeymap) {
        self.viewport.set_keymap(keymap);
    }

    fn handle_mouse_scroll(&mut self, event: MouseScrollEvent) -> bool {
        if self.viewport.selection_is_active() {
            self.pending_tool_group_click = None;
            self.clear_tool_group_hover();
            self.viewport.handle_selection_mouse_scroll(
                self.last_conversation_area,
                event.direction,
                Position::new(event.column, event.row),
            );
            return true;
        }
        if !self
            .last_conversation_area
            .contains(Position::new(event.column, event.row))
        {
            return false;
        }
        self.viewport.handle_mouse_scroll(event.direction);
        self.last_hover_position = None;
        self.update_tool_group_hover(Position::new(event.column, event.row));
        true
    }

    fn begin_selection(&mut self, position: Position) -> bool {
        let conversation_area = self.last_conversation_area;
        let position = if !conversation_area.is_empty()
            && self.last_pane_area.contains(position)
            && position.y >= conversation_area.y
            && position.y < conversation_area.bottom()
            && position.x >= conversation_area.right()
        {
            Position::new(conversation_area.right().saturating_sub(1), position.y)
        } else {
            position
        };
        let pressed_group = self.compact_tool_group_header_at(position);
        self.tool_group_hover.update(pressed_group);
        self.pending_tool_group_click = pressed_group;
        let started = self.viewport.begin_selection(conversation_area, position);
        if !started {
            self.pending_tool_group_click = None;
        }
        started
    }

    fn update_selection(&mut self, position: Position) -> bool {
        self.pending_tool_group_click = None;
        self.clear_tool_group_hover();
        self.viewport
            .update_selection(self.last_conversation_area, position)
    }

    fn finish_selection(&mut self, position: Position) -> Option<String> {
        let released_group = self.compact_tool_group_header_at(position);
        let pressed_group = self.pending_tool_group_click.take();
        // Capture the resolved offset before finish_selection applies any deferred tool updates.
        // A deferred append may switch the pager back to its bottom sentinel, while a fold click
        // must keep the clicked header anchored at its current screen row.
        let scroll_offset = self.viewport.scroll_offset();
        let selected = self
            .viewport
            .finish_selection(self.last_conversation_area, position);
        if selected.is_none()
            && pressed_group.is_some()
            && pressed_group == released_group
            && let Some(group) = pressed_group
        {
            self.toggle_tool_group(group, scroll_offset);
        }
        selected
    }

    fn cancel_selection(&mut self) {
        self.pending_tool_group_click = None;
        self.viewport.cancel_selection();
    }

    fn selection_is_active(&self) -> bool {
        self.viewport.selection_is_active()
    }

    fn clear_last_render_areas(&mut self) {
        self.last_pane_area = Rect::default();
        self.last_conversation_area = Rect::default();
        self.last_bottom_area = Rect::default();
    }

    fn update_tool_group_hover(&mut self, position: Position) -> bool {
        let same_region = self.last_hover_position.is_some_and(|previous| {
            previous.y == position.y
                && self.last_conversation_area.contains(previous)
                    == self.last_conversation_area.contains(position)
        });
        if same_region {
            return false;
        }
        self.last_hover_position = Some(position);
        let hovered = self.compact_tool_group_header_at(position);
        self.tool_group_hover.update(hovered)
    }

    fn clear_tool_group_hover(&mut self) -> bool {
        self.last_hover_position = None;
        self.tool_group_hover.update(None)
    }

    fn compact_tool_group_header_at(
        &mut self,
        position: Position,
    ) -> Option<compact_tool_groups::CompactToolGroupId> {
        if !self.compact_tool_groups {
            return None;
        }
        let hit = self
            .viewport
            .committed_cell_hit(self.last_conversation_area, position)?;
        let cell = self.viewport.committed_cell(hit.index)?;
        compact_tool_groups::compact_tool_group_header_id(
            cell.as_ref(),
            self.last_conversation_area.width,
            hit.row_within_cell,
        )
    }

    fn toggle_tool_group(
        &mut self,
        group: compact_tool_groups::CompactToolGroupId,
        scroll_offset: usize,
    ) -> bool {
        if !self.compact_tool_groups {
            return false;
        }
        let expanding = !self.expanded_tool_groups.contains(&group);
        if !self.apply_tool_group_projection(
            group,
            expanding,
            /*respect_anchor_guard*/ false,
            crate::quiet_metrics::RangeReplacementReason::UserToggle,
        ) {
            return false;
        }
        if expanding {
            self.expanded_tool_groups.insert(group);
        } else {
            self.expanded_tool_groups.remove(&group);
        }
        self.last_clicked_tool_group = Some(group);
        self.viewport
            .preserve_scroll_offset_through_next_render(scroll_offset);
        true
    }

    fn inspection_cells(&mut self) -> Option<Vec<Arc<dyn HistoryCell>>> {
        let clicked = self.last_clicked_tool_group.filter(|group| {
            compact_tool_groups::compact_tool_group_cells_by_id(&self.source_cells, *group)
                .is_some()
        });
        let hovered = self.tool_group_hover.hovered().filter(|group| {
            compact_tool_groups::compact_tool_group_cells_by_id(&self.source_cells, *group)
                .is_some()
        });
        let visible = self
            .viewport
            .visible_committed_cell_range(self.last_conversation_area)
            .and_then(|(first, last)| {
                (first..=last).rev().find_map(|index| {
                    let cell = self.viewport.committed_cell(index)?;
                    compact_tool_groups::compact_tool_group_header_id(
                        cell.as_ref(),
                        self.last_conversation_area.width.max(1),
                        /*row_within_cell*/ 0,
                    )
                    .or_else(|| {
                        compact_tool_groups::compact_tool_group_id_containing_cell(
                            &self.source_cells,
                            cell,
                        )
                    })
                })
            });
        let latest = compact_tool_groups::latest_compact_tool_group_id(&self.source_cells);
        [clicked, hovered, visible, latest]
            .into_iter()
            .flatten()
            .next()
            .and_then(|group| {
                compact_tool_groups::compact_tool_group_cells_by_id(&self.source_cells, group)
            })
    }
}

fn pane_body_area(area: Rect, show_header: bool) -> Rect {
    if !show_header {
        return area;
    }
    let header_height = PANE_HEADER_HEIGHT.min(area.height);
    Rect::new(
        area.x,
        area.y.saturating_add(header_height),
        area.width,
        area.height.saturating_sub(header_height),
    )
}

fn render_pane_header(slot: PaneSlot, focused: bool, area: Rect, buffer: &mut Buffer) {
    if area.height == 0 {
        return;
    }
    let label = match slot {
        PaneSlot::Parent => "Parent",
        PaneSlot::Side => "Side",
    };
    let line: Line<'static> = if focused {
        let parent: Span<'static> = conversation_panes::parent_pane_shortcut().into();
        let side: Span<'static> = conversation_panes::side_pane_shortcut().into();
        vec![
            " ".into(),
            label.cyan().bold(),
            "  ".into(),
            format!("{} / {} focus", parent.content, side.content).dim(),
        ]
        .into()
    } else {
        vec![" ".into(), label.dim()].into()
    };
    Paragraph::new(line).render(
        Rect::new(
            area.x,
            area.y,
            area.width,
            PANE_HEADER_HEIGHT.min(area.height),
        ),
        buffer,
    );
}

fn render_divider(area: Rect, active: bool, buffer: &mut Buffer) {
    if area.width == 0 {
        return;
    }
    let style = if active {
        Style::default().cyan().bold()
    } else {
        Style::default().dim()
    };
    for y in area.y..area.bottom() {
        buffer[(area.x, y)]
            .set_symbol(if active { "┃" } else { "│" })
            .set_style(style);
    }
}

fn render_pane(
    panes: &mut ConversationPanes,
    slot: PaneSlot,
    area: Rect,
    show_header: bool,
    focused: PaneSlot,
    buffer: &mut Buffer,
) -> Option<RenderedOwnedScreen> {
    render_pane_header(slot, slot == focused, area, buffer);
    let body_area = pane_body_area(area, show_header);
    let pane = panes.by_slot_mut(slot)?;
    pane.chat_widget.update_owned_screen_width(body_area.width);
    let screen = pane.owned_screen.as_mut()?;
    screen.last_pane_area = area;
    Some(screen.render(&pane.chat_widget, body_area, buffer))
}

fn render_layout(
    panes: &mut ConversationPanes,
    layout: OwnedScreenLayout,
    focused: PaneSlot,
    buffer: &mut Buffer,
) -> Option<RenderedOwnedScreen> {
    panes.record_owned_screen_layout(layout);
    let divider_active = panes.owned_screen_split_is_dragging();
    Clear.render(layout.area(), buffer);
    for slot in [PaneSlot::Parent, PaneSlot::Side] {
        let slot_is_visible = match layout {
            OwnedScreenLayout::Single { slot: visible, .. } => slot == visible,
            OwnedScreenLayout::Split { .. } => true,
        };
        if let Some(screen) = panes
            .by_slot_mut(slot)
            .and_then(|pane| pane.owned_screen.as_mut())
        {
            if !slot_is_visible {
                screen.cancel_selection();
                screen.clear_tool_group_hover();
            }
            screen.clear_last_render_areas();
        }
    }

    match layout {
        OwnedScreenLayout::Single {
            slot,
            area,
            show_header,
        } => render_pane(panes, slot, area, show_header, focused, buffer),
        OwnedScreenLayout::Split {
            parent,
            divider,
            side,
            ..
        } => {
            let parent_rendered = render_pane(
                panes,
                PaneSlot::Parent,
                parent,
                /*show_header*/ true,
                focused,
                buffer,
            );
            let side_rendered = render_pane(
                panes,
                PaneSlot::Side,
                side,
                /*show_header*/ true,
                focused,
                buffer,
            );
            render_divider(divider, divider_active, buffer);
            let selection_autoscroll_active = parent_rendered
                .as_ref()
                .is_some_and(|rendered| rendered.selection_autoscroll_active)
                || side_rendered
                    .as_ref()
                    .is_some_and(|rendered| rendered.selection_autoscroll_active);
            let focused_rendered = match focused {
                PaneSlot::Parent => parent_rendered,
                PaneSlot::Side => side_rendered,
            };
            focused_rendered.map(|mut rendered| {
                rendered.selection_autoscroll_active = selection_autoscroll_active;
                rendered
            })
        }
    }
}

fn activity_bottom_band(panes: &ConversationPanes) -> Option<Rect> {
    let mut band: Option<Rect> = None;
    for slot in [PaneSlot::Parent, PaneSlot::Side] {
        let Some(area) = panes
            .by_slot(slot)
            .and_then(|pane| pane.owned_screen.as_ref())
            .map(|screen| screen.last_bottom_area)
            .filter(|area| !area.is_empty())
        else {
            continue;
        };
        band = Some(match band {
            Some(current) => {
                let x = current.x.min(area.x);
                let y = current.y.min(area.y);
                let right = current.right().max(area.right());
                let bottom = current.bottom().max(area.bottom());
                Rect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
            }
            None => area,
        });
    }
    band
}

fn render_activity_bottom_panes(
    panes: &mut ConversationPanes,
    focused: PaneSlot,
    buffer: &mut Buffer,
) -> Option<RenderedOwnedScreen> {
    let mut focused_rendered = None;
    for slot in [PaneSlot::Parent, PaneSlot::Side] {
        let rendered = panes.by_slot_mut(slot).and_then(|pane| {
            pane.owned_screen
                .as_ref()?
                .render_activity_bottom_pane(&pane.chat_widget, buffer)
        });
        if slot == focused {
            focused_rendered = rendered;
        }
    }
    focused_rendered
}

impl App {
    pub(super) fn owned_screen_for_behavior(
        alt_screen_behavior: AltScreenBehavior,
        chat_widget: &ChatWidget,
        keymap: crate::keymap::PagerKeymap,
    ) -> Option<OwnedScreen> {
        match alt_screen_behavior {
            AltScreenBehavior::Disabled => None,
            AltScreenBehavior::Owned => Some(OwnedScreen::new(chat_widget, keymap)),
        }
    }

    pub(super) fn has_owned_screen(&self) -> bool {
        self.chat_widget
            .by_slot(PaneSlot::Parent)
            .is_some_and(|pane| pane.owned_screen.is_some())
    }

    pub(super) fn owned_activity_animation_active(&self) -> bool {
        [PaneSlot::Parent, PaneSlot::Side].into_iter().any(|slot| {
            self.chat_widget.by_slot(slot).is_some_and(|pane| {
                pane.owned_screen
                    .as_ref()
                    .is_some_and(|screen| !screen.last_bottom_area.is_empty())
                    && pane.chat_widget.activity_animation_active()
            })
        })
    }

    pub(super) fn owned_screen_push_cell(&mut self, cell: Arc<dyn HistoryCell>) {
        let compact_tool_groups = self.compact_tool_groups_enabled();
        if let Some(screen) = &mut self.chat_widget.owned_screen {
            screen.push_source_cell(cell, compact_tool_groups);
        }
    }

    pub(super) fn begin_owned_screen_replay(&mut self) {
        if let Some(screen) = &mut self.chat_widget.owned_screen {
            screen.replay_in_progress = true;
        }
    }

    pub(super) fn finish_owned_screen_replay(&mut self) {
        if let Some(screen) = &mut self.chat_widget.owned_screen {
            screen.replay_in_progress = false;
        }
    }

    pub(super) fn owned_screen_replay_in_progress(&self) -> bool {
        self.chat_widget
            .owned_screen
            .as_ref()
            .is_some_and(|screen| screen.replay_in_progress)
    }

    pub(super) fn handle_owned_screen_navigation_key(
        &mut self,
        tui: &mut tui::Tui,
        key_event: KeyEvent,
    ) -> bool {
        if !self.chat_widget.composer_is_empty() || !self.chat_widget.no_modal_or_popup_active() {
            return false;
        }
        let handled = self
            .chat_widget
            .owned_screen
            .as_mut()
            .is_some_and(|screen| screen.handle_navigation_key(key_event));
        if handled {
            tui.frame_requester()
                .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
        }
        handled
    }

    pub(super) fn handle_owned_screen_mouse_scroll(
        &mut self,
        tui: &mut tui::Tui,
        event: MouseScrollEvent,
    ) -> bool {
        if self.chat_widget.owned_screen_split_is_dragging() {
            return true;
        }
        let active_selection_slot = [PaneSlot::Parent, PaneSlot::Side].into_iter().find(|slot| {
            self.chat_widget
                .by_slot(*slot)
                .and_then(|pane| pane.owned_screen.as_ref())
                .is_some_and(OwnedScreen::selection_is_active)
        });
        if let Some(slot) = active_selection_slot {
            let handled = self
                .chat_widget
                .by_slot_mut(slot)
                .and_then(|pane| pane.owned_screen.as_mut())
                .is_some_and(|screen| screen.handle_mouse_scroll(event));
            if handled {
                tui.frame_requester()
                    .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
            }
            return handled;
        }
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            let handled = self.chat_widget.by_slot_mut(slot).is_some_and(|pane| {
                pane.chat_widget.no_modal_or_popup_active()
                    && pane
                        .owned_screen
                        .as_mut()
                        .is_some_and(|screen| screen.handle_mouse_scroll(event))
            });
            if handled {
                tui.frame_requester()
                    .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
                return true;
            }
        }
        false
    }

    pub(super) fn handle_owned_screen_mouse_move(
        &mut self,
        tui: &mut tui::Tui,
        event: MouseMoveEvent,
    ) -> bool {
        if self.overlay.is_some() {
            let changed = self.clear_owned_screen_tool_group_hover();
            if changed {
                tui.frame_requester().schedule_frame();
            }
            return changed;
        }

        let position = Position::new(event.column, event.row);
        let target = [PaneSlot::Parent, PaneSlot::Side].into_iter().find(|slot| {
            self.chat_widget.by_slot(*slot).is_some_and(|pane| {
                pane.chat_widget.no_modal_or_popup_active()
                    && pane
                        .owned_screen
                        .as_ref()
                        .is_some_and(|screen| screen.last_pane_area.contains(position))
            })
        });
        let mut changed = false;
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            let Some(pane) = self.chat_widget.by_slot_mut(slot) else {
                continue;
            };
            let can_hover = target == Some(slot) && pane.chat_widget.no_modal_or_popup_active();
            let Some(screen) = pane.owned_screen.as_mut() else {
                continue;
            };
            changed |= if can_hover {
                screen.update_tool_group_hover(position)
            } else {
                screen.clear_tool_group_hover()
            };
        }
        if changed {
            tui.frame_requester()
                .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
        }
        changed
    }

    pub(super) fn handle_owned_screen_mouse_primary(
        &mut self,
        tui: &mut tui::Tui,
        event: MousePrimaryEvent,
    ) -> bool {
        if self.overlay.is_some() || !self.chat_widget.no_modal_or_popup_active() {
            let split_canceled = self.chat_widget.cancel_owned_screen_split_drag();
            let selection_canceled = self.cancel_owned_screen_selection();
            let hover_cleared = self.clear_owned_screen_tool_group_hover();
            if split_canceled || selection_canceled || hover_cleared {
                tui.frame_requester().schedule_frame();
            }
            return false;
        }

        if self.chat_widget.handle_owned_screen_split_mouse(event) {
            self.cancel_owned_screen_selection();
            self.clear_owned_screen_tool_group_hover();
            match event.kind {
                MousePrimaryEventKind::Drag => tui
                    .frame_requester()
                    .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL),
                MousePrimaryEventKind::Press | MousePrimaryEventKind::Release => {
                    tui.frame_requester().schedule_frame();
                }
            }
            return true;
        }
        let position = Position::new(event.column, event.row);
        if event.kind != MousePrimaryEventKind::Press {
            let active_slot = [PaneSlot::Parent, PaneSlot::Side].into_iter().find(|slot| {
                self.chat_widget
                    .by_slot(*slot)
                    .and_then(|pane| pane.owned_screen.as_ref())
                    .is_some_and(OwnedScreen::selection_is_active)
            });
            let Some(active_slot) = active_slot else {
                return false;
            };
            let mut copied = None;
            let handled = self
                .chat_widget
                .by_slot_mut(active_slot)
                .and_then(|pane| pane.owned_screen.as_mut())
                .is_some_and(|screen| match event.kind {
                    MousePrimaryEventKind::Drag => screen.update_selection(position),
                    MousePrimaryEventKind::Release => {
                        copied = screen.finish_selection(position);
                        true
                    }
                    MousePrimaryEventKind::Press => false,
                });
            if let Some(text) = copied
                && let Some(pane) = self.chat_widget.by_slot_mut(active_slot)
            {
                pane.chat_widget.copy_selected_text(&text);
                let lease = pane.chat_widget.take_clipboard_lease();
                self.chat_widget.retain_selection_clipboard_lease(lease);
            }
            if handled {
                match event.kind {
                    MousePrimaryEventKind::Drag => tui
                        .frame_requester()
                        .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL),
                    MousePrimaryEventKind::Press | MousePrimaryEventKind::Release => {
                        tui.frame_requester().schedule_frame();
                    }
                }
            }
            return handled;
        }

        let selection_canceled = self.cancel_owned_screen_selection();
        let target = [PaneSlot::Parent, PaneSlot::Side].into_iter().find(|slot| {
            self.chat_widget
                .by_slot(*slot)
                .and_then(|pane| pane.owned_screen.as_ref())
                .is_some_and(|screen| screen.last_pane_area.contains(position))
        });
        let Some(target) = target else {
            let hover_cleared = self.clear_owned_screen_tool_group_hover();
            if selection_canceled || hover_cleared {
                tui.frame_requester().schedule_frame();
            }
            return false;
        };
        let mut hover_cleared = false;
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            if slot != target {
                hover_cleared |= self
                    .chat_widget
                    .by_slot_mut(slot)
                    .and_then(|pane| pane.owned_screen.as_mut())
                    .is_some_and(OwnedScreen::clear_tool_group_hover);
            }
        }
        let selection_started = self.chat_widget.by_slot_mut(target).is_some_and(|pane| {
            pane.chat_widget.no_modal_or_popup_active()
                && pane
                    .owned_screen
                    .as_mut()
                    .is_some_and(|screen| screen.begin_selection(position))
        });
        let focus_changed = self.chat_widget.focused_slot() != target;
        let backtrack_was_primed = self.backtrack.primed;
        if !self.focus_conversation_pane(target) {
            if selection_canceled || hover_cleared {
                tui.frame_requester().schedule_frame();
            }
            return false;
        }
        if selection_started
            || selection_canceled
            || hover_cleared
            || focus_changed
            || backtrack_was_primed
        {
            tui.frame_requester().schedule_frame();
        }
        true
    }

    pub(super) fn cancel_owned_screen_selection(&mut self) -> bool {
        let mut canceled = false;
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            if let Some(screen) = self
                .chat_widget
                .by_slot_mut(slot)
                .and_then(|pane| pane.owned_screen.as_mut())
                && screen.selection_is_active()
            {
                screen.cancel_selection();
                canceled = true;
            }
        }
        canceled
    }

    pub(super) fn clear_owned_screen_tool_group_hover(&mut self) -> bool {
        let mut changed = false;
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            changed |= self
                .chat_widget
                .by_slot_mut(slot)
                .and_then(|pane| pane.owned_screen.as_mut())
                .is_some_and(OwnedScreen::clear_tool_group_hover);
        }
        changed
    }

    pub(crate) fn sync_owned_screen_cells(&mut self) {
        self.sync_owned_screen_cells_with_reason(
            crate::quiet_metrics::QuietMetric::FullProjectionUnexpected,
        );
    }

    pub(super) fn sync_owned_screen_cells_with_reason(
        &mut self,
        reason: crate::quiet_metrics::QuietMetric,
    ) {
        let cells = self.chat_widget.transcript_cells.clone();
        let compact_tool_groups = self.compact_tool_groups_enabled();
        if let Some(screen) = &mut self.chat_widget.owned_screen {
            if screen.source_cells_match(&cells) {
                screen.set_compact_tool_groups(compact_tool_groups);
            } else {
                let _valid_groups = screen.replace_source_cells(cells, compact_tool_groups, reason);
            }
        }
    }

    pub(super) fn replace_owned_screen_source_cell(
        &mut self,
        source_index: usize,
        replacement: Arc<dyn HistoryCell>,
    ) -> bool {
        let compact_tool_groups = self.compact_tool_groups_enabled();
        self.chat_widget
            .owned_screen
            .as_mut()
            .is_some_and(|screen| {
                screen.replace_source_cell(source_index, replacement, compact_tool_groups)
            })
    }

    pub(super) fn splice_owned_screen_source_cells(
        &mut self,
        source_index: usize,
        remove_count: usize,
        replacements: Vec<Arc<dyn HistoryCell>>,
    ) -> bool {
        let compact_tool_groups = self.compact_tool_groups_enabled();
        self.chat_widget
            .owned_screen
            .as_mut()
            .is_some_and(|screen| {
                screen.splice_source_cells(
                    source_index,
                    remove_count,
                    replacements,
                    compact_tool_groups,
                )
            })
    }

    pub(super) fn sync_all_owned_screen_cells(&mut self) {
        self.chat_widget.for_each_installed_mut(|pane| {
            let cells = pane.transcript_cells.clone();
            let compact_tool_groups = pane.chat_widget.history_render_mode()
                == HistoryRenderMode::Rich
                && !pane.compact_tool_groups_expanded;
            if let Some(screen) = &mut pane.owned_screen {
                if screen.source_cells_match(&cells) {
                    screen.set_compact_tool_groups(compact_tool_groups);
                } else {
                    let _valid_groups = screen.replace_source_cells(
                        cells,
                        compact_tool_groups,
                        crate::quiet_metrics::QuietMetric::FullProjectionGlobalMode,
                    );
                }
            }
        });
    }

    pub(super) fn owned_screen_tool_group_inspection_cells(
        &mut self,
    ) -> Option<Vec<Arc<dyn HistoryCell>>> {
        self.chat_widget
            .owned_screen
            .as_mut()
            .and_then(OwnedScreen::inspection_cells)
    }

    pub(super) fn sync_owned_screen_render_mode(&mut self) {
        self.chat_widget.for_each_installed_mut(|pane| {
            let render_mode = pane.chat_widget.history_render_mode();
            if let Some(screen) = &mut pane.owned_screen {
                screen.viewport.set_render_mode(render_mode);
            }
        });
    }

    pub(super) fn sync_owned_screen_keymap(&mut self) {
        let pager_keymap = self.keymap.pager.clone();
        self.chat_widget.for_each_installed_mut(|pane| {
            if let Some(screen) = &mut pane.owned_screen {
                screen.set_keymap(pager_keymap.clone());
            }
        });
    }

    pub(super) fn handle_owned_draw_pre_render(&mut self, tui: &mut tui::Tui) -> Result<bool> {
        if !self.has_owned_screen() {
            return Ok(false);
        }
        let size = tui.terminal.size()?;
        let size_changed = size != tui.terminal.last_known_screen_size;
        for slot in [PaneSlot::Parent, PaneSlot::Side] {
            if let Some(pane) = self.chat_widget.by_slot_mut(slot) {
                if size_changed {
                    pane.chat_widget.refresh_status_line();
                }
                pane.transcript_reflow.clear();
            }
        }
        tui.clear_pending_history_lines();
        Ok(true)
    }

    pub(super) fn schedule_owned_resize_draw(
        &mut self,
        frame_requester: &crate::tui::FrameRequester,
    ) -> bool {
        if !self.has_owned_screen() {
            return false;
        }
        self.chat_widget.for_each_installed_mut(|pane| {
            if pane.owned_screen.is_some() {
                pane.transcript_reflow.schedule_debounced(None);
            }
        });
        frame_requester.schedule_frame_in(crate::transcript_reflow::TRANSCRIPT_REFLOW_DEBOUNCE);
        true
    }

    pub(super) fn defer_owned_draw_until_resize_quiet(
        &mut self,
        frame_requester: &crate::tui::FrameRequester,
    ) -> bool {
        if !self.has_owned_screen() {
            return false;
        }
        let mut latest_deadline = None;
        self.chat_widget.for_each_installed_mut(|pane| {
            if pane.owned_screen.is_some()
                && let Some(deadline) = pane.transcript_reflow.pending_until()
            {
                latest_deadline = Some(
                    latest_deadline.map_or(deadline, |current: std::time::Instant| {
                        current.max(deadline)
                    }),
                );
            }
        });
        let Some(deadline) = latest_deadline else {
            return false;
        };
        let now = std::time::Instant::now();
        if now < deadline {
            frame_requester.schedule_frame_in(deadline - now);
            return true;
        }
        self.chat_widget.for_each_installed_mut(|pane| {
            if pane.owned_screen.is_some() {
                pane.transcript_reflow.clear_pending_reflow();
            }
        });
        false
    }

    pub(super) fn render_owned_screen_frame(&mut self, tui: &mut tui::Tui) -> Result<Option<Rect>> {
        if !self.has_owned_screen() {
            return Ok(None);
        }
        if !self.chat_widget.no_modal_or_popup_active() {
            self.chat_widget.cancel_owned_screen_split_drag();
            self.cancel_owned_screen_selection();
            self.clear_owned_screen_tool_group_hover();
        }
        let focused = self.chat_widget.focused_slot();
        let has_side = self.chat_widget.has_side();
        let split_preference = self.chat_widget.owned_screen_split_preference();
        let mut rendered_area = Rect::default();
        let mut selection_autoscroll_active = false;
        tui.draw(/*height*/ u16::MAX, |frame| {
            rendered_area = frame.area();
            let layout = OwnedScreenLayout::new(rendered_area, has_side, focused, split_preference);
            if let Some(rendered) =
                render_layout(&mut self.chat_widget, layout, focused, frame.buffer)
            {
                selection_autoscroll_active = rendered.selection_autoscroll_active;
                if let Some((x, y)) = rendered.cursor {
                    frame.set_cursor_style(rendered.cursor_style);
                    frame.set_cursor_position((x, y));
                }
            }
        })?;
        if selection_autoscroll_active {
            tui.frame_requester()
                .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
        }
        Ok(Some(rendered_area))
    }

    pub(super) fn render_owned_activity_frame(&mut self, tui: &mut tui::Tui) -> Result<bool> {
        let Some(area) = activity_bottom_band(&self.chat_widget) else {
            return Ok(false);
        };
        let focused = self.chat_widget.focused_slot();
        if let Some(pane) = self.chat_widget.by_slot_mut(focused) {
            pane.chat_widget.refresh_terminal_title();
        }
        let chat_widget = &mut self.chat_widget;
        Ok(tui.draw_partial(area, |frame| {
            if let Some(rendered) =
                render_activity_bottom_panes(chat_widget, focused, frame.buffer_mut())
                && let Some((x, y)) = rendered.cursor
            {
                frame.set_cursor_style(rendered.cursor_style);
                frame.set_cursor_position((x, y));
            }
        })?)
    }
}

#[cfg(test)]
#[path = "owned_screen_tests.rs"]
mod tests;
