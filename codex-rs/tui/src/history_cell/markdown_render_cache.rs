//! Small render cache shared by finalized source-backed markdown history cells.

use crate::terminal_hyperlinks::HyperlinkLine;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

#[derive(Debug, Default)]
pub(super) struct MarkdownRenderCache {
    pub(super) cached: Mutex<VecDeque<(MarkdownRenderCacheKey, Arc<[HyperlinkLine]>)>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MarkdownRenderCacheKey {
    pub(super) width: u16,
    pub(super) syntax_theme_revision: u64,
    pub(super) terminal_fg: Option<(u8, u8, u8)>,
    pub(super) terminal_bg: Option<(u8, u8, u8)>,
    pub(super) color_level: crate::terminal_palette::StdoutColorLevel,
}

impl MarkdownRenderCache {
    /// Return lines cached for this width and terminal render state, rendering on a cache miss.
    ///
    /// The two most recent entries are retained so a one-column terminal oscillation or side-pane
    /// layout toggle does not repeatedly render the same markdown. Shared ownership keeps steady
    /// viewport draws from cloning every line and span in a cached message.
    pub(super) fn render(
        &self,
        width: u16,
        render: impl FnOnce() -> Vec<HyperlinkLine>,
    ) -> Arc<[HyperlinkLine]> {
        let key = MarkdownRenderCacheKey {
            width,
            syntax_theme_revision: crate::render::highlight::syntax_theme_revision(),
            terminal_fg: crate::terminal_palette::default_fg(),
            terminal_bg: crate::terminal_palette::default_bg(),
            color_level: crate::terminal_palette::stdout_color_level(),
        };
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = cached
            .iter()
            .position(|(cached_key, _lines)| *cached_key == key)
        {
            let entry = cached.remove(index).expect("cached markdown entry");
            let lines = Arc::clone(&entry.1);
            cached.push_back(entry);
            return lines;
        }

        let lines = Arc::<[HyperlinkLine]>::from(render());
        cached.push_back((key, Arc::clone(&lines)));
        while cached.len() > 2 {
            cached.pop_front();
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    #[test]
    fn retains_two_widths_without_rerendering_the_first() {
        let cache = MarkdownRenderCache::default();
        let renders = AtomicUsize::new(0);
        let render = || {
            renders.fetch_add(1, Ordering::Relaxed);
            vec![HyperlinkLine::from("rendered")]
        };

        let first = cache.render(/*width*/ 80, render);
        let _second = cache.render(/*width*/ 79, render);
        let first_again = cache.render(/*width*/ 80, render);

        assert_eq!(renders.load(Ordering::Relaxed), 2);
        assert!(Arc::ptr_eq(&first, &first_again));
    }
}
