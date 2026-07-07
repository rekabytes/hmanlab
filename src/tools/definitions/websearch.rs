//! `web_search` and `web_fetch` tool schemas.
//!
//! These are conditionally included in the tool surface (via
//! `definitions::tool_definitions_with`) only when an MCP search
//! provider is configured — so the model never sees tools it can't
//! actually call.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn websearch_tools() -> Vec<Tool> {
    vec![
        Tool::function(
            "web_search",
            "Search the web for up-to-date information. Use this when the user asks about \
             recent events, current data, or anything beyond your knowledge cutoff. Returns \
             a numbered list of results with title, URL, and a short excerpt. \
             Always cite URLs when you use information from search results.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query. Be specific and include relevant \
                                        keywords. For time-sensitive queries include the year."
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Number of results to return (default: 8, max: 20).",
                        "minimum": 1,
                        "maximum": 20
                    }
                },
                "required": ["query"]
            }),
        ),
        Tool::function(
            "web_fetch",
            "Fetch and read the contents of a URL. Use this to get the full text of a \
             webpage, article, or documentation page. Returns plain text extracted from \
             the HTML. Respects a 5 MB size limit and a 30 second timeout.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The full URL to fetch (must start with http:// or https://)."
                    },
                    "format": {
                        "type": "string",
                        "description": "Output format: 'text' (default) or 'markdown'.",
                        "enum": ["text", "markdown"]
                    }
                },
                "required": ["url"]
            }),
        ),
    ]
}
