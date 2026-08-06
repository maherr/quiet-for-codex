use std::path::Path;
use std::path::PathBuf;

use crate::conversation_selection::CellSelectionProjection;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::visible_lines;

pub(crate) enum ActiveCellSelectionHandle {
    Ready(Option<CellSelectionProjection>),
    AgentMarkdown {
        source: String,
        cwd: PathBuf,
        markdown_width: usize,
        display_width: u16,
    },
}

impl ActiveCellSelectionHandle {
    pub(crate) fn ready(projection: Option<CellSelectionProjection>) -> Self {
        Self::Ready(projection)
    }

    pub(crate) fn agent_markdown(source: &str, cwd: &Path, display_width: u16) -> Self {
        let Some(markdown_width) =
            crate::width::usable_content_width_u16(display_width, /*reserved_cols*/ 2)
        else {
            return Self::Ready(None);
        };
        Self::AgentMarkdown {
            source: crate::markdown::unwrap_markdown_fences(source).into_owned(),
            cwd: cwd.to_path_buf(),
            markdown_width,
            display_width,
        }
    }

    pub(crate) fn resolve(
        &self,
        display_lines: &[HyperlinkLine],
    ) -> Option<CellSelectionProjection> {
        match self {
            Self::Ready(projection) => projection.clone(),
            Self::AgentMarkdown {
                source,
                cwd,
                markdown_width,
                display_width,
            } => crate::markdown_render::render_markdown_selection_projection(
                source,
                *markdown_width,
                Some(cwd.as_path()),
                visible_lines(display_lines.to_vec()),
                *display_width,
                /*outer_prefix_columns*/ 2,
            ),
        }
    }
}
