use crate::render::renderable::Renderable;

/// Cached vertical layout for a retained list of pager renderables.
///
/// Stable renderables are measured once per width. Unknown or explicitly unstable renderables are
/// remeasured on each layout pass, while their stable neighbors reuse cached heights. Prefix
/// offsets make total-height and visible-range queries constant or logarithmic time.
#[derive(Default)]
pub(crate) struct PagerLayoutCache {
    width: Option<u16>,
    heights: Vec<u16>,
    starts: Vec<usize>,
    unstable_indices: Vec<usize>,
}

impl PagerLayoutCache {
    pub(crate) fn clear(&mut self) {
        self.width = None;
        self.heights.clear();
        self.starts.clear();
        self.unstable_indices.clear();
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        self.heights.truncate(len);
        self.starts.truncate(len.saturating_add(1));
        self.unstable_indices.retain(|index| *index < len);
    }

    pub(crate) fn invalidate_from(&mut self, index: usize) {
        if self.width.is_some() {
            self.truncate(index);
        }
    }

    pub(crate) fn ensure(&mut self, renderables: &[Box<dyn Renderable>], width: u16) {
        if self.width != Some(width) {
            self.rebuild(renderables, width);
            return;
        }

        let retained_len = self.heights.len().min(renderables.len());
        if retained_len < self.heights.len() {
            self.truncate(retained_len);
        }

        let mut rebuild_from = None;
        for &index in &self.unstable_indices {
            let height = renderables[index].desired_height(width);
            if self.heights[index] != height {
                self.heights[index] = height;
                rebuild_from = Some(rebuild_from.map_or(index, |first: usize| first.min(index)));
            }
        }

        for (index, renderable) in renderables.iter().enumerate().skip(retained_len) {
            self.heights.push(renderable.desired_height(width));
            let is_stable = renderable.has_stable_height();
            if !is_stable {
                self.unstable_indices.push(index);
            }
        }

        if let Some(first_changed) = rebuild_from {
            self.rebuild_starts_from(first_changed);
        } else {
            let mut top = self.starts.last().copied().unwrap_or(0);
            for height in &self.heights[retained_len..] {
                top = top.saturating_add(*height as usize);
                self.starts.push(top);
            }
        }
    }

    pub(crate) fn total_height(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    pub(crate) fn heights(&self) -> &[u16] {
        &self.heights
    }

    pub(crate) fn start(&self, index: usize) -> usize {
        self.starts.get(index).copied().unwrap_or(0)
    }

    pub(crate) fn end(&self, index: usize) -> usize {
        self.starts
            .get(index.saturating_add(1))
            .copied()
            .unwrap_or_else(|| self.total_height())
    }

    /// Returns the first item whose bottom is at or below `minimum_end`.
    pub(crate) fn first_with_end_at_least(&self, minimum_end: usize) -> usize {
        self.starts[1..].partition_point(|end| *end < minimum_end)
    }

    fn rebuild(&mut self, renderables: &[Box<dyn Renderable>], width: u16) {
        self.width = Some(width);
        self.heights.clear();
        self.starts.clear();
        self.unstable_indices.clear();
        self.starts.push(0);

        for (index, renderable) in renderables.iter().enumerate() {
            let height = renderable.desired_height(width);
            self.heights.push(height);
            let is_stable = renderable.has_stable_height();
            if !is_stable {
                self.unstable_indices.push(index);
            }
        }
        self.rebuild_starts_from(0);
    }

    fn rebuild_starts_from(&mut self, index: usize) {
        self.starts.truncate(index.saturating_add(1));
        let mut top = self.starts.last().copied().unwrap_or(0);
        for height in &self.heights[index..] {
            top = top.saturating_add(*height as usize);
            self.starts.push(top);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    struct FixedHeight(u16);

    impl Renderable for FixedHeight {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn desired_height(&self, _width: u16) -> u16 {
            self.0
        }

        fn has_stable_height(&self) -> bool {
            true
        }
    }

    #[test]
    fn first_visible_boundary_is_strict_and_handles_zero_height_items() {
        let renderables = vec![
            Box::new(FixedHeight(0)) as Box<dyn Renderable>,
            Box::new(FixedHeight(1)),
            Box::new(FixedHeight(0)),
            Box::new(FixedHeight(2)),
        ];
        let mut layout = PagerLayoutCache::default();
        layout.ensure(&renderables, /*width*/ 80);

        assert_eq!(layout.heights(), &[0, 1, 0, 2]);
        assert_eq!(layout.start(3), 1);
        assert_eq!(layout.end(3), 3);
        assert_eq!(layout.first_with_end_at_least(0), 0);
        assert_eq!(layout.first_with_end_at_least(1), 1);
        assert_eq!(layout.first_with_end_at_least(2), 3);
        assert_eq!(layout.first_with_end_at_least(4), 4);
    }
}
