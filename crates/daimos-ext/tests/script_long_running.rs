//! The long-running script protocol end to end: `fixtures/echo.py loop` is driven through a
//! `Session`. The tests skip when there is no Python.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use daimos_ext::script::Event;
use daimos_ext::{Failure, ScriptRuntime, Session};
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

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

struct Fixture {
    root: tempfile::TempDir,
    session: Session,
}

impl Fixture {
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.path().join(name)).unwrap_or_default()
    }

    fn starts(&self) -> usize {
        self.read("starts.log").lines().count()
    }

    fn pid(&self) -> String {
        self.read("pid.txt")
    }

    /// The ids of the requests the script has seen, in order.
    fn request_ids(&self) -> Vec<u64> {
        self.read("requests.log")
            .lines()
            .map(|line| line.split(' ').next().unwrap().parse().unwrap())
            .collect()
    }

    async fn ask(&self, text: &str) -> Vec<Event> {
        self.session
            .query("search", text, Default::default(), CancellationToken::new())
            .collect()
            .await
    }
}

fn install(python: &str, timeout_ms: u64) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, root.path().join("echo.py")).unwrap();
    let runtime = ScriptRuntime {
        command: vec![python.to_owned(), "echo.py".to_owned(), "loop".to_owned()],
        long_running: true,
        timeout_ms,
    };
    let session = Session::new("echo", root.path().to_path_buf(), &runtime);
    Fixture { root, session }
}

fn titles(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| match event {
            Event::Items(items) => items.iter().map(|i| i.title.clone()).collect(),
            Event::Failed(_) => vec![],
        })
        .collect()
}

/// Polls `check` until it holds, for at most ten seconds.
async fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// Whether `pid` still has a process table entry; a zombie counts. Only checked on Unix.
fn process_exists(pid: &str) -> bool {
    cfg!(unix)
        && Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

#[tokio::test]
async fn the_process_starts_on_the_first_request_and_serves_several() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    assert!(!ext.session.is_running());
    // Building the stream starts nothing.
    let stream = ext.session.query("search", "x", Default::default(), CancellationToken::new());
    assert_eq!(ext.starts(), 0);
    drop(stream);
    assert_eq!(ext.starts(), 0);

    assert_eq!(titles(&ext.ask("a").await), ["echo a"]);
    let pid = ext.pid();
    assert!(ext.session.is_running());
    assert_eq!(titles(&ext.ask("b").await), ["echo b"]);
    assert_eq!(titles(&ext.ask("pid").await), std::slice::from_ref(&pid));
    assert_eq!(ext.starts(), 1);
    assert_eq!(ext.pid(), pid);
}

#[tokio::test]
async fn request_ids_increase_across_queries_and_actions() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    ext.ask("a").await;
    let toast = ext.session.action("search", "item-1", "star", CancellationToken::new()).await;
    assert_eq!(toast, Ok(Some("Starred item-1".to_owned())));
    ext.ask("b").await;
    assert_eq!(ext.request_ids(), [1, 2, 3]);
}

#[tokio::test]
async fn interleaved_responses_reach_the_query_with_their_id() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    // `slow` is asked first and answered last.
    let slow = ext.session.query("search", "slow", Default::default(), CancellationToken::new());
    let fast = ext.session.query("search", "fast", Default::default(), CancellationToken::new());
    let slow = tokio::spawn(slow.collect::<Vec<_>>());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let fast = tokio::spawn(fast.collect::<Vec<_>>());
    let (slow, fast) = (slow.await.unwrap(), fast.await.unwrap());
    assert_eq!(titles(&fast), ["echo fast"]);
    assert_eq!(titles(&slow), ["slow done"]);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn a_superseded_query_sends_cancel_and_yields_nothing_more() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let cancel = CancellationToken::new();
    let mut events = ext.session.query("search", "partial", Default::default(), cancel.clone());
    let Some(Event::Items(first)) = events.next().await else { panic!("no first batch") };
    assert_eq!(first[0].title, "partial first");

    cancel.cancel();
    assert_eq!(events.next().await, None);
    // The script saw `cancel` with a fresh id for request 1 and stopped its work.
    assert!(eventually(|| ext.read("cancels.log") == "2 1\n").await, "{}", ext.read("cancels.log"));
    // The session still serves the next query on the same process.
    assert_eq!(titles(&ext.ask("next").await), ["echo next"]);
    assert_eq!(ext.starts(), 1);
    assert_eq!(ext.request_ids(), [1, 2, 3]);
}

#[tokio::test]
async fn late_answers_for_a_cancelled_query_are_dropped() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let cancel = CancellationToken::new();
    let stubborn = ext.session.query("search", "stubborn", Default::default(), cancel.clone());
    let stubborn = tokio::spawn(stubborn.collect::<Vec<_>>());
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancel.cancel();
    assert!(stubborn.await.unwrap().is_empty());
    assert!(eventually(|| ext.read("cancels.log") == "2 1\n").await);

    // The script answers request 1 anyway after 0.5 s; a query running meanwhile is unaffected.
    let events = ext.ask("slow").await;
    assert_eq!(titles(&events), ["slow done"]);
    assert_eq!(events.len(), 1);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn a_query_cancelled_before_it_starts_spawns_nothing() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let events: Vec<_> =
        ext.session.query("search", "x", Default::default(), cancel.clone()).collect().await;
    assert!(events.is_empty());
    assert_eq!(ext.session.action("search", "i", "star", cancel).await, Ok(None));
    assert_eq!(ext.starts(), 0);
    assert!(!ext.session.is_running());
}

#[tokio::test]
async fn a_timeout_fails_the_request_kills_the_process_and_the_next_request_starts_a_new_one() {
    let python = python_or_skip!();
    let ext = install(python, 400);
    assert_eq!(titles(&ext.ask("pid").await).len(), 1);
    let pid = ext.pid();

    let started = Instant::now();
    let events = ext.ask("hang").await;
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(events, [Event::Failed(Failure::Timeout(Duration::from_millis(400)))]);
    assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
    assert!(!ext.session.is_running());

    assert_eq!(titles(&ext.ask("again").await), ["echo again"]);
    assert_eq!(ext.starts(), 2);
    assert_ne!(ext.pid(), pid);
}

#[tokio::test]
async fn a_process_that_exits_fails_the_waiting_request_and_is_restarted_for_the_next() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let events = ext.ask("die").await;
    assert_eq!(
        events,
        [Event::Failed(Failure::Exit { code: Some(4), stderr: Some("dying".to_owned()) })]
    );
    assert_eq!(titles(&ext.ask("back").await), ["echo back"]);
    assert_eq!(ext.starts(), 2);
}

#[tokio::test]
async fn a_malformed_line_fails_the_waiting_request_with_a_protocol_failure() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let events = ext.ask("garbage").await;
    let [Event::Failed(Failure::Protocol(message))] = events.as_slice() else {
        panic!("{events:?}")
    };
    assert!(message.contains("not json"), "{message}");
    assert_eq!(titles(&ext.ask("ok").await), ["echo ok"]);
}

#[tokio::test]
async fn a_failure_reaches_every_request_waiting_on_the_process() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    let waiting = ext.session.query("search", "slow", Default::default(), CancellationToken::new());
    let waiting = tokio::spawn(waiting.collect::<Vec<_>>());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let dying = ext.ask("die").await;
    assert!(matches!(dying.as_slice(), [Event::Failed(Failure::Exit { code: Some(4), .. })]));
    let waiting = waiting.await.unwrap();
    assert!(matches!(waiting.as_slice(), [Event::Failed(Failure::Exit { .. })]), "{waiting:?}");
}

#[tokio::test]
async fn shutdown_kills_the_process_and_the_session_starts_again_on_demand() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    ext.ask("a").await;
    let pid = ext.pid();
    ext.session.shutdown().await;
    assert!(!ext.session.is_running());
    assert!(!process_exists(&pid));
    assert_eq!(titles(&ext.ask("b").await), ["echo b"]);
    assert_eq!(ext.starts(), 2);
}

#[tokio::test]
async fn a_program_that_does_not_exist_is_a_spawn_failure() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = ScriptRuntime {
        command: vec!["daimos-no-such-program".to_owned()],
        long_running: true,
        timeout_ms: 5000,
    };
    let session = Session::new("echo", PathBuf::from(dir.path()), &runtime);
    let events: Vec<_> =
        session.query("search", "x", Default::default(), CancellationToken::new()).collect().await;
    let [Event::Failed(Failure::Spawn(message))] = events.as_slice() else { panic!("{events:?}") };
    assert!(message.contains("daimos-no-such-program"), "{message}");
    let action = session.action("search", "i", "a", CancellationToken::new()).await;
    assert!(matches!(action, Err(Failure::Spawn(_))));
}
