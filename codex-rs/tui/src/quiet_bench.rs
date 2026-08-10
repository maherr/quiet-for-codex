//! Public shim used only by the `quiet-render-bench` feature-gated benchmark binary.

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use crate::conversation_viewport::ConversationViewport;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::SelectionContribution;
use crate::history_cell::selection_contribution_from_display_lines;
use crate::keymap::RuntimeKeymap;
use crate::quiet_metrics;
use crate::quiet_metrics::QuietMetric;

#[derive(Debug)]
struct BenchCell {
    text: String,
}

impl HistoryCell for BenchCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![Line::from(self.text.clone())]
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(u16::MAX)
    }

    fn selection_contribution(&self, width: u16, mode: HistoryRenderMode) -> SelectionContribution {
        selection_contribution_from_display_lines(self.display_lines_for_mode(width, mode), width)
    }
}

/// Warm, 4,096-cell retained-frame fixture used by Divan.
pub struct OwnedFrameFixture {
    viewport: ConversationViewport,
    conversation_area: Rect,
    footer_area: Rect,
    buffer: Buffer,
    status_tick: u64,
    input_down: bool,
}

/// Footer-only diagnostic control without retained transcript redraw work.
///
/// This approximates the lower bound of an inline status refresh; the owned-versus-inline release
/// decision still requires the real terminal A/B because this fixture does not execute the inline
/// history writer.
pub struct FooterOnlyFrameFixture {
    area: Rect,
    buffer: Buffer,
    status_tick: u64,
}

impl FooterOnlyFrameFixture {
    pub fn new(width: u16, height: u16) -> Self {
        let footer_height = 4_u16.min(height);
        let area = Rect::new(/*x*/ 0, /*y*/ 0, width, footer_height);
        Self {
            area,
            buffer: Buffer::empty(area),
            status_tick: 0,
        }
    }

    pub fn render_status_frame(&mut self) -> u64 {
        self.status_tick = self.status_tick.wrapping_add(1);
        self.buffer.reset();
        let footer = Line::from(vec![
            "esc".bold(),
            " interrupt · Working ".dim(),
            format!("{}s", self.status_tick).dim(),
        ]);
        Paragraph::new(footer).render(self.area, &mut self.buffer);

        let bottom = self.buffer.area.bottom().saturating_sub(1);
        let right = self.buffer.area.right().saturating_sub(1);
        self.status_tick
            ^ u64::from(
                self.buffer[(self.buffer.area.x, self.buffer.area.y)]
                    .symbol()
                    .len() as u32,
            )
            ^ u64::from(self.buffer[(right, bottom)].symbol().len() as u32)
    }
}

impl OwnedFrameFixture {
    pub fn new(width: u16, height: u16) -> Self {
        let cells = (0..4_096)
            .map(|index| {
                let marker = match index % 4 {
                    0 => "read source",
                    1 => "searched symbols",
                    2 => "ran focused tests",
                    _ => "updated projection",
                };
                Arc::new(BenchCell {
                    text: format!("  {marker} {index:04} · cache-stable retained row"),
                }) as Arc<dyn HistoryCell>
            })
            .collect();
        let footer_height = 4_u16.min(height);
        let conversation_area = Rect::new(
            /*x*/ 0,
            /*y*/ 0,
            width,
            height.saturating_sub(footer_height),
        );
        let footer_area = Rect::new(/*x*/ 0, conversation_area.height, width, footer_height);
        let mut fixture = Self {
            viewport: ConversationViewport::new(
                cells,
                HistoryRenderMode::Rich,
                RuntimeKeymap::defaults().pager,
            ),
            conversation_area,
            footer_area,
            buffer: Buffer::empty(Rect::new(0, 0, width, height)),
            status_tick: 0,
            input_down: false,
        };
        for _ in 0..3 {
            let _ = fixture.render_status_frame();
        }
        fixture
    }

    /// Render one unchanged transcript frame plus a changing one-line status footer.
    pub fn render_status_frame(&mut self) -> u64 {
        self.status_tick = self.status_tick.wrapping_add(1);
        self.buffer.reset();
        self.viewport
            .render(self.conversation_area, &mut self.buffer);
        let footer = Line::from(vec![
            "esc".bold(),
            " interrupt · Working ".dim(),
            format!("{}s", self.status_tick).dim(),
        ]);
        Paragraph::new(footer).render(self.footer_area, &mut self.buffer);

        let bottom = self.buffer.area.bottom().saturating_sub(1);
        let right = self.buffer.area.right().saturating_sub(1);
        self.status_tick
            ^ u64::from(
                self.buffer[(self.buffer.area.x, self.buffer.area.y)]
                    .symbol()
                    .len() as u32,
            )
            ^ u64::from(self.buffer[(right, bottom)].symbol().len() as u32)
    }

    /// Handle one real transcript navigation event, then draw the running-status frame.
    pub fn handle_input_and_render(&mut self) -> u64 {
        let key = if self.input_down {
            KeyCode::PageDown
        } else {
            KeyCode::PageUp
        };
        self.input_down = !self.input_down;
        let _ = self.viewport.handle_navigation_key(
            self.conversation_area,
            KeyEvent::new(key, KeyModifiers::NONE),
        );
        self.render_status_frame()
    }

    /// Return process-global prepared-line counters for before/after delta measurements.
    pub fn prepared_cache_counts(&self) -> (u64, u64) {
        (
            quiet_metrics::get_for_bench(QuietMetric::PreparedCellCacheHit),
            quiet_metrics::get_for_bench(QuietMetric::PreparedCellCacheMiss),
        )
    }
}
