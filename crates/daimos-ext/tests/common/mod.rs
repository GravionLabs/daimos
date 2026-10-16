//! Installs the Python fixture `fixtures/echo.py` as a script extension in either mode. Shared by
//! the mode conformance test and the warm round-trip benchmark (and meant for the conformance
//! suite of the script runtime).
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use daimos_core::model::ItemBatch;
use daimos_core::provider::QueryContext;
use daimos_ext::dir::load_extension;
use daimos_ext::{Backend, ScriptBackend};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

/// The Python of this machine: `python` on Windows, `python3` elsewhere, with the other as a
/// fallback. Resolved once.
pub fn python() -> Option<&'static str> {
    static PYTHON: OnceLock<Option<&'static str>> = OnceLock::new();
    *PYTHON.get_or_init(|| {
        let candidates: &[&str] =
            if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
        candidates.iter().copied().find(|name| {
            Command::new(name).arg("--version").output().is_ok_and(|out| out.status.success())
        })
    })
}

/// Returns the Python or skips the test with a note on stderr.
#[macro_export]
macro_rules! python_or_skip {
    () => {
        match $crate::common::python() {
            Some(python) => python,
            None => {
                eprintln!("skipped: no python3 or python on PATH");
                return;
            }
        }
    };
}

/// `fixtures/echo.py` installed as the extension `echo` (command `search`, keyword `e`).
pub struct Installed {
    _root: tempfile::TempDir,
    pub backend: ScriptBackend,
}

impl Installed {
    /// Installs the fixture; the one-shot mode runs `echo.py`, the long-running mode `echo.py loop`.
    pub fn new(python: &str, long_running: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("echo");
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
        std::fs::copy(fixture, dir.join("echo.py")).unwrap();
        let args = if long_running { ", \"loop\"" } else { "" };
        let manifest = format!(
            "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
             [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = [\"e\"]\n\n\
             [runtime.script]\ncommand = [{python:?}, \"echo.py\"{args}]\ntimeout_ms = 10000\n\
             long_running = {long_running}\n"
        );
        std::fs::write(dir.join("extension.toml"), manifest).unwrap();
        let backend = ScriptBackend::with_data_dir(root.path().join("data"));
        backend.load(&load_extension(&dir).unwrap());
        Self { _root: root, backend }
    }

    /// All batches of one query, until the stream ends.
    pub async fn batches(&self, text: &str) -> Vec<ItemBatch> {
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: true,
            cancel: CancellationToken::new(),
        };
        self.backend.query("echo", "search", ctx).collect().await
    }
}
