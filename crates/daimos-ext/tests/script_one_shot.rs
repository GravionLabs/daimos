//! The one-shot script protocol end to end: a Python script is installed as an extension and
//! driven through `ScriptBackend` and `CommandProvider`. The tests skip when there is no Python.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use daimos_core::model::ItemBatch;
use daimos_core::provider::{ActionEffect, Provider, QueryContext};
use daimos_ext::builtin::Opener;
use daimos_ext::dir::load_extension;
use daimos_ext::provider::CommandProvider;
use daimos_ext::{Backend, ScriptBackend};
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

/// The Python of this machine: `python` on Windows, `python3` elsewhere, with the other as a
/// fallback. Resolved once.
fn python() -> Option<&'static str> {
    static PYTHON: OnceLock<Option<&'static str>> = OnceLock::new();
    *PYTHON.get_or_init(|| {
        let candidates: &[&str] =
            if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
        candidates.iter().copied().find(|name| {
            Command::new(name).arg("--version").output().is_ok_and(|out| out.status.success())
        })
    })
}

macro_rules! python_or_skip {
    () => {
        match python() {
            Some(python) => python,
            None => {
                eprintln!("skipped: no python3 or python on PATH");
                return;
            }
        }
    };
}

struct Installed {
    root: tempfile::TempDir,
    backend: ScriptBackend,
}

impl Installed {
    fn dir(&self) -> PathBuf {
        self.root.path().join("echo")
    }

    fn data_dir(&self) -> PathBuf {
        self.root.path().join("data")
    }

    fn runs(&self) -> usize {
        std::fs::read_to_string(self.dir().join("runs.log")).map_or(0, |log| log.lines().count())
    }

    fn query(&self, text: &str, cancel: &CancellationToken) -> BoxStream<'static, ItemBatch> {
        let ctx = QueryContext { text: text.into(), keyword_routed: true, cancel: cancel.clone() };
        self.backend.query("echo", "search", ctx)
    }

    async fn batches(&self, text: &str) -> Vec<ItemBatch> {
        self.query(text, &CancellationToken::new()).collect().await
    }
}

/// Installs `fixtures/echo.py` as the extension `echo` with the given timeout.
fn install(python: &str, timeout_ms: u64) -> Installed {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, dir.join("echo.py")).unwrap();
    let manifest = format!(
        "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
         [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = [\"e\"]\n\n\
         [runtime.script]\ncommand = [{python:?}, \"echo.py\"]\ntimeout_ms = {timeout_ms}\n"
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    let backend = ScriptBackend::with_data_dir(root.path().join("data"));
    backend.load(&load_extension(&dir).unwrap());
    Installed { root, backend }
}

fn titles(batches: &[ItemBatch]) -> Vec<String> {
    batches.iter().flat_map(|batch| batch.items.iter().map(|item| item.title.clone())).collect()
}

#[tokio::test]
async fn items_stream_in_order_and_done_ends_the_query() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let batches = ext.batches("hello").await;
    assert_eq!(batches.len(), 2);
    assert_eq!(titles(&batches), ["echo hello", "again hello", "last hello"]);
    let first = &batches[0].items[0];
    assert_eq!(first.subtitle.as_deref(), Some("from echo.py"));
    assert_eq!(first.actions.len(), 2);
    assert_eq!(ext.runs(), 1);
}

#[tokio::test]
async fn done_true_ends_the_query_without_waiting_for_the_exit() {
    let python = python_or_skip!();
    let ext = install(python, 30_000);
    let started = Instant::now();
    assert_eq!(titles(&ext.batches("tail").await), ["tail"]);
    assert!(started.elapsed() < Duration::from_secs(20), "the query waited for the process");
}

#[tokio::test]
async fn a_hanging_script_is_killed_at_the_timeout_and_one_error_item_follows_its_items() {
    let python = python_or_skip!();
    let ext = install(python, 400);
    let started = Instant::now();
    let batches = ext.batches("hang").await;
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].items[0].title, "before-hang");
    let [error] = batches[1].items.as_slice() else { panic!("{batches:?}") };
    assert_eq!(error.title, "echo took too long");
    assert_eq!(error.subtitle.as_deref(), Some("timed out after 400 ms"));
}

#[tokio::test]
async fn a_non_zero_exit_shows_one_error_item_with_the_last_stderr_line() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let batches = ext.batches("exit1").await;
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].items[0].title, "before-exit");
    let [error] = batches[1].items.as_slice() else { panic!("{batches:?}") };
    assert_eq!(error.title, "echo failed");
    assert_eq!(error.subtitle.as_deref(), Some("fixture failed"));
}

#[tokio::test]
async fn a_malformed_line_shows_one_error_item_with_the_line() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let batches = ext.batches("malformed").await;
    assert_eq!(titles(&batches[..1]), ["before-garbage"]);
    assert_eq!(batches.len(), 2, "nothing after the bad line is shown");
    let subtitle = batches[1].items[0].subtitle.clone().unwrap();
    assert!(subtitle.contains("this is not json"), "{subtitle}");
}

#[tokio::test]
async fn stderr_lines_land_in_the_extension_log() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    ext.batches("log").await;
    let path = ext.data_dir().join("logs").join("echo.log");
    let expected = "log line one\nlog line two\n";
    // The run waits only a grace period for stderr; on a busy machine the lines land later.
    let started = Instant::now();
    let mut log = String::new();
    while started.elapsed() < Duration::from_secs(10) {
        log = std::fs::read_to_string(&path).unwrap_or_default();
        if log == expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(log, expected);
}

#[tokio::test]
async fn cancel_ends_the_stream_and_kills_the_process() {
    let python = python_or_skip!();
    let ext = install(python, 60_000);
    let cancel = CancellationToken::new();
    let mut stream = ext.query("hang", &cancel);
    let first = stream.next().await.unwrap();
    assert_eq!(first.items[0].title, "before-hang");
    cancel.cancel();
    assert!(stream.next().await.is_none());
    let pid = std::fs::read_to_string(ext.dir().join("pid.txt")).unwrap();
    assert!(!process_exists(&pid), "process {pid} is still there");
}

/// Whether `pid` still has a process table entry (a zombie counts). Only checked on Unix.
fn process_exists(pid: &str) -> bool {
    cfg!(unix)
        && Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[test]
fn a_custom_action_reaches_the_script_and_its_toast_comes_back() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    assert_eq!(
        ext.backend.custom_action("echo", "search", "item-7", "star"),
        Ok(Some("Starred item-7".to_owned()))
    );
    assert_eq!(ext.backend.custom_action("echo", "search", "x", "quiet"), Ok(None));
}

#[test]
fn a_failing_or_hanging_custom_action_is_an_error_not_a_hang() {
    let python = python_or_skip!();
    let ext = install(python, 400);
    assert_eq!(
        ext.backend.custom_action("echo", "search", "x", "fail"),
        Err("echo: star failed".to_owned())
    );
    let started = Instant::now();
    let error = ext.backend.custom_action("echo", "search", "x", "hang").unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[derive(Default)]
struct Opened(std::sync::Mutex<Vec<String>>);

impl Opener for Opened {
    fn open(&self, target: &str) -> std::io::Result<()> {
        self.0.lock().unwrap().push(target.to_owned());
        Ok(())
    }
    fn run(&self, _: &str) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn built_in_actions_stay_in_the_host_and_custom_ones_go_to_the_script() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let manifest = load_extension(&ext.dir()).unwrap().manifest;
    let opened = Arc::new(Opened::default());
    let backend: Arc<dyn Backend> = Arc::new(ext.backend);
    let provider = CommandProvider::new(&manifest, &manifest.commands[0], backend)
        .with_opener(Arc::clone(&opened) as Arc<dyn Opener>);
    let ctx =
        QueryContext { text: "z".into(), keyword_routed: true, cancel: CancellationToken::new() };
    let batches: Vec<_> = provider.query(ctx).collect().await;
    assert_eq!(batches[0].items[0].accessories, ["Echo"]);

    let runs = || {
        std::fs::read_to_string(ext.root.path().join("echo").join("runs.log"))
            .map_or(0, |log| log.lines().count())
    };
    assert_eq!(runs(), 1);
    let open = provider.run_action_outcome("echo z", "open").unwrap().unwrap();
    assert_eq!(open.effect, ActionEffect::Hide);
    assert_eq!(*opened.0.lock().unwrap(), ["https://example.com/echo z"]);
    assert_eq!(runs(), 1, "open did not start the script");

    let star = provider.run_action_outcome("echo z", "star").unwrap().unwrap();
    assert_eq!((star.effect, star.toast.as_deref()), (ActionEffect::Stay, Some("Starred echo z")));
    assert_eq!(runs(), 2);
}
