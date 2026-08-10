use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolGroupItem {
    pub(crate) summary: ToolGroupSummary,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolGroupSummary {
    pub(crate) read_labels: BTreeSet<String>,
    pub(crate) list_count: usize,
    pub(crate) search_count: usize,
    pub(crate) edit_file_count: usize,
    pub(crate) run_count: usize,
    pub(crate) test_count: usize,
    pub(crate) build_count: usize,
    pub(crate) check_count: usize,
    pub(crate) install_count: usize,
    pub(crate) web_search_count: usize,
    pub(crate) web_open_count: usize,
    pub(crate) web_find_count: usize,
    pub(crate) mcp_counts: BTreeMap<String, usize>,
    pub(crate) dynamic_tool_counts: BTreeMap<String, usize>,
    pub(crate) actions: usize,
}
