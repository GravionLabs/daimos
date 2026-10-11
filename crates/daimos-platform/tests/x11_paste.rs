//! The XTest paster against a second X client that has the input focus. Needs an X server
//! (`DISPLAY`) and no Wayland session; otherwise the test does nothing.
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use daimos_core::paste::{PasteSupport, Paster};
use daimos_platform::paste::x11::XTestPaster;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    ConnectionExt as _, CreateWindowAux, EventMask, InputFocus, KeyButMask, WindowClass,
};
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

#[test]
fn ctrl_v_reaches_the_focused_window() {
    // Never against a desktop: under a Wayland session DISPLAY is Xwayland, and the test would
    // type into the user's windows. CI and `xvfb-run` have no Wayland display.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }
    let Some(_) = std::env::var_os("DISPLAY") else { return };
    let Ok((conn, screen)) = x11rb::connect(None) else { return };
    if XTestPaster.support() != PasteSupport::Full {
        return;
    }
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id().unwrap();
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        window,
        root,
        0,
        0,
        100,
        100,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS | EventMask::EXPOSURE),
    )
    .unwrap();
    conn.map_window(window).unwrap();
    conn.flush().unwrap();
    // Focus can only go to a viewable window: wait until the server shows it.
    let exposed = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.poll_for_event().unwrap() {
            Some(Event::Expose(_)) => break,
            Some(_) => {}
            None => {
                assert!(Instant::now() < exposed, "the window was never shown");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    conn.set_input_focus(InputFocus::NONE, window, CURRENT_TIME).unwrap().check().unwrap();

    XTestPaster.paste().unwrap();

    // The second key press is the V, with Control held.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut presses = Vec::new();
    while presses.len() < 2 && Instant::now() < deadline {
        match conn.poll_for_event().unwrap() {
            Some(Event::KeyPress(event)) => presses.push(event),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    assert_eq!(presses.len(), 2, "Control and V arrive as key presses");
    assert!(presses[1].state.contains(KeyButMask::CONTROL));
}
