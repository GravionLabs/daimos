//! Clipboard history: what a platform backend reports, the SQLite table that keeps it, and the
//! service that connects the two.
//!
//! The backends live in `daimos-platform` (one per OS and display server); this crate only sees the
//! [`ClipboardBackend`] trait, so the history runs headless and is tested with a mock.

mod capture;
mod history;

use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub use capture::{Capture, CaptureError};
pub use history::{ClipboardHistory, HistoryError, Limits};

/// What was copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Text(String),
    /// An encoded image (PNG).
    Image(Vec<u8>),
}

impl ClipboardContent {
    /// The size of the payload in bytes.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Image(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// One change of the clipboard as a backend sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub content: ClipboardContent,
    /// A password manager marked the content (`x-kde-passwordManagerHint`,
    /// `org.nspasteboard.ConcealedType`); it is never stored.
    pub concealed: bool,
}

/// How much of the clipboard a backend can follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardSupport {
    /// Every change is reported, whichever window has the focus.
    Full,
    /// Changes are reported only under a condition, for example while the Daimos window is focused
    /// (GNOME on Wayland has no `wlr-data-control`).
    Limited(String),
    Unsupported(String),
}

/// Called by a backend for every change of the clipboard.
pub type ClipboardSink = Box<dyn FnMut(ClipboardSnapshot) + Send>;

pub trait ClipboardBackend: Send + Sync {
    /// `x11`, `wlr-data-control`, `windows`, `macos`, ...
    fn id(&self) -> &str;

    fn support(&self) -> ClipboardSupport;

    /// Reports every change to `sink` until `stop` is set, then returns. It blocks, so the caller
    /// runs it on a thread of its own. The content that is on the clipboard when the call starts
    /// is not reported.
    fn watch(&self, sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()>;
}
