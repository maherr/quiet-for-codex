//! Retained conversation rendering for an application-owned terminal viewport.
//!
//! This module intentionally owns only transcript projection and bottom-follow state. The app
//! decides when the terminal is owned and how much space remains after the bottom pane is laid
//! out. Committed cells use their main-viewport representation; the more detailed `Ctrl+T`
//! representation remains owned by `pager_overlay`.

use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::chatwidget::ActiveCellDisplaySnapshot;
use crate::chatwidget::ActiveCellLane;
use crate::chatwidget::ActiveCellRenderKey;
use crate::conversation_selection::CellSelectionProjection;
use crate::conversation_selection::ConversationSelection;
use crate::conversation_selection::SelectionCellLayout;
use crate::conversation_selection::SelectionPoint;
use crate::conversation_selection_autoscroll::AutoscrollDirection;
use crate::conversation_selection_autoscroll::SelectionAutoscroll;
use crate::conversation_selection_reflow::SelectionBookmarks;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::keymap::PagerKeymap;
use crate::pager_overlay::BottomFollowMode;
use crate::pager_overlay::PagerContent;
use crate::render::Insets;
use crate::render::line_utils::fill_line_backgrounds_to_width;
use crate::render::renderable::InsetRenderable;
use crate::render::renderable::Renderable;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::mark_buffer_hyperlinks;
use crate::terminal_hyperlinks::visible_lines_ref;
use crate::tui::MouseScrollDirection;

pub(crate) struct ConversationViewport {
    content: PagerContent,
    cells: Vec<Arc<dyn HistoryCell>>,
    render_mode: HistoryRenderMode,
    live_tail_key: Option<LiveTailKey>,
    live_primary_cells: Vec<Arc<ActiveCellDisplaySnapshot>>,
    live_token_cells: Vec<Arc<ActiveCellDisplaySnapshot>>,
    live_rate_limit_cells: Vec<Arc<ActiveCellDisplaySnapshot>>,
    live_cells: Vec<Arc<ActiveCellDisplaySnapshot>>,
    deferred_cells: Option<Vec<Arc<dyn HistoryCell>>>,
    deferred_render_mode: Option<HistoryRenderMode>,
    selection: ConversationSelection,
    selection_pointer: Option<Position>,
    selection_projection_cache: Option<SelectionProjectionCache>,
    selection_autoscroll: SelectionAutoscroll,
    selection_autoscroll_last_tick: Option<Instant>,
    freeze_bottom_follow_once: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ConversationCellHit {
    pub(crate) index: usize,
    pub(crate) row_within_cell: usize,
}

struct SelectionProjectionCache {
    width: u16,
    render_mode: HistoryRenderMode,
    projections: Vec<Option<CellSelectionProjection>>,
    computed: Vec<bool>,
    layout: Vec<SelectionCellLayout>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LiveTailKey {
    width: u16,
    revision: u64,
    primary_present: bool,
    is_stream_continuation: bool,
    token_activity_revision: Option<u64>,
    rate_limit_revision: Option<u64>,
}

impl ConversationViewport {
    pub(crate) fn new(
        cells: Vec<Arc<dyn HistoryCell>>,
        render_mode: HistoryRenderMode,
        keymap: PagerKeymap,
    ) -> Self {
        let renderables = Self::render_cells(&cells, render_mode);
        Self {
            content: PagerContent::new(renderables, keymap),
            cells,
            render_mode,
            live_tail_key: None,
            live_primary_cells: Vec::new(),
            live_token_cells: Vec::new(),
            live_rate_limit_cells: Vec::new(),
            live_cells: Vec::new(),
            deferred_cells: None,
            deferred_render_mode: None,
            selection: ConversationSelection::default(),
            selection_pointer: None,
            selection_projection_cache: None,
            selection_autoscroll: SelectionAutoscroll::default(),
            selection_autoscroll_last_tick: None,
            freeze_bottom_follow_once: false,
        }
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let freeze_bottom_follow = std::mem::take(&mut self.freeze_bottom_follow_once);
        let bottom_follow = if self.selection.is_active() || freeze_bottom_follow {
            BottomFollowMode::Frozen
        } else {
            BottomFollowMode::Enabled
        };
        if self.selection_projection_cache.is_some() {
            self.ensure_selection_projections(area.width);
        }
        self.content.render(area, buf, bottom_follow);
        if self.selection_projection_cache.is_some() {
            self.ensure_selected_projections();
            self.render_selection(area, buf);
        }
    }

    pub(crate) fn handle_navigation_key(&mut self, area: Rect, key_event: KeyEvent) -> bool {
        self.content.handle_navigation_key(area, key_event)
    }

    pub(crate) fn set_keymap(&mut self, keymap: PagerKeymap) {
        self.content.set_keymap(keymap);
    }

    pub(crate) fn handle_mouse_scroll(&mut self, direction: MouseScrollDirection) {
        self.content.handle_mouse_scroll(direction);
    }

    pub(crate) fn handle_selection_mouse_scroll(
        &mut self,
        area: Rect,
        direction: MouseScrollDirection,
        position: Position,
    ) {
        self.content.scroll_mouse_wheel_rows(direction);
        self.update_selection(area, position);
    }

    pub(crate) fn begin_selection(&mut self, area: Rect, position: Position) -> bool {
        let Some(point) = self.selection_point(area, position, /*clamp*/ false) else {
            return false;
        };
        self.stop_selection_autoscroll();
        self.selection_pointer = None;
        self.selection.start(point);
        true
    }

    pub(crate) fn update_selection(&mut self, area: Rect, position: Position) -> bool {
        let Some(point) = self.selection_point(area, position, /*clamp*/ true) else {
            return false;
        };
        let updated = self.selection.update(point);
        if updated {
            self.selection_pointer = Some(position);
            let was_active = self.selection_autoscroll.needs_frame();
            self.selection_autoscroll.update_pointer(area, position);
            match (was_active, self.selection_autoscroll.needs_frame()) {
                (false, true) => self.selection_autoscroll_last_tick = Some(Instant::now()),
                (_, false) => self.selection_autoscroll_last_tick = None,
                (true, true) => {}
            }
        }
        updated
    }

    pub(crate) fn finish_selection(&mut self, area: Rect, position: Position) -> Option<String> {
        let release_matches_last_drag = self.selection_pointer == Some(position);
        self.stop_selection_autoscroll();
        self.selection_pointer = None;
        let point = if release_matches_last_drag {
            None
        } else {
            self.selection_point(area, position, /*clamp*/ true)
        };
        self.selection.set_release_point(point);
        self.ensure_selected_projections();
        let selected = if let Some(cache) = self.selection_projection_cache.as_ref() {
            self.selection
                .finish(/*point*/ None, &cache.projections, &cache.layout)
        } else {
            self.selection.finish(/*point*/ None, &[], &[])
        };
        self.apply_deferred_state();
        selected
    }

    pub(crate) fn cancel_selection(&mut self) {
        self.stop_selection_autoscroll();
        self.selection_pointer = None;
        self.selection.cancel();
        self.apply_deferred_state();
    }

    pub(crate) fn selection_is_active(&self) -> bool {
        self.selection.is_active()
    }

    pub(crate) fn committed_cell_hit(
        &mut self,
        area: Rect,
        position: Position,
    ) -> Option<ConversationCellHit> {
        if area.is_empty() || !area.contains(position) {
            return None;
        }
        let screen_row = usize::from(position.y.saturating_sub(area.y));
        let content_row = self.content.scroll_offset().saturating_add(screen_row);
        let (index, row_within_cell) = self.content.renderable_hit(area.width, content_row)?;
        if index >= self.cells.len() {
            return None;
        }
        Some(ConversationCellHit {
            index,
            row_within_cell,
        })
    }

    pub(crate) fn committed_cell(&self, index: usize) -> Option<&Arc<dyn HistoryCell>> {
        self.cells.get(index)
    }

    pub(crate) fn scroll_offset(&self) -> usize {
        self.content.scroll_offset()
    }

    pub(crate) fn preserve_scroll_offset_through_next_render(&mut self, scroll_offset: usize) {
        self.content.set_scroll_offset(scroll_offset);
        self.freeze_bottom_follow_once = true;
    }

    fn should_follow_bottom(&self) -> bool {
        !self.freeze_bottom_follow_once && self.content.is_following_bottom()
    }

    pub(crate) fn advance_selection_autoscroll(&mut self, area: Rect, now: Instant) -> bool {
        let Some(pointer) = self.selection_autoscroll.pointer() else {
            return false;
        };
        self.selection_autoscroll.update_pointer(area, pointer);
        if !self.selection_autoscroll.needs_frame() {
            self.selection_autoscroll_last_tick = None;
            return false;
        }
        let elapsed = self
            .selection_autoscroll_last_tick
            .replace(now)
            .map_or(Duration::ZERO, |last_tick| {
                now.saturating_duration_since(last_tick)
            });
        self.advance_selection_autoscroll_by(area, elapsed)
    }

    pub(crate) fn prepare_selection_autoscroll(&mut self, area: Rect, now: Instant) -> bool {
        let Some(pointer) = self.selection_autoscroll.pointer() else {
            return false;
        };
        self.selection_autoscroll.update_pointer(area, pointer);
        let needs_frame = self.selection_autoscroll.needs_frame();
        self.selection_autoscroll_last_tick = needs_frame.then_some(now);
        needs_frame
    }

    fn advance_selection_autoscroll_by(&mut self, area: Rect, elapsed: Duration) -> bool {
        let Some(step) = self.selection_autoscroll.advance(elapsed) else {
            return self.selection_autoscroll.needs_frame();
        };
        let direction = match step.direction {
            AutoscrollDirection::Up => MouseScrollDirection::Up,
            AutoscrollDirection::Down => MouseScrollDirection::Down,
        };
        let moved = self.content.scroll_rows(direction, step.rows);
        if moved == 0 {
            self.stop_selection_autoscroll();
            return false;
        }
        if let Some(point) = self.selection_point(area, step.pointer, /*clamp*/ true) {
            self.selection.update(point);
        }
        if moved < step.rows {
            self.stop_selection_autoscroll();
            return false;
        }
        true
    }

    fn stop_selection_autoscroll(&mut self) {
        self.selection_autoscroll.reset();
        self.selection_autoscroll_last_tick = None;
    }

    pub(crate) fn push_cell(&mut self, cell: Arc<dyn HistoryCell>) {
        if self.selection.is_active() {
            self.deferred_cells
                .get_or_insert_with(|| self.cells.clone())
                .push(cell);
            return;
        }
        self.invalidate_selection_projections();
        let follow_bottom = self.should_follow_bottom();
        let had_prior_cells = !self.cells.is_empty();
        self.take_live_tail_renderables();
        let renderable = Self::cell_renderable(
            cell.clone(),
            self.render_mode,
            /*has_prior_cells*/ had_prior_cells,
        );
        self.cells.push(cell);
        self.content.push(renderable);
        self.push_live_tail_renderables();
        if follow_bottom {
            self.content.scroll_to_bottom();
        }
    }

    /// Replaces one committed source range without rebuilding the retained prefix or suffix.
    pub(crate) fn replace_range(
        &mut self,
        width: u16,
        start: usize,
        remove_count: usize,
        replacement: Vec<Arc<dyn HistoryCell>>,
    ) {
        crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::RangeReplacement);
        let start = start.min(self.cells.len());
        let end = start.saturating_add(remove_count).min(self.cells.len());
        if self.selection.is_active() {
            let cells = self
                .deferred_cells
                .get_or_insert_with(|| self.cells.clone());
            cells.splice(start..end, replacement);
            return;
        }

        self.invalidate_selection_projections();
        let follow_bottom = self.should_follow_bottom();
        self.take_live_tail_renderables();
        let scroll_offset = self.content.scroll_offset();
        let replacement_len = replacement.len();
        let (old_top, old_bottom) =
            self.content
                .renderable_range_rows(width, start, end.saturating_sub(start));
        let mut next_renderables = Vec::with_capacity(replacement.len());
        for (offset, cell) in replacement.iter().enumerate() {
            next_renderables.push(Self::cell_renderable(
                cell.clone(),
                self.render_mode,
                start.saturating_add(offset) > 0,
            ));
        }
        self.cells.splice(start..end, replacement);
        self.content
            .replace_range(start, end.saturating_sub(start), next_renderables);

        let (new_top, new_bottom) =
            self.content
                .renderable_range_rows(width, start, replacement_len);
        self.push_live_tail_renderables();
        if follow_bottom {
            self.content.scroll_to_bottom();
        } else if old_bottom <= scroll_offset {
            let old_height = old_bottom.saturating_sub(old_top);
            let new_height = new_bottom.saturating_sub(new_top);
            let adjusted = if new_height >= old_height {
                scroll_offset.saturating_add(new_height - old_height)
            } else {
                scroll_offset.saturating_sub(old_height - new_height)
            };
            self.content.set_scroll_offset(adjusted);
        }
    }

    pub(crate) fn committed_cell_position(&self, target: &Arc<dyn HistoryCell>) -> Option<usize> {
        self.cells.iter().position(|cell| Arc::ptr_eq(cell, target))
    }

    pub(crate) fn replace_cells(&mut self, cells: Vec<Arc<dyn HistoryCell>>) {
        if self.selection.is_active() {
            self.deferred_cells = Some(cells);
            return;
        }
        self.invalidate_selection_projections();
        let follow_bottom = self.should_follow_bottom();
        self.take_live_tail_renderables();
        self.live_tail_key = None;
        self.live_primary_cells.clear();
        self.live_token_cells.clear();
        self.live_rate_limit_cells.clear();
        self.live_cells.clear();
        let retained_prefix = self
            .cells
            .iter()
            .zip(&cells)
            .take_while(|(current, replacement)| Arc::ptr_eq(current, replacement))
            .count();
        while self.cells.len() > retained_prefix {
            self.cells.pop();
            self.content.pop();
        }
        for cell in cells.into_iter().skip(retained_prefix) {
            let has_prior_cells = !self.cells.is_empty();
            let renderable = Self::cell_renderable(cell.clone(), self.render_mode, has_prior_cells);
            self.cells.push(cell);
            self.content.push(renderable);
        }
        if follow_bottom {
            self.content.scroll_to_bottom();
        }
    }

    pub(crate) fn set_render_mode(&mut self, render_mode: HistoryRenderMode) {
        let effective_render_mode = self.deferred_render_mode.unwrap_or(self.render_mode);
        if effective_render_mode == render_mode {
            return;
        }
        if self.selection.is_active() {
            self.deferred_render_mode = Some(render_mode);
            return;
        }
        self.invalidate_selection_projections();
        let follow_bottom = self.should_follow_bottom();
        self.take_live_tail_renderables();
        self.live_tail_key = None;
        self.live_primary_cells.clear();
        self.live_token_cells.clear();
        self.live_rate_limit_cells.clear();
        self.live_cells.clear();
        self.render_mode = render_mode;
        self.content
            .replace(Self::render_cells(&self.cells, self.render_mode));
        if follow_bottom {
            self.content.scroll_to_bottom();
        }
    }

    pub(crate) fn sync_live_tail(
        &mut self,
        width: u16,
        active_key: Option<ActiveCellRenderKey>,
        mut compute_cells: impl FnMut(u16, ActiveCellLane) -> Option<Vec<ActiveCellDisplaySnapshot>>,
    ) {
        // A drag's screen coordinates and source projections must describe the same immutable
        // content. Active cells can mutate on every output delta or animation tick, so defer all
        // live-tail changes until the drag completes or is cancelled.
        if self.selection.is_active() {
            let width_is_stable = self
                .selection_projection_cache
                .as_ref()
                .is_some_and(|cache| cache.width == width);
            if width_is_stable {
                return;
            }
            match (self.live_tail_key, active_key) {
                (None, _) => return,
                (Some(current), Some(next))
                    if current.revision == next.revision
                        && current.primary_present == next.primary_present
                        && current.is_stream_continuation == next.is_stream_continuation
                        && current.token_activity_revision == next.token_activity_revision
                        && current.rate_limit_revision == next.rate_limit_revision =>
                {
                    // The semantic live tail is unchanged, so it is safe to re-render it at the
                    // new width before remapping the selection's source-backed endpoints.
                }
                _ => self.cancel_selection(),
            }
        }
        let next_key = active_key.map(|key| LiveTailKey {
            width,
            revision: key.revision,
            primary_present: key.primary_present,
            is_stream_continuation: key.is_stream_continuation,
            token_activity_revision: key.token_activity_revision,
            rate_limit_revision: key.rate_limit_revision,
        });
        if self.live_tail_key == next_key {
            return;
        }

        let previous_key = self.live_tail_key;
        let width_changed = previous_key.map(|key| key.width) != next_key.map(|key| key.width);
        let primary_changed = width_changed
            || previous_key.map(|key| {
                (
                    key.revision,
                    key.primary_present,
                    key.is_stream_continuation,
                )
            }) != next_key.map(|key| {
                (
                    key.revision,
                    key.primary_present,
                    key.is_stream_continuation,
                )
            });
        let token_changed = width_changed
            || previous_key.map(|key| key.token_activity_revision)
                != next_key.map(|key| key.token_activity_revision);
        let rate_limit_changed = width_changed
            || previous_key.map(|key| key.rate_limit_revision)
                != next_key.map(|key| key.rate_limit_revision);
        let follow_bottom = !self.selection.is_active() && self.should_follow_bottom();
        self.take_live_tail_renderables();
        if !self.selection.is_active() {
            self.selection_projection_cache = None;
        }
        self.live_tail_key = next_key;
        if primary_changed {
            self.live_primary_cells.clear();
            if next_key.is_some_and(|key| key.primary_present) {
                crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::LivePrimaryRebuild);
                self.live_primary_cells = compute_cells(width, ActiveCellLane::Primary)
                    .unwrap_or_default()
                    .into_iter()
                    .map(Arc::new)
                    .collect();
            }
        }
        if token_changed {
            self.live_token_cells.clear();
            if next_key.is_some_and(|key| key.token_activity_revision.is_some()) {
                crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::LiveTokenRebuild);
                self.live_token_cells = compute_cells(width, ActiveCellLane::TokenActivity)
                    .unwrap_or_default()
                    .into_iter()
                    .map(Arc::new)
                    .collect();
            }
        }
        if rate_limit_changed {
            self.live_rate_limit_cells.clear();
            if next_key.is_some_and(|key| key.rate_limit_revision.is_some()) {
                crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::LiveRateLimitRebuild);
                self.live_rate_limit_cells = compute_cells(width, ActiveCellLane::RateLimit)
                    .unwrap_or_default()
                    .into_iter()
                    .map(Arc::new)
                    .collect();
            }
        }
        self.live_cells.clear();
        self.live_cells
            .extend(self.live_primary_cells.iter().cloned());
        self.live_cells
            .extend(self.live_token_cells.iter().cloned());
        self.live_cells
            .extend(self.live_rate_limit_cells.iter().cloned());
        #[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
        {
            let live_rows = self
                .live_cells
                .iter()
                .map(|cell| {
                    u64::try_from(
                        Paragraph::new(Text::from(visible_lines_ref(&cell.lines)))
                            .wrap(Wrap { trim: false })
                            .line_count(width),
                    )
                    .unwrap_or(u64::MAX)
                })
                .sum();
            crate::quiet_metrics::record_max(
                crate::quiet_metrics::QuietMetric::LiveTailMaxRows,
                live_rows,
            );
        }
        if next_key.is_some() {
            self.push_live_tail_renderables();
        }
        if follow_bottom {
            self.content.scroll_to_bottom();
        }
    }

    pub(crate) fn is_following_bottom(&self) -> bool {
        self.content.is_following_bottom()
    }

    pub(crate) fn committed_cell_count(&self) -> usize {
        self.cells.len()
    }

    pub(crate) fn committed_range_intersects_viewport(
        &mut self,
        width: u16,
        start: usize,
        count: usize,
        area: Rect,
    ) -> bool {
        if area.is_empty() || count == 0 {
            return false;
        }
        let (top, bottom) = self.content.renderable_range_rows(width, start, count);
        let viewport_top = self.content.scroll_offset();
        let viewport_bottom = viewport_top.saturating_add(usize::from(area.height));
        top < viewport_bottom && bottom > viewport_top
    }

    pub(crate) fn visible_committed_cell_range(&mut self, area: Rect) -> Option<(usize, usize)> {
        if area.is_empty() || self.cells.is_empty() {
            return None;
        }
        let top = self.content.scroll_offset();
        let bottom = top.saturating_add(usize::from(area.height.saturating_sub(1)));
        let first = self.content.renderable_hit(area.width, top)?.0;
        let last = self.content.renderable_hit(area.width, bottom)?.0;
        let max = self.cells.len().saturating_sub(1);
        Some((first.min(max), last.min(max)))
    }

    fn render_cells(
        cells: &[Arc<dyn HistoryCell>],
        render_mode: HistoryRenderMode,
    ) -> Vec<Box<dyn Renderable>> {
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                Self::cell_renderable(
                    cell.clone(),
                    render_mode,
                    /*has_prior_cells*/ index > 0,
                )
            })
            .collect()
    }

    fn cell_renderable(
        cell: Arc<dyn HistoryCell>,
        render_mode: HistoryRenderMode,
        has_prior_cells: bool,
    ) -> Box<dyn Renderable> {
        let is_stream_continuation = cell.is_stream_continuation();
        let renderable: Box<dyn Renderable> = Box::new(ConversationCellRenderable {
            cell,
            render_mode,
            prepared: RefCell::new(None),
        });
        if has_prior_cells && !is_stream_continuation {
            Self::with_leading_spacing(renderable)
        } else {
            renderable
        }
    }

    fn live_tail_renderable(
        lines: Arc<[HyperlinkLine]>,
        has_prior_cells: bool,
        is_stream_continuation: bool,
    ) -> Box<dyn Renderable> {
        let renderable: Box<dyn Renderable> = Box::new(HyperlinkLinesRenderable {
            lines,
            cached_height: Cell::new(None),
        });
        if has_prior_cells && !is_stream_continuation {
            Self::with_leading_spacing(renderable)
        } else {
            renderable
        }
    }

    fn push_live_tail_renderables(&mut self) {
        let mut has_prior_cells = !self.cells.is_empty();
        for cell in &self.live_cells {
            self.content.push(Self::live_tail_renderable(
                Arc::clone(&cell.lines),
                has_prior_cells,
                cell.is_stream_continuation,
            ));
            has_prior_cells = true;
        }
    }

    fn with_leading_spacing(renderable: Box<dyn Renderable>) -> Box<dyn Renderable> {
        Box::new(InsetRenderable::new(
            renderable,
            Insets::tlbr(
                /*top*/ 1, /*left*/ 0, /*bottom*/ 0, /*right*/ 0,
            ),
        ))
    }

    fn take_live_tail_renderables(&mut self) {
        while self.content.len() > self.cells.len() {
            self.content.pop();
        }
    }

    fn invalidate_selection_projections(&mut self) {
        self.cancel_selection();
        self.selection_projection_cache = None;
    }

    fn apply_deferred_state(&mut self) {
        let deferred_cells = self.deferred_cells.take();
        let deferred_render_mode = self.deferred_render_mode.take();
        if deferred_cells.is_none() && deferred_render_mode.is_none() {
            return;
        }

        let follow_bottom = self.should_follow_bottom();
        self.take_live_tail_renderables();
        self.live_tail_key = None;
        self.live_primary_cells.clear();
        self.live_token_cells.clear();
        self.live_rate_limit_cells.clear();
        self.live_cells.clear();
        if let Some(cells) = deferred_cells {
            self.cells = cells;
        }
        if let Some(render_mode) = deferred_render_mode {
            self.render_mode = render_mode;
        }
        self.content
            .replace(Self::render_cells(&self.cells, self.render_mode));
        self.selection_projection_cache = None;
        if follow_bottom {
            self.content.scroll_to_bottom();
        }
    }

    fn selection_bookmarks(&mut self) -> Option<SelectionBookmarks> {
        let (anchor, focus) = self.selection.endpoints()?;
        for point in [anchor, focus] {
            let cell = self
                .selection_projection_cache
                .as_ref()?
                .layout
                .iter()
                .position(|layout| {
                    layout.height > 0
                        && point.row >= layout.top
                        && point.row < layout.top.saturating_add(layout.height)
                });
            if let Some(cell) = cell {
                self.ensure_cell_projection(cell);
            }
        }
        let cache = self.selection_projection_cache.as_ref()?;
        SelectionBookmarks::capture(
            &self.selection,
            &cache.projections,
            &cache.layout,
            self.content.scroll_offset(),
        )
    }

    fn restore_selection_bookmarks(&mut self, bookmarks: SelectionBookmarks) -> bool {
        let projected_cells = bookmarks.projected_cells().collect::<Vec<_>>();
        for cell in projected_cells {
            self.ensure_cell_projection(cell);
        }
        let Some(restored) = self
            .selection_projection_cache
            .as_ref()
            .and_then(|cache| bookmarks.restore(&cache.projections, &cache.layout, cache.width))
        else {
            return false;
        };
        self.selection
            .remap_endpoints(restored.anchor, restored.focus);
        self.content.set_scroll_offset(restored.scroll_offset);
        true
    }

    fn ensure_selection_projections(&mut self, width: u16) {
        let current_cell_count = self.cells.len().saturating_add(self.live_cells.len());
        let cache_matches = self
            .selection_projection_cache
            .as_ref()
            .is_some_and(|cache| {
                cache.width == width
                    && cache.render_mode == self.render_mode
                    && cache.projections.len() == current_cell_count
            });
        if cache_matches {
            return;
        }
        let selection_was_active = self.selection.is_active();
        let bookmarks = selection_was_active
            .then(|| self.selection_bookmarks())
            .flatten();
        let selection_cell_count = self.cells.len().saturating_add(self.live_cells.len());
        let renderable_heights = self.content.renderable_heights(width);
        let mut content_top = 0usize;
        let mut layout = Vec::with_capacity(selection_cell_count);
        for index in 0..selection_cell_count {
            let (is_stream_continuation, live_cell) = if let Some(cell) = self.cells.get(index) {
                (cell.is_stream_continuation(), None)
            } else {
                let Some(cell) = self.live_cells.get(index.saturating_sub(self.cells.len())) else {
                    return;
                };
                (cell.is_stream_continuation, Some(cell))
            };
            let leading_spacing = usize::from(index > 0 && !is_stream_continuation);
            let renderable_height = if let Some(height) = renderable_heights.get(index).copied() {
                height
            } else {
                let Some(cell) = live_cell else {
                    return;
                };
                HyperlinkLinesRenderable {
                    lines: Arc::clone(&cell.lines),
                    cached_height: Cell::new(None),
                }
                .desired_height(width)
            };
            layout.push(SelectionCellLayout {
                top: content_top.saturating_add(leading_spacing),
                height: usize::from(renderable_height).saturating_sub(leading_spacing),
            });
            content_top = content_top.saturating_add(usize::from(renderable_height));
        }
        let projections = vec![None; selection_cell_count];
        let computed = vec![false; selection_cell_count];
        self.selection_projection_cache = Some(SelectionProjectionCache {
            width,
            render_mode: self.render_mode,
            projections,
            computed,
            layout,
        });
        if selection_was_active
            && !bookmarks.is_some_and(|bookmarks| self.restore_selection_bookmarks(bookmarks))
        {
            self.cancel_selection();
        }
    }

    fn ensure_cell_projection(&mut self, index: usize) {
        let Some(cache) = self.selection_projection_cache.as_ref() else {
            return;
        };
        if cache
            .computed
            .get(index)
            .copied()
            .unwrap_or(/*default*/ true)
        {
            return;
        }
        let width = cache.width;
        let render_mode = cache.render_mode;
        let (projection, is_stream_continuation) = if let Some(cell) = self.cells.get(index) {
            (
                cell.selection_contribution(width, render_mode)
                    .into_projection(),
                cell.is_stream_continuation(),
            )
        } else {
            let Some(cell) = self.live_cells.get(index.saturating_sub(self.cells.len())) else {
                return;
            };
            (
                cell.selection_projection.resolve(&cell.lines),
                cell.is_stream_continuation,
            )
        };
        let separator = if index > 0 && is_stream_continuation {
            "\n"
        } else {
            "\n\n"
        };
        let projection =
            projection.map(|projection| projection.with_default_separator_before(separator));
        if let Some(cache) = self.selection_projection_cache.as_mut() {
            cache.projections[index] = projection;
            cache.computed[index] = true;
        }
    }

    fn ensure_selected_projections(&mut self) {
        let Some(layout) = self
            .selection_projection_cache
            .as_ref()
            .map(|cache| cache.layout.as_slice())
        else {
            return;
        };
        let Some(selected_cells) = self.selection.selected_cell_span(layout) else {
            return;
        };
        for index in selected_cells {
            self.ensure_cell_projection(index);
        }
    }

    fn selection_point(
        &mut self,
        area: Rect,
        position: Position,
        clamp: bool,
    ) -> Option<SelectionPoint> {
        if area.is_empty() {
            return None;
        }
        self.ensure_selection_projections(area.width);
        let column = position
            .x
            .clamp(area.x, area.right().saturating_sub(1))
            .saturating_sub(area.x);
        let screen_row = position
            .y
            .clamp(area.y, area.bottom().saturating_sub(1))
            .saturating_sub(area.y);
        if !clamp && !area.contains(position) {
            return None;
        }
        let content_row = self
            .content
            .scroll_offset()
            .saturating_add(usize::from(screen_row));
        Some(SelectionPoint {
            row: content_row,
            column,
        })
    }

    fn render_selection(&self, area: Rect, buf: &mut Buffer) {
        let Some(cache) = self.selection_projection_cache.as_ref() else {
            return;
        };
        let Some(selected_cells) = self.selection.selected_cell_span(&cache.layout) else {
            return;
        };
        let scroll_offset = self.content.scroll_offset();
        for cell_index in selected_cells {
            let Some(layout) = cache.layout.get(cell_index).copied() else {
                continue;
            };
            let Some(projection) = cache.projections.get(cell_index).and_then(Option::as_ref)
            else {
                continue;
            };
            for (row_index, row) in projection.rows().iter().enumerate() {
                let content_row = layout.top.saturating_add(row_index);
                let Some(screen_row) = content_row.checked_sub(scroll_offset) else {
                    continue;
                };
                let Ok(screen_row) = u16::try_from(screen_row) else {
                    continue;
                };
                if screen_row >= area.height {
                    continue;
                }
                for segment in &row.segments {
                    if !self
                        .selection
                        .segment_is_selected(layout, row_index, &segment.columns)
                    {
                        continue;
                    }
                    for column in segment.columns.clone() {
                        if column < area.width {
                            buf[(
                                area.x.saturating_add(column),
                                area.y.saturating_add(screen_row),
                            )]
                                .modifier
                                .insert(Modifier::REVERSED);
                        }
                    }
                }
            }
        }
    }
}

struct ConversationCellRenderable {
    cell: Arc<dyn HistoryCell>,
    render_mode: HistoryRenderMode,
    prepared: RefCell<Option<PreparedConversationCell>>,
}

struct PreparedConversationCell {
    width: u16,
    hyperlink_lines: Arc<[HyperlinkLine]>,
    height: u16,
}

impl ConversationCellRenderable {
    fn prepare(&self, width: u16) -> (Arc<[HyperlinkLine]>, u16) {
        if let Some(prepared) = self.prepared.borrow().as_ref()
            && prepared.width == width
        {
            crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::PreparedCellCacheHit);
            return (Arc::clone(&prepared.hyperlink_lines), prepared.height);
        }
        if self.prepared.borrow().is_some() {
            crate::quiet_metrics::bump(
                crate::quiet_metrics::QuietMetric::PreparedCellCacheInvalidation,
            );
        }
        crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::PreparedCellCacheMiss);
        crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::LayoutStableCellMeasurement);
        let hyperlink_lines = self
            .cell
            .display_hyperlink_lines_shared_for_mode(width, self.render_mode);
        let height = Paragraph::new(Text::from(visible_lines_ref(&hyperlink_lines)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(/*default*/ 0);
        *self.prepared.borrow_mut() = Some(PreparedConversationCell {
            width,
            hyperlink_lines: Arc::clone(&hyperlink_lines),
            height,
        });
        (hyperlink_lines, height)
    }
}

impl Renderable for ConversationCellRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        crate::quiet_metrics::bump(crate::quiet_metrics::QuietMetric::LayoutVisibleCellDraw);
        let (hyperlink_lines, _) = self.prepare(area.width);
        let block_style = match self.render_mode {
            HistoryRenderMode::Rich => self.cell.rich_block_style().unwrap_or_default(),
            HistoryRenderMode::Raw => Default::default(),
        };
        let mut lines = visible_lines_ref(&hyperlink_lines);
        fill_line_backgrounds_to_width(&mut lines, area.width);
        Paragraph::new(Text::from(lines))
            .style(block_style)
            .wrap(Wrap { trim: false })
            .render(area, buf);
        mark_buffer_hyperlinks(buf, area, &hyperlink_lines, /*scroll_rows*/ 0);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.prepare(width).1
    }

    fn has_stable_height(&self) -> bool {
        true
    }
}

struct HyperlinkLinesRenderable {
    lines: Arc<[HyperlinkLine]>,
    cached_height: Cell<Option<(u16, u16)>>,
}

impl Renderable for HyperlinkLinesRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut lines = visible_lines_ref(&self.lines);
        fill_line_backgrounds_to_width(&mut lines, area.width);
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .render(area, buf);
        mark_buffer_hyperlinks(buf, area, &self.lines, /*scroll_rows*/ 0);
    }

    fn desired_height(&self, width: u16) -> u16 {
        if let Some((cached_width, height)) = self.cached_height.get()
            && cached_width == width
        {
            return height;
        }
        let height = Paragraph::new(Text::from(visible_lines_ref(&self.lines)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(/*default*/ 0);
        self.cached_height.set(Some((width, height)));
        height
    }

    fn has_stable_height(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "conversation_viewport_tests.rs"]
mod tests;
