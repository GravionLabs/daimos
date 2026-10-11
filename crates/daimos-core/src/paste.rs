//! Pasting into the window the user came from: the clipboard holds the text, the window that had
//! the focus before Daimos opened gets the paste shortcut.
//!
//! The pasters live in `daimos-platform`; this crate only sees the [`Paster`] trait.

use std::io;
use std::thread;
use std::time::Duration;

/// How much of "send the paste shortcut" a paster can do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteSupport {
    Full,
    /// Possible after the user does something, for example grants the macOS Accessibility
    /// permission; the reason says what.
    Limited(String),
    Unsupported(String),
}

pub trait Paster: Send + Sync {
    /// `xtest`, `wtype`, `ydotool`, `sendinput`, `cgevent`, ...
    fn id(&self) -> &str;

    fn support(&self) -> PasteSupport;

    /// Sends the paste shortcut to the window that has the focus.
    fn paste(&self) -> io::Result<()>;
}

/// What happened to a paste request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteOutcome {
    Pasted,
    /// The text is on the clipboard only; the reason says why it was not pasted.
    Copied(String),
}

/// Sends the paste shortcut once the previous window has the focus again. `settle` is the time
/// the window system needs to hand the focus back after the Daimos window was hidden.
pub fn paste_into_previous(paster: &dyn Paster, settle: Duration) -> PasteOutcome {
    match paster.support() {
        PasteSupport::Full => {}
        PasteSupport::Limited(reason) | PasteSupport::Unsupported(reason) => {
            return PasteOutcome::Copied(reason);
        }
    }
    thread::sleep(settle);
    match paster.paste() {
        Ok(()) => PasteOutcome::Pasted,
        Err(error) => PasteOutcome::Copied(format!("the paste shortcut failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct Fake {
        support: PasteSupport,
        fail: bool,
        pasted: AtomicUsize,
    }

    impl Fake {
        fn new(support: PasteSupport, fail: bool) -> Self {
            Self { support, fail, pasted: AtomicUsize::new(0) }
        }
    }

    impl Paster for Fake {
        fn id(&self) -> &str {
            "fake"
        }

        fn support(&self) -> PasteSupport {
            self.support.clone()
        }

        fn paste(&self) -> io::Result<()> {
            self.pasted.fetch_add(1, Ordering::SeqCst);
            if self.fail { Err(io::Error::other("boom")) } else { Ok(()) }
        }
    }

    #[test]
    fn a_working_paster_pastes_once() {
        let fake = Fake::new(PasteSupport::Full, false);
        assert_eq!(paste_into_previous(&fake, Duration::ZERO), PasteOutcome::Pasted);
        assert_eq!(fake.pasted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn limited_and_unsupported_pasters_degrade_to_copy_with_the_reason() {
        for support in [
            PasteSupport::Limited("grant Accessibility".into()),
            PasteSupport::Unsupported("install wtype".into()),
        ] {
            let reason = match &support {
                PasteSupport::Limited(r) | PasteSupport::Unsupported(r) => r.clone(),
                PasteSupport::Full => unreachable!(),
            };
            let fake = Fake::new(support, false);
            assert_eq!(paste_into_previous(&fake, Duration::ZERO), PasteOutcome::Copied(reason));
            assert_eq!(fake.pasted.load(Ordering::SeqCst), 0, "nothing is sent");
        }
    }

    #[test]
    fn a_failing_shortcut_degrades_to_copy() {
        let fake = Fake::new(PasteSupport::Full, true);
        let PasteOutcome::Copied(reason) = paste_into_previous(&fake, Duration::ZERO) else {
            panic!("expected a copy");
        };
        assert!(reason.contains("boom"));
    }
}
