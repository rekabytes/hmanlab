//! Public catalog of MCP web-search providers.
//!
//! Kept in a separate module (rather than inside `commands/mcp.rs`) so
//! that both the UI popup renderer (`ui::popups::mcp`) and the keyboard
//! handler (`app::input::modals`) can reach it without fighting Rust's
//! module-privacy rules.

/// Metadata for one MCP-compatible web search provider.
pub struct McpProvider {
    /// Internal id used as the key in config and mcp_keys map.
    pub id: &'static str,
    /// Display label shown in the picker and confirmation screens.
    pub label: &'static str,
    /// URL shown on the key-input screen so the user knows where to sign up.
    pub signup_url: &'static str,
    /// One-line free-tier description shown below the signup URL.
    pub free_tier: &'static str,
    /// Whether this provider requires an API key. Parallel AI does not.
    pub needs_key: bool,
    /// HTTP endpoint used by `tools::websearch`.
    /// Kept for future use / external introspection.
    #[allow(dead_code)]
    pub endpoint: &'static str,
    /// Tool name sent in the MCP `tools/call` params.name field.
    /// Kept for future use / external introspection.
    #[allow(dead_code)]
    pub mcp_tool_name: &'static str,
}

/// All supported web-search backends, in display order.
pub const MCP_PROVIDERS: &[McpProvider] = &[
    McpProvider {
        id: "brave",
        label: "Brave Search",
        signup_url: "https://api.search.brave.com",
        free_tier: "2,000 queries / month free",
        needs_key: true,
        endpoint: "https://api.search.brave.com/res/v1/web/search",
        mcp_tool_name: "brave_web_search",
    },
    McpProvider {
        id: "exa",
        label: "Exa AI",
        signup_url: "https://exa.ai",
        free_tier: "Basic tier free (no key required)",
        needs_key: false,
        endpoint: "https://mcp.exa.ai/mcp",
        mcp_tool_name: "web_search_exa",
    },
    McpProvider {
        id: "tavily",
        label: "Tavily",
        signup_url: "https://app.tavily.com",
        free_tier: "1,000 searches / month free",
        needs_key: true,
        endpoint: "https://api.tavily.com/search",
        mcp_tool_name: "tavily_search",
    },
    McpProvider {
        id: "parallel",
        label: "Parallel AI",
        signup_url: "https://parallel.ai",
        free_tier: "No key required",
        needs_key: false,
        endpoint: "https://search.parallel.ai/mcp",
        mcp_tool_name: "web_search",
    },
];

/// Return the `McpProvider` entry for a given id, or `None`.
pub fn provider_by_id(id: &str) -> Option<&'static McpProvider> {
    MCP_PROVIDERS.iter().find(|p| p.id == id)
}
