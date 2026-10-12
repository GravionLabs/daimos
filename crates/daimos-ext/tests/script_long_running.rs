//! The long-running script protocol end to end: the Python fixture runs in loop mode behind a
//! `Session`. The tests skip when there is no Python.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use daimos_ext::dir::load_extension;
use daimos_ext::script::Event;
use daimos_ext::{Failure, Session};
use futures::StreamExt;
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
    session: Session,
}

impl Installed {
    fn dir(&self) -> PathBuf {
        self.root.path().join("echo")
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir().join(name)).unwrap_or_default()
    }

    /// How many times the fixture process started.
    fn starts(&self) -> usize {
        self.read("runs.log").lines().count()
    }

    fn pid(&self) -> String {
        self.read("pid.txt")
    }

    async fn titles(&self, text: &str) -> Vec<String> {
        let events: Vec<Event> = self
            .session
            .query("search", text, Default::default(), CancellationToken::new())
            .collect()
            .await;
        titles(&events)
    }

    async fn query(&self, text: &str) -> Vec<Event> {
        self.session
            .query("search", text, Default::default(), CancellationToken::new())
            .collect()
            .await
    }
}

/// Installs `fixtures/echo.py` as a long-running extension with the given extra manifest lines.
fn install(python: &str, extra: &str) -> Installed {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, dir.join("echo.py")).unwrap();
    let manifest = format!(
        "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
         [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = [\"e\"]\n\n\
         [runtime.script]\ncommand = [{python:?}, \"echo.py\", \"loop\"]\nlong_running = true\n{extra}\n"
    );
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    let loaded = load_extension(&dir).unwrap();
    let script = loaded.manifest.runtime.script.as_ref().unwrap();
    let session = Session::new("echo", dir, script);
    Installed { root, session }
}

fn titles(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| match event {
            Event::Items(items) => items.iter().map(|item| item.title.clone()).collect(),
            Event::Failed(_) => vec![],
        })
        .collect()
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

/// Waits (up to 20 s) until `check` is true.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_session_starts_on_first_use_and_serves_several_requests_on_one_process() {
    let python = python_or_skip!();
    let ext = install(python, "");
    assert_eq!(ext.starts(), 0, "the process must not start before the first request");
    assert_eq!(ext.titles("a").await, ["echo a", "again a", "last a"]);
    let pid = ext.pid();
    assert_eq!(ext.titles("b").await, ["echo b", "again b", "last b"]);
    assert_eq!(ext.titles("c").await, ["echo c", "again c", "last c"]);
    assert_eq!(ext.starts(), 1);
    assert_eq!(ext.pid(), pid);
}

#[tokio::test]
async fn request_ids_increase() {
    let python = python_or_skip!();
    let ext = install(python, "");
    assert_eq!(ext.titles("id").await, ["id 1"]);
    assert_eq!(ext.titles("id").await, ["id 2"]);
    assert_eq!(ext.titles("id").await, ["id 3"]);
}

#[tokio::test]
async fn interleaved_responses_reach_the_query_they_answer() {
    let python = python_or_skip!();
    let ext = install(python, "");
    // The slow query is sent first and answered last; the fast one must not see its answer.
    let (slow, fast) = tokio::join!(ext.titles("slow"), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        ext.titles("fast").await
    });
    assert_eq!(slow, ["slow answer"]);
    assert_eq!(fast, ["echo fast", "again fast", "last fast"]);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn a_superseded_query_sends_cancel_and_its_late_answer_is_dropped() {
    let python = python_or_skip!();
    let ext = install(python, "");
    let cancel = CancellationToken::new();
    let mut stream = ext.session.query("search", "late", Default::default(), cancel.clone());
    let waiting = tokio::spawn(async move { stream.next().await });
    // Wait until the script has the request, then supersede it.
    eventually("the request to arrive", || ext.read("requests.log").contains("\"late\"")).await;
    cancel.cancel();
    assert!(waiting.await.unwrap().is_none());
    eventually("the cancel to arrive", || ext.read("requests.log").contains("\"cancel\"")).await;
    let log = ext.read("requests.log");
    let cancel_line = log.lines().find(|line| line.contains("\"cancel\"")).unwrap();
    let message: serde_json::Value = serde_json::from_str(cancel_line).unwrap();
    assert_eq!(message["cancels"], 1);
    assert_eq!(message["id"], 2, "the cancel carries a fresh id");
    // The script answers the cancelled query anyway (after 300 ms); the next query must not see it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(ext.titles("next").await, ["echo next", "again next", "last next"]);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn a_cancelled_query_that_has_not_started_yields_nothing() {
    let python = python_or_skip!();
    let ext = install(python, "");
    let cancel = CancellationToken::new();
    cancel.cancel();
    let events: Vec<Event> =
        ext.session.query("search", "x", Default::default(), cancel).collect().await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn a_timeout_kills_the_process_and_the_next_request_starts_a_new_one() {
    let python = python_or_skip!();
    let ext = install(python, "timeout_ms = 500");
    let started = Instant::now();
    let events = ext.query("hang").await;
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(titles(&events), ["before-hang"]);
    assert_eq!(events.last(), Some(&Event::Failed(Failure::Timeout(Duration::from_millis(500)))));
    assert!(!process_exists(&ext.pid()), "the hanging process is still there");
    assert_eq!(ext.titles("again").await, ["echo again", "again again", "last again"]);
    assert_eq!(ext.starts(), 2);
}

#[tokio::test]
async fn a_custom_action_goes_through_the_same_process() {
    let python = python_or_skip!();
    let ext = install(python, "");
    assert_eq!(ext.titles("a").await.len(), 3);
    let toast = ext.session.action("search", "item-7", "star", CancellationToken::new()).await;
    assert_eq!(toast, Ok(Some("Starred item-7".to_owned())));
    let quiet = ext.session.action("search", "x", "quiet", CancellationToken::new()).await;
    assert_eq!(quiet, Ok(None));
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn an_action_that_hangs_times_out() {
    let python = python_or_skip!();
    let ext = install(python, "timeout_ms = 400");
    let outcome = ext.session.action("search", "x", "hang", CancellationToken::new()).await;
    assert_eq!(outcome, Err(Failure::Timeout(Duration::from_millis(400))));
}

#[tokio::test]
async fn a_session_that_is_only_used_for_actions_keeps_its_process() {
    let python = python_or_skip!();
    let ext = install(python, "");
    // Called from a runtime of its own that ends after the call, as the backend does.
    let session = ext.session.clone();
    let toast = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(session.action("search", "i", "star", CancellationToken::new()))
    })
    .join()
    .unwrap();
    assert_eq!(toast, Ok(Some("Starred i".to_owned())));
    let pid = ext.pid();
    assert_eq!(ext.titles("after").await.len(), 3);
    assert_eq!(ext.pid(), pid);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn shutdown_kills_the_process_and_later_requests_get_nothing() {
    let python = python_or_skip!();
    let ext = install(python, "");
    assert_eq!(ext.titles("a").await.len(), 3);
    let pid = ext.pid();
    ext.session.shutdown();
    eventually("the process to end", || !process_exists(&pid) || !cfg!(unix)).await;
    assert!(ext.query("b").await.is_empty());
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn a_script_that_breaks_the_protocol_fails_the_query_with_the_line() {
    let python = python_or_skip!();
    let ext = install(python, "");
    let events = ext.query("malformed").await;
    assert_eq!(titles(&events), ["before-garbage"]);
    let Some(Event::Failed(Failure::Protocol(message))) = events.last() else {
        panic!("{events:?}")
    };
    assert!(message.contains("this is not json"), "{message}");
}
