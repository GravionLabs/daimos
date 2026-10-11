//! The wlr-data-control backend in a headless sway session, with `wl-copy` as the source. The
//! tests do nothing without `wl-copy` or a compositor that has the protocol.
#![cfg(target_os = "linux")]

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daimos_core::clipboard::{ClipboardBackend, ClipboardContent, ClipboardSupport};
use daimos_platform::clipboard::wayland::WaylandClipboard;

fn available() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && Command::new("wl-copy").arg("--version").output().is_ok_and(|o| o.status.success())
        && WaylandClipboard.support() == ClipboardSupport::Full
}

fn copy(mime: &str, data: &[u8]) {
    let mut child = Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(data).unwrap();
    child.wait().unwrap();
}

#[test]
fn text_and_images_are_reported_in_order() {
    if !available() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            WaylandClipboard
                .watch(Box::new(move |s| drop(tx.lock().unwrap().send(s))), stop)
                .unwrap();
        })
    };
    std::thread::sleep(Duration::from_millis(500));

    copy("text/plain;charset=utf-8", "grüße".as_bytes());
    let text = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(text.content, ClipboardContent::Text("grüße".into()));
    assert!(!text.concealed);

    copy("image/png", &[0x89, b'P', b'N', b'G']);
    let image = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(image.content, ClipboardContent::Image(vec![0x89, b'P', b'N', b'G']));

    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
}
