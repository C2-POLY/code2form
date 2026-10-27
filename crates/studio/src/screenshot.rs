//! `CODE2FORM_SCREENSHOT=out.png`: saves a picture of the window once the
//! first render is on screen, for checking the UI from scripts.
//!
//! `CODE2FORM_SCREENSHOT_SELECT` selects something of a circuit first: a
//! pin as `part.pin`, a part by its name, or a wire as `wire:7` (from 1);
//! `assembly:7` shows the assembly mode at its seventh wire; and
//! `rewire:xiao.5V>mic.VDD`, a new wire dragged from one pin to another;
//! `catalogue` opens the parts catalogue; `move:mic:40,10` holds the part
//! `mic` mid-drag at x 40, y 10; `settings:model` shows a page of the
//! settings (`general`, `appearance`, `editor`, `model`, `export`,
//! `libraries`, `shortcuts`); `play` plays a design that reads `$t`, and
//! `exploded` draws its part()s apart (`play,exploded` does both;
//! `exploded,axis` along each part's axis); `agent` opens the chat over
//! the view alone, a short conversation in it whose edit lands and shows
//! as a diff card; `agent-empty` opens it empty, with its suggestions;
//! `agent-code` shows the code beside the view, the chat folded over it
//! and a turn under way, its spinner on the Agent button; `agent-edit`
//! shows an edit of the agent's landing (its line marked, its part
//! flashed), and `agent-review` the review of one before it applies;
//! `agent-markdown` a reply in Markdown, with a table, after a web search;
//! `agent-brief` a brief saved in a scratch folder, and its next step;
//! `start`, run without a file, the start screen of a new file.
//! `CODE2FORM_APPEARANCE` picks the theme, `CODE2FORM_LANGUAGE` the
//! language (`en` or `fr`). The run quits once the picture is saved.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

/// Lets the model reach the GPU and the layout settle.
const SETTLE: Duration = Duration::from_millis(1500);

pub struct Screenshot {
    path: PathBuf,
    /// What to select in a circuit, as `CODE2FORM_SCREENSHOT_SELECT` says.
    pub select: Option<String>,
    ready_since: Option<Instant>,
    requested: bool,
}

impl Screenshot {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("CODE2FORM_SCREENSHOT")?;
        let select = std::env::var("CODE2FORM_SCREENSHOT_SELECT").ok();
        Some(Screenshot { path: PathBuf::from(path), select, ready_since: None, requested: false })
    }

    /// Call every frame; `ready` once the first render is shown. Returns a
    /// message to log once the file is written.
    pub fn update(&mut self, ctx: &egui::Context, ready: bool) -> Option<Result<String, String>> {
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [width, height] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let png = encode_png(width as u32, height as u32, &rgba);
            let result = std::fs::write(&self.path, png)
                .map(|()| format!("Saved a screenshot to {}", self.path.display()))
                .map_err(|err| format!("Cannot save the screenshot {}: {err}", self.path.display()));
            return Some(result);
        }
        if self.requested {
            return None;
        }
        if !ready {
            self.ready_since = None;
            return None;
        }
        let since = *self.ready_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SETTLE {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        None
    }
}

/// An RGBA PNG with stored (uncompressed) deflate blocks.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks_exact(stride.max(1)) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(0xffff).collect();
    if blocks.is_empty() {
        zlib.extend([1, 0, 0, 0xff, 0xff]);
    }
    for (i, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]); // 8-bit RGBA
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn png_layout() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"));
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
    }
}
