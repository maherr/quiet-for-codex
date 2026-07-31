//! Compact history cell for replayed generic tool calls.

use super::*;
use codex_app_server_protocol::DynamicToolCallStatus;

#[derive(Debug)]
pub(crate) struct DynamicToolCallCell {
    namespace: Option<String>,
    tool: String,
    arguments: serde_json::Value,
    status: DynamicToolCallStatus,
}

impl DynamicToolCallCell {
    pub(crate) fn new(
        namespace: Option<String>,
        tool: String,
        arguments: serde_json::Value,
        status: DynamicToolCallStatus,
    ) -> Self {
        Self {
            namespace,
            tool,
            arguments,
            status,
        }
    }

    pub(crate) fn completed_label(&self) -> Option<String> {
        (self.status == DynamicToolCallStatus::Completed).then(|| self.label())
    }

    fn label(&self) -> String {
        self.namespace
            .as_ref()
            .map(|namespace| format!("{namespace}.{}", self.tool))
            .unwrap_or_else(|| self.tool.clone())
    }

    fn command_detail(&self) -> Option<&str> {
        (self.tool == "exec_command")
            .then(|| {
                self.arguments
                    .get("cmd")
                    .and_then(serde_json::Value::as_str)
            })
            .flatten()
    }

    fn text(&self) -> String {
        let label = self.label();
        let mut text = match self.status {
            DynamicToolCallStatus::InProgress => format!("Tool call unfinished: {label}"),
            DynamicToolCallStatus::Completed => format!("Called {label}"),
            DynamicToolCallStatus::Failed => format!("Tool call failed: {label}"),
        };
        if let Some(command) = self.command_detail() {
            text.push_str(": ");
            text.push_str(command);
        }
        text
    }
}

impl HistoryCell for DynamicToolCallCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let text = Text::from(self.text());
        PrefixedWrappedHistoryCell::new(text, vec!["•".dim(), " ".into()], "  ")
            .display_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.text())
    }

    fn selection_contribution(&self, width: u16, mode: HistoryRenderMode) -> SelectionContribution {
        selection_contribution_from_display_lines(self.display_lines_for_mode(width, mode), width)
    }
}

pub(crate) fn new_dynamic_tool_call(
    namespace: Option<String>,
    tool: String,
    arguments: serde_json::Value,
    status: DynamicToolCallStatus,
) -> DynamicToolCallCell {
    DynamicToolCallCell::new(namespace, tool, arguments, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_mode_splits_multiline_exec_commands() {
        let cell = new_dynamic_tool_call(
            None,
            "exec_command".to_string(),
            serde_json::json!({"cmd": "printf one\nprintf two"}),
            DynamicToolCallStatus::Completed,
        );

        let lines = cell.raw_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].to_string(), "Called exec_command: printf one");
        assert_eq!(lines[1].to_string(), "printf two");
    }
}
