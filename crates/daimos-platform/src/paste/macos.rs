//! Cmd+V on macOS with `CGEvent`. Posting key events needs the Accessibility permission; without
//! it the paster is `Limited` and the reason names the setting.

use std::ffi::c_void;
use std::io;

use daimos_core::paste::{PasteSupport, Paster};

/// `kVK_ANSI_V`.
const KEY_V: u16 = 9;
const FLAG_COMMAND: u64 = 0x0010_0000;
/// `kCGHIDEventTap`.
const HID_EVENT_TAP: u32 = 0;

pub const ACCESSIBILITY_SETTING: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const c_void);
}

pub struct CgEventPaster;

/// The reason shown while the permission is missing.
pub fn permission_reason() -> String {
    "Daimos needs the Accessibility permission to paste: System Settings → Privacy & Security → \
     Accessibility"
        .into()
}

impl Paster for CgEventPaster {
    fn id(&self) -> &str {
        "cgevent"
    }

    fn support(&self) -> PasteSupport {
        // SAFETY: a plain query without arguments.
        if unsafe { AXIsProcessTrusted() } {
            PasteSupport::Full
        } else {
            PasteSupport::Limited(permission_reason())
        }
    }

    fn paste(&self) -> io::Result<()> {
        for down in [true, false] {
            // SAFETY: the event is created, used and released here; a null source is allowed.
            unsafe {
                let event = CGEventCreateKeyboardEvent(std::ptr::null(), KEY_V, down);
                if event.is_null() {
                    return Err(io::Error::other("cannot create the key event"));
                }
                CGEventSetFlags(event, FLAG_COMMAND);
                CGEventPost(HID_EVENT_TAP, event);
                CFRelease(event);
            }
        }
        Ok(())
    }
}
