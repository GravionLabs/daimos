//! The macOS clipboard backend, with pbcopy as the source.
#![cfg(target_os = "macos")]

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daimos_core::clipboard::{ClipboardBackend, ClipboardContent};
use daimos_platform::clipboard::macos::MacClipboard;

#[test]
fn copied_text_is_reported_but_the_existing_content_is_not() {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            MacClipboard.watch(Box::new(move |s| drop(tx.lock().unwrap().send(s))), stop).unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    let mut child = Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"daimos clipboard test").unwrap();
    assert!(child.wait().unwrap().success());
    let snapshot = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(snapshot.content, ClipboardContent::Text("daimos clipboard test".into()));
    assert!(!snapshot.concealed);
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
}
