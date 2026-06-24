//! Inline image state for the message view.
//!
//! Holds a terminal-graphics [`Picker`] (kitty / iTerm2 / sixel, falling back to
//! unicode half-blocks) plus a per-URL cache of decoded image protocols. Images
//! are fetched asynchronously; bytes are handed to [`ImageStore::load`] as they
//! arrive and rendered on the next frame.

use std::collections::HashMap;

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

/// Upper bound on an inline image's on-screen footprint, in terminal cells.
const MAX_COLS: u16 = 48;
const MAX_ROWS: u16 = 16;

/// Per-image lifecycle.
pub enum ImageState {
    /// A fetch has been queued / is in flight.
    Loading,
    /// The fetch or decode failed; the renderer shows a chip instead.
    Failed,
    /// Decoded and ready to paint, with its chosen cell footprint. The protocol
    /// is boxed because it's far larger than the other variants.
    Ready {
        proto: Box<StatefulProtocol>,
        cols: u16,
        rows: u16,
    },
}

/// A Picker plus the decoded-image cache, keyed by object URL.
pub struct ImageStore {
    picker: Picker,
    images: HashMap<String, ImageState>,
    pending: Vec<String>,
}

impl ImageStore {
    /// Probe the terminal for a graphics protocol, falling back to half-blocks
    /// (which render anywhere). Must run while the terminal is in raw mode but
    /// before the input reader starts consuming stdin.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)));
        Self {
            picker,
            images: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// State for `url`, queuing a fetch the first time it's requested.
    pub fn state(&mut self, url: &str) -> &ImageState {
        if !self.images.contains_key(url) {
            self.images.insert(url.to_string(), ImageState::Loading);
            self.pending.push(url.to_string());
        }
        &self.images[url]
    }

    /// Mutable access for rendering (the protocol resizes/encodes in place).
    pub fn get_mut(&mut self, url: &str) -> Option<&mut ImageState> {
        self.images.get_mut(url)
    }

    /// URLs awaiting a fetch, cleared on read.
    pub fn take_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }

    /// Decode fetched bytes into a render-ready protocol sized for the terminal.
    pub fn load(&mut self, url: &str, bytes: &[u8]) {
        let state = match image::load_from_memory(bytes) {
            Ok(img) => {
                let (cols, rows) = self.cell_size(img.width(), img.height());
                ImageState::Ready {
                    proto: Box::new(self.picker.new_resize_protocol(img)),
                    cols,
                    rows,
                }
            }
            Err(_) => ImageState::Failed,
        };
        self.images.insert(url.to_string(), state);
    }

    /// Mark a URL as failed so the renderer shows its chip.
    pub fn fail(&mut self, url: &str) {
        self.images.insert(url.to_string(), ImageState::Failed);
    }

    /// Choose a cell footprint preserving the image's aspect ratio within the
    /// max bounds, using the terminal's measured font cell size.
    fn cell_size(&self, w: u32, h: u32) -> (u16, u16) {
        let (fw, fh) = self.picker.font_size();
        let (fw, fh) = (fw.max(1) as u32, fh.max(1) as u32);
        let cols = (w / fw).clamp(1, MAX_COLS as u32);
        let px_w = cols * fw;
        let px_h = px_w.saturating_mul(h).checked_div(w).unwrap_or(0);
        let rows = px_h.div_ceil(fh).clamp(1, MAX_ROWS as u32);
        (cols as u16, rows as u16)
    }
}

#[cfg(test)]
impl ImageStore {
    /// A store forced to the half-blocks protocol — renders into a `TestBackend`
    /// buffer without needing a real graphics terminal.
    pub fn halfblocks_for_test() -> Self {
        use ratatui_image::picker::ProtocolType;
        let mut picker = Picker::from_fontsize((8, 16));
        picker.set_protocol_type(ProtocolType::Halfblocks);
        Self {
            picker,
            images: HashMap::new(),
            pending: Vec::new(),
        }
    }
}
