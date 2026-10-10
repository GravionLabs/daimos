//! Owns the current registration and swaps it when the binding changes.

use std::sync::Mutex;

use super::{Binding, Hotkey, HotkeyError, Registration};

pub struct HotkeyService {
    backend: Box<dyn Hotkey>,
    current: Mutex<Option<Registration>>,
}

impl HotkeyService {
    pub fn new(backend: Box<dyn Hotkey>) -> Self {
        Self { backend, current: Mutex::new(None) }
    }

    pub fn backend(&self) -> &dyn Hotkey {
        self.backend.as_ref()
    }

    pub fn current(&self) -> Option<Binding> {
        self.current.lock().expect("hotkey lock").as_ref().map(|r| r.binding.clone())
    }

    /// Registers `binding`, replacing the current one. The new binding is registered first, so a
    /// refused binding leaves the old one working.
    pub fn bind(&self, binding: &Binding) -> Result<(), HotkeyError> {
        let mut current = self.current.lock().expect("hotkey lock");
        if current.as_ref().is_some_and(|r| &r.binding == binding) {
            return Ok(());
        }
        let next = self.backend.register(binding)?;
        if let Some(old) = current.replace(next) {
            self.backend.unregister(&old)?;
        }
        Ok(())
    }

    /// Unregisters the current binding, for example on quit.
    pub fn release(&self) -> Result<(), HotkeyError> {
        // A first-run portal dialog can hold the lock for minutes; quitting must not wait for it.
        let Ok(mut current) = self.current.try_lock() else { return Ok(()) };
        match current.take() {
            Some(old) => self.backend.unregister(&old),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::mpsc::Receiver;

    use super::*;
    use crate::hotkey::Press;

    /// A backend with a set of bindings the "OS" has given to someone else.
    #[derive(Default)]
    struct Fake {
        taken: HashSet<&'static str>,
        registered: Mutex<Vec<String>>,
    }

    impl Hotkey for Fake {
        fn register(&self, binding: &Binding) -> Result<Registration, HotkeyError> {
            if self.taken.contains(binding.as_str()) {
                return Err(HotkeyError::AlreadyTaken {
                    binding: binding.clone(),
                    detail: "owned by another program".into(),
                });
            }
            self.registered.lock().unwrap().push(binding.to_string());
            Ok(Registration { binding: binding.clone() })
        }

        fn unregister(&self, registration: &Registration) -> Result<(), HotkeyError> {
            self.registered.lock().unwrap().retain(|b| b != registration.binding.as_str());
            Ok(())
        }

        fn presses(&self) -> Receiver<Press> {
            std::sync::mpsc::channel().1
        }
    }

    fn service(taken: &[&'static str]) -> (HotkeyService, std::sync::Arc<Fake>) {
        struct Shared(std::sync::Arc<Fake>);
        impl Hotkey for Shared {
            fn register(&self, b: &Binding) -> Result<Registration, HotkeyError> {
                self.0.register(b)
            }
            fn unregister(&self, r: &Registration) -> Result<(), HotkeyError> {
                self.0.unregister(r)
            }
            fn presses(&self) -> Receiver<Press> {
                self.0.presses()
            }
        }
        let fake =
            std::sync::Arc::new(Fake { taken: taken.iter().copied().collect(), ..Fake::default() });
        (HotkeyService::new(Box::new(Shared(fake.clone()))), fake)
    }

    #[test]
    fn rebinding_swaps_the_registration() {
        let (service, fake) = service(&[]);
        service.bind(&Binding::new("Alt+Space")).unwrap();
        service.bind(&Binding::new("Ctrl+Space")).unwrap();
        assert_eq!(*fake.registered.lock().unwrap(), ["Ctrl+Space"]);
        assert_eq!(service.current(), Some(Binding::new("Ctrl+Space")));
    }

    #[test]
    fn a_refused_binding_keeps_the_old_one_and_names_itself() {
        let (service, fake) = service(&["Ctrl+Space"]);
        service.bind(&Binding::new("Alt+Space")).unwrap();
        let err = service.bind(&Binding::new("Ctrl+Space")).unwrap_err();
        assert!(
            matches!(&err, HotkeyError::AlreadyTaken { binding, .. } if binding.as_str() == "Ctrl+Space")
        );
        assert_eq!(*fake.registered.lock().unwrap(), ["Alt+Space"]);
    }

    #[test]
    fn binding_the_same_accelerator_twice_registers_once() {
        let (service, fake) = service(&[]);
        service.bind(&Binding::new("Alt+Space")).unwrap();
        service.bind(&Binding::new("Alt+Space")).unwrap();
        assert_eq!(fake.registered.lock().unwrap().len(), 1);
    }

    #[test]
    fn release_unregisters() {
        let (service, fake) = service(&[]);
        service.bind(&Binding::new("Alt+Space")).unwrap();
        service.release().unwrap();
        assert!(fake.registered.lock().unwrap().is_empty());
        assert_eq!(service.current(), None);
    }
}
