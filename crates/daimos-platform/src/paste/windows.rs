//! Ctrl+V on Windows with `SendInput`.

use std::io;
use std::mem::size_of;

use daimos_core::paste::{PasteSupport, Paster};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY, VK_CONTROL,
};

const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

pub struct SendInputPaster;

fn key(code: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: code, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

impl Paster for SendInputPaster {
    fn id(&self) -> &str {
        "sendinput"
    }

    fn support(&self) -> PasteSupport {
        PasteSupport::Full
    }

    fn paste(&self) -> io::Result<()> {
        let inputs = [
            key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
            key(VK_V, KEYBD_EVENT_FLAGS(0)),
            key(VK_V, KEYEVENTF_KEYUP),
            key(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        // SAFETY: `inputs` is a valid slice of INPUT structures and the size is theirs.
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(io::Error::other(format!("only {sent} of {} key events were sent", inputs.len())))
        }
    }
}
