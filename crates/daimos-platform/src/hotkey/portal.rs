//! Wayland: the XDG `GlobalShortcuts` portal (KDE Plasma 6, GNOME 48 and later, Hyprland).
//!
//! The portal, not the app, owns the key combination: the app asks for a `toggle` shortcut with a
//! preferred trigger and the compositor shows a dialog the first time. The portal remembers the
//! binding for this app id, so later starts find `toggle` already listed and show no dialog.
//! The preferred trigger is only a suggestion; the user may have picked another one.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ashpd::desktop::Session;
use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use tokio::sync::oneshot;

use super::{Binding, Hotkey, HotkeyError, Press, PressBus, Registration};

/// Id of the one shortcut Daimos registers.
pub const TOGGLE_ID: &str = "toggle";

/// The application id the portal knows Daimos by.
pub(super) const APP_ID: &str = "dev.gravionlabs.daimos";

enum Command {
    Register { binding: Binding, reply: Sender<Result<(), HotkeyError>> },
    Unregister { reply: Sender<()> },
}

pub struct PortalHotkey {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    bus: Arc<PressBus>,
    /// What the portal says the `toggle` trigger is (see [`Hotkey::reported_trigger`]).
    reported: Arc<Mutex<Option<String>>>,
}

impl PortalHotkey {
    /// Starts the portal thread. Nothing talks to D-Bus until the first `register`.
    pub fn new(compositor: String) -> Self {
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let bus = Arc::new(PressBus::default());
        let thread_bus = Arc::clone(&bus);
        let reported = Arc::new(Mutex::new(None));
        let thread_reported = Arc::clone(&reported);
        std::thread::Builder::new()
            .name("daimos-portal-hotkey".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                runtime.block_on(serve(receiver, thread_bus, thread_reported, compositor));
            })
            .expect("spawn portal thread");
        Self { commands, bus, reported }
    }

    fn call<T>(&self, command: impl FnOnce(Sender<T>) -> Command) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(command(reply)).ok()?;
        // The first registration may wait for the user to answer the portal dialog.
        answer.recv_timeout(Duration::from_secs(300)).ok()
    }
}

impl Hotkey for PortalHotkey {
    fn register(&self, binding: &Binding) -> Result<Registration, HotkeyError> {
        trigger_for(binding)?;
        self.call(|reply| Command::Register { binding: binding.clone(), reply })
            .unwrap_or_else(|| Err(HotkeyError::Backend("the portal thread is gone".into())))?;
        Ok(Registration { binding: binding.clone() })
    }

    fn unregister(&self, _: &Registration) -> Result<(), HotkeyError> {
        self.call(|reply| Command::Unregister { reply });
        Ok(())
    }

    fn presses(&self) -> Receiver<Press> {
        self.bus.subscribe()
    }

    fn reported_trigger(&self) -> Option<String> {
        self.reported.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

struct Live {
    session: Session<GlobalShortcuts>,
    stop: oneshot::Sender<()>,
}

async fn serve(
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Command>,
    bus: Arc<PressBus>,
    reported: Arc<Mutex<Option<String>>>,
    compositor: String,
) {
    let mut live: Option<Live> = None;
    // The binding of the last successful registration: a different one is a change of mind, so the
    // portal is asked again; the same one after a restart finds `toggle` bound and asks nothing.
    let mut last: Option<Binding> = None;
    while let Some(command) = commands.recv().await {
        match command {
            Command::Register { binding, reply } => {
                // The new session first: a failed attempt keeps the old one alive.
                let rebind = should_rebind(last.as_ref(), &binding);
                let result = open(&binding, rebind, &bus, &compositor).await;
                let _ = reply.send(match result {
                    Ok((opened, trigger)) => {
                        close(live.replace(opened)).await;
                        *reported.lock().unwrap_or_else(|e| e.into_inner()) = trigger;
                        last = Some(binding);
                        Ok(())
                    }
                    Err(error) => Err(error),
                });
            }
            Command::Unregister { reply } => {
                close(live.take()).await;
                let _ = reply.send(());
            }
        }
    }
    close(live.take()).await;
}

/// A binding that differs from the last registered one is a change of mind (offer it to the
/// portal again); the first registration of a run, or the same binding again, asks nothing of a
/// shortcut the portal has bound already (no dialog at every start).
fn should_rebind(last: Option<&Binding>, requested: &Binding) -> bool {
    last.is_some_and(|previous| previous != requested)
}

async fn close(live: Option<Live>) {
    if let Some(live) = live {
        let _ = live.stop.send(());
        let _ = live.session.close().await;
    }
}

/// Opens a session and makes sure `toggle` is bound. `rebind` offers the portal the trigger even
/// when `toggle` is bound already (the user changed the hotkey in the settings); what the portal
/// keeps is its decision, so the trigger it reports is returned along with the session.
async fn open(
    binding: &Binding,
    rebind: bool,
    bus: &Arc<PressBus>,
    compositor: &str,
) -> Result<(Live, Option<String>), HotkeyError> {
    let unsupported = |error: ashpd::Error| {
        log::info!("GlobalShortcuts portal unavailable: {error}");
        HotkeyError::Unsupported { compositor: compositor.to_owned() }
    };
    // A process that is not sandboxed has no app id for the portal to attach the shortcut to
    // ("An app id is required"); the registry interface gives it one. Same as the Tauri identifier.
    if let Err(error) = super::desktop_entry::ensure_for_current_exe() {
        log::info!("cannot write the desktop entry: {error}");
    }
    if let Ok(app_id) = ashpd::AppID::try_from(APP_ID)
        && let Err(error) = ashpd::register_host_app(app_id).await
    {
        log::info!("cannot register the host app id: {error}");
    }
    let portal = GlobalShortcuts::new().await.map_err(unsupported)?;
    let session = portal.create_session(Default::default()).await.map_err(unsupported)?;

    let backend = |e: ashpd::Error| HotkeyError::Backend(e.to_string());
    let listed = || async {
        portal
            .list_shortcuts(&session, Default::default())
            .await
            .and_then(|request| request.response())
            .map(|list| {
                list.shortcuts()
                    .iter()
                    .find(|s| s.id() == TOGGLE_ID)
                    .map(|s| s.trigger_description().to_owned())
            })
    };
    let bound = listed().await.map_err(backend)?;
    if bound.is_none() || rebind {
        let shortcut = NewShortcut::new(TOGGLE_ID, "Show or hide Daimos")
            .preferred_trigger(trigger_for(binding)?.as_str());
        portal
            .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
            .await
            .and_then(|request| request.response())
            .map_err(backend)?;
    }
    // Whatever the portal ended up with: it, not the app, owns the key combination.
    let trigger = listed().await.ok().flatten().filter(|t| !t.trim().is_empty());

    let mut activated =
        portal.receive_activated().await.map_err(|e| HotkeyError::Backend(e.to_string()))?;
    let (stop, mut stopped) = oneshot::channel();
    let (bus, binding) = (Arc::clone(bus), binding.clone());
    tokio::task::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                event = activated.next() => match event {
                    Some(event) if event.shortcut_id() == TOGGLE_ID => {
                        bus.publish(&Press { binding: binding.clone() });
                    }
                    Some(_) => {}
                    None => break,
                },
            }
        }
    });
    Ok((Live { session, stop }, trigger))
}

/// Turns `Alt+Space` into the portal's `ALT+space` (XDG shortcuts specification: upper-case
/// modifiers, xkb key names).
pub fn trigger_for(binding: &Binding) -> Result<String, HotkeyError> {
    let invalid =
        |detail: &str| HotkeyError::Invalid { binding: binding.clone(), detail: detail.to_owned() };
    let mut parts: Vec<&str> = binding.as_str().split('+').map(str::trim).collect();
    let key = parts.pop().filter(|k| !k.is_empty()).ok_or_else(|| invalid("no key"))?;
    let mut out = Vec::new();
    for modifier in parts {
        out.push(match modifier.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "CTRL",
            "alt" | "option" => "ALT",
            "shift" => "SHIFT",
            "super" | "meta" | "cmd" | "command" | "logo" => "LOGO",
            _ => return Err(invalid(&format!("unknown modifier {modifier}"))),
        });
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "space" => "space".to_owned(),
        "enter" | "return" => "Return".to_owned(),
        "esc" | "escape" => "Escape".to_owned(),
        "tab" => "Tab".to_owned(),
        "up" | "down" | "left" | "right" => {
            let lower = key.to_ascii_lowercase();
            format!("{}{}", lower[..1].to_ascii_uppercase(), &lower[1..])
        }
        other if other.len() == 1 => other.to_owned(),
        other if other.starts_with('f') && other[1..].parse::<u8>().is_ok() => {
            other.to_ascii_uppercase()
        }
        _ => return Err(invalid(&format!("unknown key {key}"))),
    };
    out.push(&key);
    Ok(out.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(s: &str) -> Result<String, HotkeyError> {
        trigger_for(&Binding::new(s))
    }

    #[test]
    fn the_default_bindings_map_to_portal_triggers() {
        assert_eq!(trigger("Alt+Space").unwrap(), "ALT+space");
        assert_eq!(trigger("Option+Space").unwrap(), "ALT+space");
        assert_eq!(trigger("Ctrl+Shift+P").unwrap(), "CTRL+SHIFT+p");
        assert_eq!(trigger("Super+F11").unwrap(), "LOGO+F11");
        assert_eq!(trigger("Ctrl+Up").unwrap(), "CTRL+Up");
    }

    #[test]
    fn unknown_parts_are_invalid_and_name_the_binding() {
        assert!(matches!(trigger("Hyper+A"), Err(HotkeyError::Invalid { .. })));
        assert!(matches!(trigger("Ctrl+Banana"), Err(HotkeyError::Invalid { .. })));
        assert!(matches!(trigger(""), Err(HotkeyError::Invalid { .. })));
    }

    #[test]
    fn only_a_changed_binding_is_offered_to_the_portal_again() {
        let alt = Binding::new("Alt+Space");
        let other = Binding::new("Ctrl+Alt+K");
        assert!(!should_rebind(None, &alt), "the first registration of a run");
        assert!(!should_rebind(Some(&alt), &alt), "the same binding again");
        assert!(should_rebind(Some(&alt), &other), "a change in the settings");
    }
}
