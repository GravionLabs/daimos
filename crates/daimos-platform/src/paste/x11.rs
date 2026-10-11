//! Ctrl+V on X11 with the XTest extension: the key events look like the user's own.

use std::io;

use daimos_core::paste::{PasteSupport, Paster};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

const XK_CONTROL_L: u32 = 0xffe3;
const XK_V: u32 = 0x0076;

pub struct XTestPaster;

impl Paster for XTestPaster {
    fn id(&self) -> &str {
        "xtest"
    }

    fn support(&self) -> PasteSupport {
        match x11rb::connect(None) {
            Ok((conn, _)) if has_xtest(&conn) => PasteSupport::Full,
            Ok(_) => PasteSupport::Unsupported("the X server has no XTest extension".into()),
            Err(error) => PasteSupport::Unsupported(format!("no X server: {error}")),
        }
    }

    fn paste(&self) -> io::Result<()> {
        let (conn, screen) = x11rb::connect(None).map_err(io::Error::other)?;
        let root = conn.setup().roots[screen].root;
        let control = keycode(&conn, XK_CONTROL_L)?;
        let v = keycode(&conn, XK_V)?;
        for (kind, code) in [
            (KEY_PRESS_EVENT, control),
            (KEY_PRESS_EVENT, v),
            (KEY_RELEASE_EVENT, v),
            (KEY_RELEASE_EVENT, control),
        ] {
            conn.xtest_fake_input(kind, code, 0, root, 0, 0, 0).map_err(io::Error::other)?;
        }
        // Closing the connection without a round trip could drop the events.
        conn.sync().map_err(io::Error::other)
    }
}

fn has_xtest(conn: &RustConnection) -> bool {
    conn.xtest_get_version(2, 2).ok().and_then(|cookie| cookie.reply().ok()).is_some()
}

/// The keycode that produces `keysym`, from the keyboard mapping of the server.
fn keycode(conn: &RustConnection, keysym: u32) -> io::Result<u8> {
    let setup = conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let mapping = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(io::Error::other)?
        .reply()
        .map_err(io::Error::other)?;
    let per_code = usize::from(mapping.keysyms_per_keycode);
    mapping
        .keysyms
        .chunks(per_code)
        .position(|syms| syms.contains(&keysym))
        .map(|index| min + index as u8)
        .ok_or_else(|| io::Error::other(format!("no key for keysym {keysym:#x}")))
}
