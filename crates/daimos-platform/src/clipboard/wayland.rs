//! The Wayland clipboard through `wlr-data-control`: a client that binds the data-control manager
//! is told about every new selection, whichever window has the focus (sway, Hyprland, niri, KDE
//! Plasma).
//!
//! GNOME's compositor has no such protocol. There [`WaylandClipboard::support`] reports `Limited`
//! and [`WaylandClipboard::watch`] only waits, so nothing is recorded yet. Recording while the
//! Daimos window is focused is a separate piece of work.

use std::collections::HashMap;
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use daimos_core::clipboard::{
    ClipboardBackend, ClipboardContent, ClipboardSink, ClipboardSnapshot, ClipboardSupport,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_device_v1::{
    self, ZwlrDataControlDeviceV1,
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1::{
    self, ZwlrDataControlOfferV1,
};

/// How long the source of a selection may take to deliver its data.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// More than this is dropped (the image limit of the history is 10 MB).
const MAX_TRANSFER: usize = 32 * 1024 * 1024;
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";

pub struct WaylandClipboard;

impl WaylandClipboard {
    /// Whether the compositor offers the data-control protocol.
    fn has_data_control() -> Result<bool, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland display: {e}"))?;
        let (globals, _queue) = registry_queue_init::<State>(&conn)
            .map_err(|e| format!("cannot list the Wayland globals: {e}"))?;
        Ok(globals.contents().with_list(|list| {
            list.iter().any(|g| g.interface == ZwlrDataControlManagerV1::interface().name)
        }))
    }
}

impl ClipboardBackend for WaylandClipboard {
    fn id(&self) -> &str {
        "wlr-data-control"
    }

    fn support(&self) -> ClipboardSupport {
        match Self::has_data_control() {
            Ok(true) => ClipboardSupport::Full,
            Ok(false) => ClipboardSupport::Limited(
                "this compositor has no wlr-data-control (GNOME), so copied content is not \
                 recorded yet"
                    .into(),
            ),
            Err(reason) => ClipboardSupport::Unsupported(reason),
        }
    }

    fn watch(&self, sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()> {
        let conn = Connection::connect_to_env().map_err(io::Error::other)?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(io::Error::other)?;
        let qh = queue.handle();
        let Ok(manager) = globals.bind::<ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ()) else {
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(200));
            }
            return Ok(());
        };
        let seat = globals
            .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=2, ())
            .map_err(|e| io::Error::other(format!("no seat: {e}")))?;
        let _device = manager.get_data_device(&seat, &qh, ());

        let mut state = State { conn: conn.clone(), sink, offers: HashMap::new(), ready: false };
        // The selection that exists now is announced right after binding; it is not a change.
        queue.roundtrip(&mut state).map_err(io::Error::other)?;
        state.ready = true;

        while !stop.load(Ordering::SeqCst) {
            queue.dispatch_pending(&mut state).map_err(io::Error::other)?;
            conn.flush().map_err(io::Error::other)?;
            let Some(guard) = queue.prepare_read() else { continue };
            if wait_readable(guard.connection_fd().as_raw_fd(), Duration::from_millis(200)) {
                guard.read().map_err(io::Error::other)?;
            }
        }
        Ok(())
    }
}

fn wait_readable(fd: i32, timeout: Duration) -> bool {
    let mut poll = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, and the count says so.
    unsafe { libc::poll(&mut poll, 1, timeout.as_millis() as i32) > 0 }
}

struct State {
    conn: Connection,
    sink: ClipboardSink,
    /// The mime types each offer announced, by offer id.
    offers: HashMap<wayland_client::backend::ObjectId, Vec<String>>,
    ready: bool,
}

impl State {
    fn selection(&mut self, offer: ZwlrDataControlOfferV1) {
        let mimes = self.offers.remove(&offer.id()).unwrap_or_default();
        if self.ready
            && let Some(snapshot) = self.read(&offer, &mimes)
        {
            (self.sink)(snapshot);
        }
        offer.destroy();
    }

    fn read(&self, offer: &ZwlrDataControlOfferV1, mimes: &[String]) -> Option<ClipboardSnapshot> {
        let concealed = mimes.iter().any(|m| m == PASSWORD_HINT);
        let (mime, image) = if mimes.iter().any(|m| m == "image/png") {
            ("image/png", true)
        } else {
            let text = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING"];
            (*text.iter().find(|t| mimes.iter().any(|m| m == **t))?, false)
        };
        let data = self.receive(offer, mime)?;
        let content = if image {
            ClipboardContent::Image(data)
        } else {
            ClipboardContent::Text(String::from_utf8_lossy(&data).into_owned())
        };
        Some(ClipboardSnapshot { content, concealed })
    }

    /// Has the source write `mime` into a pipe and reads it to the end.
    fn receive(&self, offer: &ZwlrDataControlOfferV1, mime: &str) -> Option<Vec<u8>> {
        let mut fds = [0; 2];
        // SAFETY: `fds` has room for the two descriptors pipe2 writes.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return None;
        }
        // SAFETY: both descriptors are new and owned by nobody else.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        offer.receive(mime.to_owned(), write.as_fd());
        drop(write);
        self.conn.flush().ok()?;

        let deadline = Instant::now() + READ_TIMEOUT;
        let mut file = std::fs::File::from(read);
        let mut data = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let now = Instant::now();
            if now >= deadline || !wait_readable(file.as_raw_fd(), deadline - now) {
                return None;
            }
            match file.read(&mut buffer) {
                Ok(0) => return Some(data),
                Ok(n) => {
                    data.extend_from_slice(&buffer[..n]);
                    if data.len() > MAX_TRANSFER {
                        return None;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(id.id(), Vec::new());
            }
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) } => {
                state.selection(offer);
            }
            // The primary selection (middle click) is not the clipboard.
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.offers.remove(&offer.id());
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(State, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for State {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offers.entry(offer.id()).or_default().push(mime_type);
        }
    }
}
