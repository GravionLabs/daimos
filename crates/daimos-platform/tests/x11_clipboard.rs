//! The X11 clipboard backend against a second X client that owns the selection. Needs an X
//! server (`DISPLAY`) and no Wayland session; otherwise the tests do nothing.
#![cfg(target_os = "linux")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use daimos_core::clipboard::{ClipboardBackend, ClipboardContent, ClipboardSnapshot};
use daimos_platform::clipboard::x11::X11Clipboard;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

static SERIAL: Mutex<()> = Mutex::new(());
static KEEP_ALIVE: std::sync::OnceLock<Option<x11rb::rust_connection::RustConnection>> =
    std::sync::OnceLock::new();

/// The tests share one clipboard, so they run one at a time. A connection that stays open keeps
/// the X server from resetting (and dropping every client) when a test's last client leaves.
fn x_server() -> Option<std::sync::MutexGuard<'static, ()>> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let alive = KEEP_ALIVE.get_or_init(|| {
        // Never against a desktop: under a Wayland session DISPLAY is Xwayland, and the test
        // would replace the user's clipboard.
        std::env::var_os("WAYLAND_DISPLAY")
            .is_none()
            .then(|| std::env::var_os("DISPLAY"))
            .flatten()
            .and_then(|_| x11rb::connect(None).ok())
            .map(|(conn, _)| conn)
    });
    alive.is_some().then_some(guard)
}

fn atom(conn: &impl Connection, name: &str) -> Atom {
    conn.intern_atom(false, name.as_bytes()).unwrap().reply().unwrap().atom
}

/// A client that owns `CLIPBOARD` and answers requests with `targets`.
struct Owner {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Owner {
    fn offer(targets: Vec<(&'static str, Vec<u8>)>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let (conn, screen) = x11rb::connect(None).unwrap();
                let root = conn.setup().roots[screen].root;
                let window = conn.generate_id().unwrap();
                conn.create_window(
                    COPY_DEPTH_FROM_PARENT,
                    window,
                    root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_ONLY,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::NO_EVENT),
                )
                .unwrap();
                let clipboard = atom(&conn, "CLIPBOARD");
                let targets_atom = atom(&conn, "TARGETS");
                let offered: Vec<(Atom, Vec<u8>)> =
                    targets.into_iter().map(|(name, data)| (atom(&conn, name), data)).collect();
                conn.set_selection_owner(window, clipboard, CURRENT_TIME).unwrap();
                conn.flush().unwrap();
                ready_tx.send(()).unwrap();
                while !stop.load(Ordering::SeqCst) {
                    let Some(event) = conn.poll_for_event().unwrap() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    let Event::SelectionRequest(request) = event else { continue };
                    let property = if request.target == targets_atom {
                        let mut list: Vec<u32> = offered.iter().map(|(a, _)| *a).collect();
                        list.push(targets_atom);
                        conn.change_property32(
                            PropMode::REPLACE,
                            request.requestor,
                            request.property,
                            AtomEnum::ATOM,
                            &list,
                        )
                        .unwrap();
                        request.property
                    } else if let Some((_, data)) =
                        offered.iter().find(|(a, _)| *a == request.target)
                    {
                        conn.change_property8(
                            PropMode::REPLACE,
                            request.requestor,
                            request.property,
                            request.target,
                            data,
                        )
                        .unwrap();
                        request.property
                    } else {
                        u32::from(AtomEnum::NONE)
                    };
                    conn.send_event(
                        false,
                        request.requestor,
                        EventMask::NO_EVENT,
                        SelectionNotifyEvent {
                            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
                            sequence: 0,
                            time: request.time,
                            requestor: request.requestor,
                            selection: request.selection,
                            target: request.target,
                            property,
                        },
                    )
                    .unwrap();
                    conn.flush().unwrap();
                }
            })
        };
        ready_rx.recv().unwrap();
        Self { stop, thread: Some(thread) }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Watching {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    received: Receiver<ClipboardSnapshot>,
}

fn watch() -> Watching {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, received) = mpsc::channel();
    let tx = Mutex::new(tx);
    let thread = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            X11Clipboard
                .watch(Box::new(move |snapshot| drop(tx.lock().unwrap().send(snapshot))), stop)
                .unwrap();
        })
    };
    // The watcher selects the XFixes events on start-up; give it a moment.
    std::thread::sleep(Duration::from_millis(500));
    Watching { stop, thread: Some(thread), received }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn copied_text_is_reported() {
    let Some(_serial) = x_server() else { return };
    let watching = watch();
    let _owner = Owner::offer(vec![("UTF8_STRING", "grüße".as_bytes().to_vec())]);
    let snapshot = watching.received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(snapshot.content, ClipboardContent::Text("grüße".into()));
    assert!(!snapshot.concealed);
}

#[test]
fn an_image_is_preferred_to_text() {
    let Some(_serial) = x_server() else { return };
    let watching = watch();
    let _owner = Owner::offer(vec![
        ("UTF8_STRING", b"name".to_vec()),
        ("image/png", vec![0x89, b'P', b'N', b'G']),
    ]);
    let snapshot = watching.received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(snapshot.content, ClipboardContent::Image(vec![0x89, b'P', b'N', b'G']));
}

#[test]
fn content_marked_by_a_password_manager_is_concealed() {
    let Some(_serial) = x_server() else { return };
    let watching = watch();
    let _owner = Owner::offer(vec![
        ("UTF8_STRING", b"hunter2".to_vec()),
        ("x-kde-passwordManagerHint", b"secret".to_vec()),
    ]);
    let snapshot = watching.received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(snapshot.concealed);
}
