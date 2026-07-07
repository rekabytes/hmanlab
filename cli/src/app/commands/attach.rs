//! `/attach <path>` and `/detach [name|all]` — manage image / media
//! attachments queued for the next user message. Also hosts the
//! Ctrl+V / `/paste` clipboard handler: tries an image first, falls
//! back to text into the input.
//!
//! Attachments live in `App.pending_attachments` between commands. The
//! actual wire encoding (base64 data URL → `image_url` content part)
//! happens in `ollama::ChatMessage::to_api_content` once `start_turn`
//! drains the queue onto the user message.

use std::path::PathBuf;

use crate::ollama::Attachment;

use super::super::App;

/// Hard cap per attachment. Above this, providers either reject the
/// request outright or drop the image silently — better to fail fast
/// with a clear error than ship a payload that gets stripped server-
/// side. 20 MB matches OpenAI / Anthropic's documented per-image limit.
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Max attachments queued at once. Token budget on the next turn
/// degrades fast past 4-5 images on most multimodal models; cap here
/// so the user gets feedback instead of a context-overflow error.
const MAX_PENDING: usize = 8;

impl App {
    pub(in crate::app) fn handle_attach(&mut self, raw: String) {
        let raw = raw.trim();
        if raw.is_empty() {
            self.push_info(
                "Usage: /attach <path>   — queue an image or file for the next message.".into(),
            );
            return;
        }

        if self.pending_attachments.len() >= MAX_PENDING {
            self.push_info(format!(
                "Already at the {MAX_PENDING}-attachment cap. Send the current message or /detach all to reset."
            ));
            return;
        }

        let path = resolve_path(raw, &self.workspace);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                self.push_info(format!("/attach: can't read {} — {e}", path.display()));
                return;
            }
        };
        if !meta.is_file() {
            self.push_info(format!("/attach: {} is not a file.", path.display()));
            return;
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            self.push_info(format!(
                "/attach: {} is {:.1} MB — over the 20 MB per-attachment cap.",
                path.display(),
                meta.len() as f64 / (1024.0 * 1024.0)
            ));
            return;
        }

        match Attachment::from_path(&path) {
            Ok(att) => {
                let summary = format!(
                    "✓ Attached {} ({}, {}). Send your next message to include it.",
                    att.filename,
                    att.media_type,
                    att.size_display()
                );
                self.pending_attachments.push(att);
                self.push_info(summary);
            }
            Err(e) => {
                self.push_info(format!("/attach: {e}"));
            }
        }
    }

    pub(in crate::app) fn handle_detach(&mut self, arg: String) {
        let arg = arg.trim();
        if self.pending_attachments.is_empty() {
            self.push_info("Nothing to detach — no attachments queued.".into());
            return;
        }
        if arg.is_empty() {
            let dropped = self.pending_attachments.pop().expect("non-empty");
            self.push_info(format!("✓ Detached {} (most recent).", dropped.filename));
            return;
        }
        if arg.eq_ignore_ascii_case("all") || arg == "*" {
            let n = self.pending_attachments.len();
            self.pending_attachments.clear();
            self.push_info(format!("✓ Cleared {n} queued attachment(s)."));
            return;
        }
        let lo = arg.to_lowercase();
        let pos = self
            .pending_attachments
            .iter()
            .position(|a| a.filename.to_lowercase().contains(&lo));
        match pos {
            Some(i) => {
                let dropped = self.pending_attachments.remove(i);
                self.push_info(format!("✓ Detached {}.", dropped.filename));
            }
            None => {
                self.push_info(format!(
                    "No queued attachment matches '{arg}'. Use /detach all to clear everything."
                ));
            }
        }
    }
}

/// Resolve an attach target: `~`-expansion, then absolute as-is, else
/// relative to the workspace. We do NOT canonicalize — symlinks should
/// be followed by `std::fs::read` later, and pre-canonicalizing here
/// would hide the user's original path in the chat log.
fn resolve_path(raw: &str, workspace: &std::path::Path) -> PathBuf {
    let expanded: PathBuf = if let Some(rest) = raw.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(raw),
        }
    } else if raw == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(raw))
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        workspace.join(expanded)
    }
}

impl App {
    /// `Ctrl+V` / `/paste` — try the system clipboard for an image first;
    /// if there's no image (or the platform refused), fall back to
    /// inserting clipboard text into the textarea. Mirrors the way
    /// chat / image apps overload paste so users don't have to learn a
    /// second key for "paste an image" vs "paste text".
    ///
    /// Quiet on the common case (text paste prints nothing — the chars
    /// just appear), loud on the interesting one (image attached → info
    /// row in chat). When the platform clipboard is unavailable
    /// (headless SSH, no display), we surface that once so the user
    /// understands why nothing happened.
    pub(in crate::app) fn paste_from_clipboard(&mut self) {
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                self.push_info(format!(
                    "Clipboard unavailable: {e}. (Headless SSH / no graphical session?)"
                ));
                return;
            }
        };

        // Image first — that's the interesting path. arboard returns
        // RGBA8; we re-encode to PNG so we ship the same media_type as
        // /attach paths and the data URL stays predictable.
        match cb.get_image() {
            Ok(img) if img.width > 0 && img.height > 0 => {
                match encode_rgba_to_png(&img.bytes, img.width as u32, img.height as u32) {
                    Ok(png_bytes) => {
                        self.queue_clipboard_image(png_bytes);
                        return;
                    }
                    Err(e) => {
                        self.push_info(format!("Couldn't encode clipboard image: {e}"));
                        return;
                    }
                }
            }
            Ok(_) => {
                // Zero-sized image — treat as "no image", fall through to text.
            }
            Err(_) => {
                // No image on clipboard (or platform errored). Fall through to text.
            }
        }

        // No image — try text. Insert at cursor, same as a typed paste.
        // tui-textarea splits on newlines for us via insert_str.
        match cb.get_text() {
            Ok(text) if !text.is_empty() => {
                self.input.insert_str(&text);
            }
            _ => {
                // Clipboard is empty or unreadable. Don't push_info —
                // users hitting paste on an empty clipboard expect
                // nothing to happen, not a chat-log entry.
            }
        }
    }

    /// Shared between Ctrl+V and `/paste`: enforce the same caps as
    /// `/attach` (count + size), synthesize a clipboard-timestamped
    /// filename, push the attachment, and confirm in chat.
    fn queue_clipboard_image(&mut self, png_bytes: Vec<u8>) {
        if self.pending_attachments.len() >= MAX_PENDING {
            self.push_info(format!(
                "Already at the {MAX_PENDING}-attachment cap — send the message or /detach all first."
            ));
            return;
        }
        if png_bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            self.push_info(format!(
                "Clipboard image is {:.1} MB — over the 20 MB cap. Crop and try again.",
                png_bytes.len() as f64 / (1024.0 * 1024.0)
            ));
            return;
        }
        let filename = format!("clipboard-{}.png", clipboard_timestamp());
        let size = png_bytes.len();
        let att = Attachment {
            media_type: "image/png".to_string(),
            data: png_bytes,
            filename: filename.clone(),
        };
        let size_display = att.size_display();
        self.pending_attachments.push(att);
        self.push_info(format!(
            "✓ Pasted clipboard image as {filename} ({size_display}, {size} bytes). Send your next message to include it."
        ));
    }
}

/// RGBA8 → PNG byte vector. The `png` crate writes through a generic
/// `Write` so we collect into a Vec instead of touching disk.
fn encode_rgba_to_png(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    // arboard documents `bytes.len() == width * height * 4` for RGBA8.
    let expected = (w as usize) * (h as usize) * 4;
    if bytes.len() != expected {
        return Err(format!(
            "unexpected clipboard image size: {} bytes for {}x{} (expected {})",
            bytes.len(),
            w,
            h,
            expected
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(bytes).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// `YYYYMMDD-HHMMSS` in local time, used to make clipboard paste
/// filenames sortable and unique enough within a session. Falls back
/// to a Unix-seconds string if the system clock is broken.
fn clipboard_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Naive local-time conversion via the C library would pull in `chrono`
    // for one call. Instead: just use the epoch seconds — uniqueness
    // matters more than human readability for an auto-generated
    // attachment name.
    format!("{secs}")
}
