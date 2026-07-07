//! `web_search` and `web_fetch` tool implementations.
//!
//! Provider dispatch for `web_search`:
//!   - "brave"    → Brave Search REST API (GET, X-Subscription-Token header)
//!   - "exa"      → Exa AI JSON-RPC MCP endpoint (no key needed at basic tier)
//!   - "tavily"   → Tavily REST API (POST with api_key in body)
//!   - "parallel" → Parallel AI JSON-RPC MCP endpoint (no key needed)
//!
//! `web_fetch` is provider-independent: it fetches any URL and converts
//! HTML to readable text using the `html2text` crate.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::ToolContext;

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_NUM_RESULTS: u64 = 8;
const MAX_NUM_RESULTS: u64 = 20;
const SEARCH_TIMEOUT_SECS: u64 = 25;
const FETCH_TIMEOUT_SECS: u64 = 30;
const MAX_FETCH_BYTES: usize = 5 * 1024 * 1024; // 5 MB
const MAX_OUTPUT_CHARS: usize = 12_000;

// ── web_search ────────────────────────────────────────────────────────────────

pub async fn tool_web_search(args: &Value, ctx: &ToolContext) -> Result<String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("web_search requires 'query'"))?
        .trim()
        .to_string();
    if query.is_empty() {
        bail!("web_search: query cannot be empty");
    }

    let num_results = args
        .get("num_results")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_NUM_RESULTS)
        .min(MAX_NUM_RESULTS) as usize;

    let provider = ctx
        .mcp_active_provider
        .as_deref()
        .ok_or_else(|| anyhow!("web_search: no MCP provider configured (use /mcp to set one)"))?;

    let api_key = ctx.mcp_keys.get(provider).map(String::as_str);

    match provider {
        "brave" => search_brave(&query, num_results, api_key).await,
        "exa" => search_mcp_exa(&query, num_results, api_key).await,
        "tavily" => search_tavily(&query, num_results, api_key).await,
        "parallel" => search_mcp_parallel(&query, num_results, ctx).await,
        other => bail!("web_search: unknown provider '{other}'"),
    }
}

// ── Brave Search ──────────────────────────────────────────────────────────────

async fn search_brave(query: &str, num_results: usize, api_key: Option<&str>) -> Result<String> {
    let key = api_key.ok_or_else(|| {
        anyhow!("web_search (Brave): API key required — use /mcp to enter your Brave Search key")
    })?;

    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding(query),
        num_results
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", key)
        .send()
        .await
        .map_err(|e| anyhow!("Brave Search request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Brave Search returned {status}: {body}");
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("Brave Search: failed to parse response: {e}"))?;

    // Brave returns: { "web": { "results": [ { "title", "url", "description" } ] } }
    let results = json
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Brave Search: unexpected response shape"))?;

    format_results(
        results.iter().map(|r| SearchResult {
            title: r
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            url: r
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            snippet: r
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        "Brave Search",
        query,
    )
}

// ── Exa AI (MCP JSON-RPC) ────────────────────────────────────────────────────

async fn search_mcp_exa(query: &str, num_results: usize, api_key: Option<&str>) -> Result<String> {
    let url = match api_key {
        Some(k) => format!("https://mcp.exa.ai/mcp?exaApiKey={}", urlencoding(k)),
        None => "https://mcp.exa.ai/mcp".to_string(),
    };

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "type": "auto",
                "numResults": num_results,
                "livecrawl": "fallback"
            }
        }
    });

    let text = call_mcp(&url, payload, None).await?;
    Ok(truncate_output(&text, MAX_OUTPUT_CHARS))
}

// ── Tavily ────────────────────────────────────────────────────────────────────

async fn search_tavily(query: &str, num_results: usize, api_key: Option<&str>) -> Result<String> {
    let key = api_key.ok_or_else(|| {
        anyhow!("web_search (Tavily): API key required — use /mcp to enter your Tavily key")
    })?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .json(&json!({
            "api_key": key,
            "query": query,
            "max_results": num_results,
            "include_answer": false
        }))
        .send()
        .await
        .map_err(|e| anyhow!("Tavily request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Tavily returned {status}: {body}");
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("Tavily: failed to parse response: {e}"))?;

    // Tavily returns: { "results": [ { "title", "url", "content" } ] }
    let results = json
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Tavily: unexpected response shape"))?;

    format_results(
        results.iter().map(|r| SearchResult {
            title: r
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            url: r
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            snippet: r
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        "Tavily",
        query,
    )
}

// ── Parallel AI (MCP JSON-RPC) ───────────────────────────────────────────────

async fn search_mcp_parallel(
    query: &str,
    _num_results: usize,
    ctx: &ToolContext,
) -> Result<String> {
    let api_key = ctx.mcp_keys.get("parallel").map(String::as_str);
    let auth_header = api_key.map(|k| format!("Bearer {k}"));

    let payload = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search",
            "arguments": {
                "objective": query,
                "search_queries": [query]
            }
        }
    });

    let text = call_mcp(
        "https://search.parallel.ai/mcp",
        payload,
        auth_header.as_deref(),
    )
    .await?;
    Ok(truncate_output(&text, MAX_OUTPUT_CHARS))
}

// ── MCP JSON-RPC transport ────────────────────────────────────────────────────

/// Send a JSON-RPC `tools/call` to an MCP endpoint and return the first
/// `content[].text` from the response. Handles both direct JSON body and
/// Server-Sent Events (SSE) stream (`data: {...}` lines), matching
/// opencode's `parseResponse` logic exactly.
async fn call_mcp(url: &str, payload: Value, auth_header: Option<&str>) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .build()?;

    let mut req = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json");

    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }

    let resp = req
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow!("MCP request to {url} failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("MCP endpoint {url} returned {status}: {body}");
    }

    let body = resp
        .text()
        .await
        .map_err(|e| anyhow!("MCP: failed to read response body: {e}"))?;

    parse_mcp_response(&body)
        .ok_or_else(|| anyhow!("MCP: could not extract text content from response"))
}

/// Extract the first `result.content[].text` from an MCP response body.
/// Handles both direct JSON and SSE (`data: {…}` lines).
fn parse_mcp_response(body: &str) -> Option<String> {
    // Try direct JSON first.
    if let Some(text) = extract_mcp_text(body.trim()) {
        return Some(text);
    }
    // Try SSE: each relevant line starts with "data: ".
    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if let Some(text) = extract_mcp_text(data.trim()) {
                return Some(text);
            }
        }
    }
    None
}

fn extract_mcp_text(json_str: &str) -> Option<String> {
    if !json_str.starts_with('{') {
        return None;
    }
    let v: Value = serde_json::from_str(json_str).ok()?;
    v.get("result")?
        .get("content")?
        .as_array()?
        .iter()
        .find_map(|item| item.get("text")?.as_str().map(|s| s.to_string()))
}

// ── web_fetch ─────────────────────────────────────────────────────────────────

pub async fn tool_web_fetch(args: &Value, _ctx: &ToolContext) -> Result<String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("web_fetch requires 'url'"))?
        .trim()
        .to_string();

    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("web_fetch: URL must start with http:// or https://");
    }

    let format = args.get("format").and_then(Value::as_str).unwrap_or("text");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .get(&url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header("Accept", "text/html,application/xhtml+xml,*/*;q=0.9")
        .send()
        .await
        .map_err(|e| anyhow!("web_fetch: request to {url} failed: {e}"))?;

    if !resp.status().is_success() {
        bail!("web_fetch: {url} returned {}", resp.status());
    }

    // Guard size before buffering the full body.
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_FETCH_BYTES {
            bail!("web_fetch: response too large ({len} bytes, limit 5 MB)");
        }
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| anyhow!("web_fetch: failed to read body: {e}"))?;

    if bytes.len() > MAX_FETCH_BYTES {
        bail!(
            "web_fetch: response too large ({} bytes, limit 5 MB)",
            bytes.len()
        );
    }

    let content = String::from_utf8_lossy(&bytes).into_owned();

    let output = if content_type.contains("text/html") {
        match format {
            "markdown" => html_to_markdown(&content),
            _ => html_to_text(&content),
        }
    } else {
        content
    };

    Ok(truncate_output(&output, MAX_OUTPUT_CHARS))
}

// ── HTML conversion ───────────────────────────────────────────────────────────

fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 100)
}

fn html_to_markdown(html: &str) -> String {
    // html2text produces reasonable plain text; we prefix with the URL
    // as a simple Markdown reference rather than pulling in a full HTML→MD
    // crate (which would be a heavy dep for marginal gain).
    html2text::from_read(html.as_bytes(), 100)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

fn format_results(
    results: impl Iterator<Item = SearchResult>,
    source: &str,
    query: &str,
) -> Result<String> {
    let mut out = format!("Search results for \"{query}\" (via {source}):\n\n");
    let mut count = 0;

    for (i, r) in results.enumerate() {
        if r.url.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "{}. {}\n   {}\n   {}\n\n",
            i + 1,
            if r.title.is_empty() {
                r.url.clone()
            } else {
                r.title
            },
            r.url,
            r.snippet.chars().take(300).collect::<String>(),
        ));
        count += 1;
    }

    if count == 0 {
        out.push_str("No results found. Try a different query.");
    }

    Ok(truncate_output(&out, MAX_OUTPUT_CHARS))
}

/// Percent-encode a query string for URL embedding.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Trim the output to at most `max_chars` Unicode scalar values, appending
/// a truncation notice when content is cut.
fn truncate_output(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{cut}\n\n[output truncated — {max_chars} char limit reached]")
}
