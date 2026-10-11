//! Runs a backend on a thread and records what it reports. Capturing is opt-in: nothing runs until
//! [`Capture::start`], and [`Capture::stop`] ends it again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use super::{ClipboardBackend, ClipboardHistory, ClipboardSupport};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("the clipboard cannot be followed here: {0}")]
    Unsupported(String),
}

pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    support: ClipboardSupport,
}

impl Capture {
    /// Starts following the clipboard. `Limited` backends start too; the caller shows
    /// [`Capture::support`] to the user.
    pub fn start(
        backend: Arc<dyn ClipboardBackend>,
        history: Arc<ClipboardHistory>,
    ) -> Result<Self, CaptureError> {
        let support = backend.support();
        if let ClipboardSupport::Unsupported(reason) = support {
            return Err(CaptureError::Unsupported(reason));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("clipboard".into())
                .spawn(move || {
                    let sink = Box::new(move |snapshot| {
                        if let Err(error) = history.record(&snapshot) {
                            log::warn!("cannot store a clipboard entry: {error}");
                        }
                    });
                    if let Err(error) = backend.watch(sink, stop) {
                        log::warn!("clipboard backend {} stopped: {error}", backend.id());
                    }
                })
                .expect("a thread can be spawned")
        };
        Ok(Self { stop, thread: Some(thread), support })
    }

    pub fn support(&self) -> &ClipboardSupport {
        &self.support
    }

    /// Stops the backend and waits for its thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::super::{ClipboardContent, ClipboardSink, ClipboardSnapshot, Limits};
    use super::*;

    /// Reports the queued snapshots, then waits for the stop flag.
    struct Mock {
        support: ClipboardSupport,
        queue: Mutex<Vec<ClipboardSnapshot>>,
    }

    impl ClipboardBackend for Mock {
        fn id(&self) -> &str {
            "mock"
        }

        fn support(&self) -> ClipboardSupport {
            self.support.clone()
        }

        fn watch(&self, mut sink: ClipboardSink, stop: Arc<AtomicBool>) -> io::Result<()> {
            for snapshot in self.queue.lock().unwrap().drain(..) {
                sink(snapshot);
            }
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }
    }

    fn snapshot(text: &str, concealed: bool) -> ClipboardSnapshot {
        ClipboardSnapshot { content: ClipboardContent::Text(text.into()), concealed }
    }

    fn mock(support: ClipboardSupport, queue: Vec<ClipboardSnapshot>) -> Arc<Mock> {
        Arc::new(Mock { support, queue: Mutex::new(queue) })
    }

    #[test]
    fn what_the_backend_reports_is_stored_except_concealed_content() {
        let history = Arc::new(ClipboardHistory::in_memory(Limits::default()).unwrap());
        let backend = mock(
            ClipboardSupport::Full,
            vec![snapshot("one", false), snapshot("secret", true), snapshot("one", false)],
        );
        let capture = Capture::start(backend, Arc::clone(&history)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while history.list().unwrap().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        capture.stop();
        let entries = history.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text.as_deref(), Some("one"));
    }

    #[test]
    fn a_limited_backend_starts_and_reports_why() {
        let history = Arc::new(ClipboardHistory::in_memory(Limits::default()).unwrap());
        let backend = mock(ClipboardSupport::Limited("only while focused".into()), vec![]);
        let capture = Capture::start(backend, history).unwrap();
        assert_eq!(capture.support(), &ClipboardSupport::Limited("only while focused".into()));
    }

    #[test]
    fn an_unsupported_backend_does_not_start() {
        let history = Arc::new(ClipboardHistory::in_memory(Limits::default()).unwrap());
        let backend = mock(ClipboardSupport::Unsupported("no protocol".into()), vec![]);
        assert!(matches!(
            Capture::start(backend, history),
            Err(CaptureError::Unsupported(reason)) if reason == "no protocol"
        ));
    }
}
