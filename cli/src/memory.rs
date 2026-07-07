//! Persistent memory store — markdown files under `.hmanlab/memory/`.
//!
//! Two scopes:
//!   - **User** (`~/.hmanlab/memory/`): facts that span every project. The
//!     user's role/preferences, behaviour corrections that apply globally,
//!     references to external systems the user works with.
//!   - **Project** (`<workspace>/.hmanlab/memory/`): facts specific to the
//!     current codebase. Architecture decisions, in-flight work, project-only
//!     reference URLs.
//!
//! Each memory is one markdown file with YAML frontmatter (`name`,
//! `description`, `type`) plus a free-form body. A `MEMORY.md` index in each
//! scope lists the available memories and is what the system prompt loads
//! every turn — bodies are fetched on demand via the `read_memory` tool.

use anyhow::{anyhow, bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Which `.hmanlab/memory/` tree a memory belongs to. The user-side is
/// shared across projects; the project-side is workspace-local.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryScope {
    User,
    Project,
}

impl MemoryScope {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            other => bail!("unknown scope '{other}' — expected 'user' or 'project'"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// One memory loaded from disk. `body` is the post-frontmatter markdown.
#[derive(Clone, Debug)]
pub struct MemoryFile {
    pub name: String,
    pub description: String,
    pub kind: String,
    pub body: String,
}

/// Resolve the directory `<scope>/.hmanlab/memory/`. Returns `None` for the
/// User scope when `$HOME` isn't set — that case is treated as "no user
/// memories available" rather than an error so a project-only setup still works.
pub fn scope_dir(scope: MemoryScope, workspace: &Path) -> Option<PathBuf> {
    match scope {
        MemoryScope::User => {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".hmanlab/memory"))
        }
        MemoryScope::Project => Some(workspace.join(".hmanlab/memory")),
    }
}

/// Read the current `MEMORY.md` index for a scope. Returns `None` if the
/// scope dir doesn't exist (no memories saved yet) — the caller decides
/// whether to omit the section or render a placeholder.
pub fn load_index(scope: MemoryScope, workspace: &Path) -> Option<String> {
    let path = scope_dir(scope, workspace)?.join("MEMORY.md");
    fs::read_to_string(path).ok()
}

/// Walk a scope's directory and return every `*.md` file (except the index)
/// parsed into a `MemoryFile`. Used by `rebuild_index` and by the read tool.
pub fn list_memories(scope: MemoryScope, workspace: &Path) -> Result<Vec<MemoryFile>> {
    let Some(dir) = scope_dir(scope, workspace) else {
        return Ok(Vec::new());
    };
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some("MEMORY") {
            continue;
        }
        if let Some(mf) = parse_memory_file(&path) {
            out.push(mf);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn read_memory(scope: MemoryScope, name: &str, workspace: &Path) -> Result<MemoryFile> {
    let dir = scope_dir(scope, workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    let path = dir.join(format!("{}.md", sanitize_name(name)));
    parse_memory_file(&path)
        .ok_or_else(|| anyhow!("memory '{name}' not found in {} scope", scope.as_str()))
}

/// Write a memory file and rebuild the scope's index. Creates the scope
/// directory if it doesn't exist. `name` is sanitised to filesystem-safe chars.
pub fn save_memory(
    scope: MemoryScope,
    name: &str,
    kind: &str,
    description: &str,
    body: &str,
    workspace: &Path,
) -> Result<PathBuf> {
    let dir = scope_dir(scope, workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    fs::create_dir_all(&dir)?;
    let slug = sanitize_name(name);
    if slug.is_empty() {
        bail!("name must contain at least one alphanumeric character");
    }
    let path = dir.join(format!("{slug}.md"));
    let content = build_memory_file(&slug, kind, description, body);
    fs::write(&path, content)?;
    rebuild_index(scope, workspace)?;
    Ok(path)
}

pub fn forget_memory(scope: MemoryScope, name: &str, workspace: &Path) -> Result<PathBuf> {
    let dir = scope_dir(scope, workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    let path = dir.join(format!("{}.md", sanitize_name(name)));
    if !path.exists() {
        bail!("memory '{name}' not found in {} scope", scope.as_str());
    }
    fs::remove_file(&path)?;
    rebuild_index(scope, workspace)?;
    Ok(path)
}

/// Render the `MEMORY.md` index from the current set of memory files. Index
/// lines are intentionally short — they live in the system prompt every turn,
/// so the description column is the model's signal for whether to call
/// `read_memory` for the body.
fn rebuild_index(scope: MemoryScope, workspace: &Path) -> Result<()> {
    let dir = scope_dir(scope, workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    let memories = list_memories(scope, workspace)?;
    let mut out = String::from("# Memory Index\n\n");
    if memories.is_empty() {
        out.push_str("_No memories saved yet._\n");
    } else {
        for m in &memories {
            // `| File | Type | Description |` style would be tidier but adds
            // boilerplate tokens. One bullet per memory keeps the index dense.
            out.push_str(&format!(
                "- `{}` ({}) — {}\n",
                m.name, m.kind, m.description
            ));
        }
    }
    fs::write(dir.join("MEMORY.md"), out)?;
    Ok(())
}

/// Parse a memory file's YAML-ish frontmatter. We don't pull in a real YAML
/// crate — the format is fixed (`name:`, `description:`, `type:`) and a
/// 30-line hand-roll keeps the dependency footprint flat.
fn parse_memory_file(path: &Path) -> Option<MemoryFile> {
    let raw = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = split_frontmatter(&raw)?;
    let mut name = String::new();
    let mut description = String::new();
    let mut kind = String::from("user");
    for line in frontmatter.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("description:") {
            description = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("type:") {
            kind = rest.trim().to_string();
        }
    }
    if name.is_empty() {
        name = path.file_stem()?.to_str()?.to_string();
    }
    Some(MemoryFile {
        name,
        description,
        kind,
        body: body.to_string(),
    })
}

fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let s = raw.trim_start_matches('\u{FEFF}'); // strip BOM if any
    let s = s.trim_start_matches(['\n', '\r', ' ']);
    let s = s.strip_prefix("---")?;
    let s = s.trim_start_matches(['\n', '\r']);
    let end = s.find("\n---")?;
    let fm = &s[..end];
    let body = &s[end + "\n---".len()..];
    let body = body.trim_start_matches(['\n', '\r']);
    Some((fm, body))
}

pub fn build_memory_file(name: &str, kind: &str, description: &str, body: &str) -> String {
    let k = validate_kind(kind);
    format!(
        "---\nname: {}\ndescription: {}\ntype: {}\n---\n\n{}\n",
        name,
        description.trim(),
        k,
        body.trim_end()
    )
}

/// Restrict slugs to filesystem-safe ASCII so the AI can't write outside the
/// memory dir via path-traversal arguments. Anything else is dropped.
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .collect()
}

fn validate_kind(kind: &str) -> &str {
    match kind {
        "user" | "project" | "feedback" | "reference" => kind,
        _ => "user",
    }
}
