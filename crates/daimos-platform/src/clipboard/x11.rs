//! The X11 clipboard: XFixes reports every new owner of the `CLIPBOARD` selection, and the content
//! is then requested like any other client does (`TARGETS`, then `image/png` or `UTF8_STRING`),
//! including the `INCR` transfer for large content.

use std::io;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use daimos_core::clipboard::{
    ClipboardBackend, ClipboardContent, ClipboardSink, ClipboardSnapshot, ClipboardSupport,
};
use x11rb::atom_manager;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, Property, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

/// How long the owner of the selection may take to answer one request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// A transfer larger than this is dropped (the image limit of the history is 10 MB).
const MAX_TRANSFER: usize = 32 * 1024 * 1024;

atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        UTF8_STRING,
        STRING,
        INCR,
        DAIMOS_CLIPBOARD,
        PNG: b"image/png",
        PASSWORD_HINT: b"x-kde-passwordManagerHint",
    }
}

pub struct X11Clipboard;

impl ClipboardBackend for X11Clipboard {
    fn id(&self) -> &str {
        "x11"
    }

    fn support(&self) -> ClipboardSupport {
        match x11rb::connect(None) {
            Ok(_) => ClipboardSupport::Full,
            Err(error) => ClipboardSupport::Unsupported(format!("no X server: {error}")),
        }
    }

    fn watch(&self, mut sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()> {
        let (conn, screen) = x11rb::connect(None).map_err(io::Error::other)?;
        let root = conn.setup().roots[screen].root;
        let atoms =
            Atoms::new(&conn).map_err(io::Error::other)?.reply().map_err(io::Error::other)?;
        let window = conn.generate_id().map_err(io::Error::other)?;
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
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(io::Error::other)?;
        conn.xfixes_query_version(5, 0)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        conn.xfixes_select_selection_input(
            window,
            atoms.CLIPBOARD,
            SelectionEventMask::SET_SELECTION_OWNER,
        )
        .map_err(io::Error::other)?;
        conn.flush().map_err(io::Error::other)?;

        let mut client = Client { conn: &conn, window, atoms, changed: false };
        while !stop.load(Ordering::SeqCst) {
            wait_readable(&conn, Duration::from_millis(200));
            while let Some(event) = conn.poll_for_event().map_err(io::Error::other)? {
                if matches!(&event, Event::XfixesSelectionNotify(e) if e.selection == atoms.CLIPBOARD)
                {
                    client.changed = true;
                }
            }
            // A change that arrives while the content is being read is read again afterwards.
            while std::mem::take(&mut client.changed) && !stop.load(Ordering::SeqCst) {
                match client.snapshot() {
                    Ok(Some(snapshot)) => sink(snapshot),
                    Ok(None) => {}
                    Err(error) => log::debug!("cannot read the X11 clipboard: {error}"),
                }
            }
        }
        Ok(())
    }
}

fn wait_readable(conn: &RustConnection, timeout: Duration) {
    let mut fd = libc::pollfd { fd: conn.stream().as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd, and the count says so.
    unsafe { libc::poll(&mut fd, 1, timeout.as_millis() as i32) };
}

struct Client<'a> {
    conn: &'a RustConnection,
    window: u32,
    atoms: Atoms,
    /// Another change arrived while a request was waiting for its answer.
    changed: bool,
}

type Error = Box<dyn std::error::Error>;

impl Client<'_> {
    fn snapshot(&mut self) -> Result<Option<ClipboardSnapshot>, Error> {
        let Some(targets) = self.request(self.atoms.TARGETS)? else { return Ok(None) };
        let targets: Vec<Atom> =
            targets.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
        let concealed = targets.contains(&self.atoms.PASSWORD_HINT);
        let (target, image) = if targets.contains(&self.atoms.PNG) {
            (self.atoms.PNG, true)
        } else if targets.contains(&self.atoms.UTF8_STRING) {
            (self.atoms.UTF8_STRING, false)
        } else if targets.contains(&self.atoms.STRING) {
            (self.atoms.STRING, false)
        } else {
            return Ok(None);
        };
        let Some(data) = self.request(target)? else { return Ok(None) };
        let content = if image {
            ClipboardContent::Image(data)
        } else {
            ClipboardContent::Text(String::from_utf8_lossy(&data).into_owned())
        };
        Ok(Some(ClipboardSnapshot { content, concealed }))
    }

    /// Asks the owner of the selection to convert it to `target`; `None` when it refuses or does
    /// not answer in time.
    fn request(&mut self, target: Atom) -> Result<Option<Vec<u8>>, Error> {
        let (conn, window, property) = (self.conn, self.window, self.atoms.DAIMOS_CLIPBOARD);
        conn.delete_property(window, property)?;
        conn.convert_selection(window, self.atoms.CLIPBOARD, target, property, CURRENT_TIME)?;
        conn.flush()?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            match self.next_event(deadline)? {
                None => return Ok(None),
                Some(Event::SelectionNotify(e)) if e.requestor == window => {
                    if e.property == u32::from(AtomEnum::NONE) {
                        return Ok(None);
                    }
                    break;
                }
                Some(_) => {}
            }
        }
        let reply = conn
            .get_property(true, window, property, AtomEnum::ANY, 0, MAX_TRANSFER as u32 / 4)?
            .reply()?;
        if reply.type_ != self.atoms.INCR {
            return Ok((reply.bytes_after == 0).then_some(reply.value));
        }
        self.receive_incr(deadline + REQUEST_TIMEOUT)
    }

    /// Chunks arrive as `PropertyNotify` events until an empty one ends the transfer.
    fn receive_incr(&mut self, mut deadline: Instant) -> Result<Option<Vec<u8>>, Error> {
        let (conn, window, property) = (self.conn, self.window, self.atoms.DAIMOS_CLIPBOARD);
        let mut data = Vec::new();
        loop {
            match self.next_event(deadline)? {
                None => return Ok(None),
                Some(Event::PropertyNotify(e))
                    if e.window == window
                        && e.atom == property
                        && e.state == Property::NEW_VALUE =>
                {
                    let chunk = conn
                        .get_property(
                            true,
                            window,
                            property,
                            AtomEnum::ANY,
                            0,
                            MAX_TRANSFER as u32 / 4,
                        )?
                        .reply()?;
                    if chunk.value.is_empty() {
                        return Ok(Some(data));
                    }
                    data.extend_from_slice(&chunk.value);
                    if data.len() > MAX_TRANSFER {
                        return Ok(None);
                    }
                    deadline = Instant::now() + REQUEST_TIMEOUT;
                }
                Some(_) => {}
            }
        }
    }

    /// The next event, or `None` at `deadline`. A change of the clipboard is remembered.
    fn next_event(&mut self, deadline: Instant) -> Result<Option<Event>, Error> {
        loop {
            if let Some(event) = self.conn.poll_for_event()? {
                if matches!(&event, Event::XfixesSelectionNotify(e) if e.selection == self.atoms.CLIPBOARD)
                {
                    self.changed = true;
                    continue;
                }
                return Ok(Some(event));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            wait_readable(self.conn, (deadline - now).min(Duration::from_millis(100)));
        }
    }
}
