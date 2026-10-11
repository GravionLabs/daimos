//! The macOS clipboard: `NSPasteboard` has no change notification, so `changeCount` is polled
//! every 250 ms. Content marked `org.nspasteboard.ConcealedType` (password managers) is concealed.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use daimos_core::clipboard::{
    ClipboardBackend, ClipboardContent, ClipboardSink, ClipboardSnapshot, ClipboardSupport,
};
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::NSString;

const POLL: Duration = Duration::from_millis(250);
const CONCEALED: &str = "org.nspasteboard.ConcealedType";

pub struct MacClipboard;

impl ClipboardBackend for MacClipboard {
    fn id(&self) -> &str {
        "macos"
    }

    fn support(&self) -> ClipboardSupport {
        ClipboardSupport::Full
    }

    fn watch(&self, mut sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()> {
        let mut last = NSPasteboard::generalPasteboard().changeCount();
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(POLL);
            let count = NSPasteboard::generalPasteboard().changeCount();
            if count == last {
                continue;
            }
            last = count;
            if let Some(snapshot) = autoreleasepool(|_| read()) {
                sink(snapshot);
            }
        }
        Ok(())
    }
}

fn read() -> Option<ClipboardSnapshot> {
    let board = NSPasteboard::generalPasteboard();
    let types: Vec<String> = board
        .types()
        .map(|types| types.iter().map(|t| t.to_string()).collect())
        .unwrap_or_default();
    let has = |wanted: &str| types.iter().any(|t| t == wanted);
    let concealed = has(CONCEALED);
    let content = if has("public.png") {
        let data = board.dataForType(&NSString::from_str("public.png"))?;
        ClipboardContent::Image(data.to_vec())
    } else if has("public.utf8-plain-text") {
        let text = board.stringForType(&NSString::from_str("public.utf8-plain-text"))?;
        ClipboardContent::Text(text.to_string())
    } else {
        return None;
    };
    Some(ClipboardSnapshot { content, concealed })
}
