//! The long-running script protocol end to end: `fixtures/echo.py loop` is driven through a
//! `Session`, and through the `ScriptBackend` of an extension host and the query engine. The tests
//! skip when there is no Python.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use daimos_core::QueryEngine;
use daimos_core::provider::{Provider, QueryContext};
use daimos_core::query::Sink;
use daimos_ext::manifest::DEFAULT_IDLE_TIMEOUT_MS;
use daimos_ext::script::Event;
use daimos_ext::{
    Backend, ExtensionHost, Failure, Registry, ScriptBackend, ScriptRuntime, Session,
};
use futures::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
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
    install_idle(python, timeout_ms, DEFAULT_IDLE_TIMEOUT_MS)
}

fn install_idle(python: &str, timeout_ms: u64, idle_timeout_ms: u64) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, root.path().join("echo.py")).unwrap();
    let runtime = ScriptRuntime {
        command: vec![python.to_owned(), "echo.py".to_owned(), "loop".to_owned()],
        long_running: true,
        timeout_ms,
        idle_timeout_ms,
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
        idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
    };
    let session = Session::new("echo", PathBuf::from(dir.path()), &runtime);
    let events: Vec<_> =
        session.query("search", "x", Default::default(), CancellationToken::new()).collect().await;
    let [Event::Failed(Failure::Spawn(message))] = events.as_slice() else { panic!("{events:?}") };
    assert!(message.contains("daimos-no-such-program"), "{message}");
    let action = session.action("search", "i", "a", CancellationToken::new()).await;
    assert!(matches!(action, Err(Failure::Spawn(_))));
}

/// Crashes the process once and waits until the session knows.
async fn crash(ext: &Fixture) {
    let events = ext.ask("die").await;
    assert!(matches!(events.as_slice(), [Event::Failed(Failure::Exit { code: Some(4), .. })]));
}

#[tokio::test]
async fn a_crash_waits_for_the_backoff_before_the_restart() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    crash(&ext).await;
    let started = Instant::now();
    assert_eq!(titles(&ext.ask("back").await), ["echo back"]);
    assert!(started.elapsed() >= Duration::from_millis(250), "{:?}", started.elapsed());
    assert_eq!(ext.starts(), 2);
}

#[tokio::test]
async fn three_restarts_within_a_minute_work_and_the_fourth_crash_fails_the_session() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    for restart in 1..=3 {
        crash(&ext).await;
        assert_eq!(titles(&ext.ask("alive").await), ["echo alive"], "restart {restart}");
        assert_eq!(ext.starts(), restart + 1);
    }
    crash(&ext).await;

    let starts = ext.starts();
    for _ in 0..2 {
        let events = ext.ask("alive").await;
        assert_eq!(
            events,
            [Event::Failed(Failure::KeepsCrashing { stderr: Some("dying".to_owned()) })]
        );
    }
    assert!(ext.session.is_failed());
    let action = ext.session.action("search", "i", "star", CancellationToken::new()).await;
    assert!(matches!(action, Err(Failure::KeepsCrashing { .. })));
    assert_eq!(ext.starts(), starts, "a failed session must not spawn");
}

#[tokio::test]
async fn a_new_session_after_a_reload_is_not_failed() {
    let python = python_or_skip!();
    let ext = install(python, 10_000);
    for _ in 0..3 {
        crash(&ext).await;
        ext.ask("alive").await;
    }
    crash(&ext).await;
    assert!(matches!(
        ext.ask("x").await.as_slice(),
        [Event::Failed(Failure::KeepsCrashing { .. })]
    ));
    assert!(ext.session.is_failed());

    let reloaded = Session::new(
        "echo",
        ext.root.path().to_path_buf(),
        &ScriptRuntime {
            command: vec![python.to_owned(), "echo.py".to_owned(), "loop".to_owned()],
            long_running: true,
            timeout_ms: 10_000,
            idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
        },
    );
    let events: Vec<_> = reloaded
        .query("search", "fresh", Default::default(), CancellationToken::new())
        .collect()
        .await;
    assert_eq!(titles(&events), ["echo fresh"]);
    assert!(!reloaded.is_failed());
}

#[tokio::test]
async fn deliberate_stops_do_not_count_as_crashes() {
    let python = python_or_skip!();
    let ext = install(python, 300);
    for _ in 0..4 {
        let events = ext.ask("hang").await;
        assert!(matches!(events.as_slice(), [Event::Failed(Failure::Timeout(_))]), "{events:?}");
        ext.ask("ok").await;
        ext.session.shutdown().await;
    }
    assert!(!ext.session.is_failed());
    assert_eq!(titles(&ext.ask("still fine").await), ["echo still fine"]);
}

/// A real, short idle time: the session stops the process on its own and starts it again.
const IDLE_MS: u64 = 600;

async fn wait_until_gone(pid: &str) {
    let started = Instant::now();
    while process_exists(pid) {
        assert!(started.elapsed() < Duration::from_secs(10), "process {pid} still runs");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn an_idle_process_is_stopped_and_the_next_query_starts_it_again() {
    let python = python_or_skip!();
    let ext = install_idle(python, 10_000, IDLE_MS);
    assert_eq!(titles(&ext.ask("a").await), ["echo a"]);
    let pid = ext.pid();
    assert!(ext.session.is_running());

    wait_until_gone(&pid).await;
    assert!(!ext.session.is_running());
    assert_eq!(ext.starts(), 1);

    assert_eq!(titles(&ext.ask("b").await), ["echo b"]);
    assert_eq!(ext.starts(), 2, "the second query starts a new process");
    assert_ne!(ext.pid(), pid);
}

#[tokio::test]
async fn an_idle_stop_is_not_a_crash() {
    let python = python_or_skip!();
    let ext = install_idle(python, 10_000, 300);
    for _ in 0..5 {
        assert_eq!(titles(&ext.ask("again").await), ["echo again"]);
        wait_until_gone(&ext.pid()).await;
    }
    assert!(!ext.session.is_failed());
    // No backoff either: the sixth start is not delayed.
    let started = Instant::now();
    assert_eq!(titles(&ext.ask("last").await), ["echo last"]);
    assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());
    assert_eq!(ext.starts(), 6);
}

#[tokio::test]
async fn a_query_resets_the_idle_time() {
    let python = python_or_skip!();
    let ext = install_idle(python, 10_000, 1500);
    ext.ask("a").await;
    let pid = ext.pid();
    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(titles(&ext.ask("again").await), ["echo again"]);
    }
    // 2.4 s since the start, more than the idle time, but never idle for 1.5 s.
    assert!(process_exists(&pid));
    assert_eq!(ext.starts(), 1);
    wait_until_gone(&pid).await;
}

#[tokio::test]
async fn a_slow_request_keeps_the_process_beyond_the_idle_time() {
    let python = python_or_skip!();
    let ext = install_idle(python, 10_000, 200);
    // `slow` answers after 0.5 s: longer than the idle time, still one process.
    assert_eq!(titles(&ext.ask("slow").await), ["slow done"]);
    assert_eq!(ext.starts(), 1);
}

// Through the extension host, the `ScriptBackend` and the query engine.

/// `fixtures/echo.py loop` installed as the extension `echo` with keyword `e`, loaded by a host
/// over a real `ScriptBackend`.
struct Hosted {
    root: tempfile::TempDir,
    backend: Arc<ScriptBackend>,
    registry: Arc<Registry>,
    host: ExtensionHost,
}

fn hosted_manifest(python: &str, keyword: &str) -> String {
    format!(
        "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\n\
         runtime = \"script\"\n\n[[commands]]\nname = \"search\"\ntitle = \"Search\"\n\
         keywords = [\"{keyword}\"]\n\n[runtime.script]\n\
         command = [{python:?}, \"echo.py\", \"loop\"]\nlong_running = true\n"
    )
}

fn host(python: &str) -> Hosted {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, dir.join("echo.py")).unwrap();
    std::fs::write(dir.join("extension.toml"), hosted_manifest(python, "e")).unwrap();
    daimos_ext::consent::confirm(&dir).unwrap();
    let backend = Arc::new(ScriptBackend::new());
    let registry = Arc::new(Registry::new());
    let as_backend: Arc<dyn Backend> = backend.clone();
    let host = ExtensionHost::new(root.path().to_path_buf(), as_backend, Arc::clone(&registry));
    host.reload_all();
    Hosted { root, backend, registry, host }
}

impl Hosted {
    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.path().join("echo").join(name)).unwrap_or_default()
    }

    fn starts(&self) -> usize {
        self.read("starts.log").lines().count()
    }

    fn pid(&self) -> String {
        self.read("pid.txt")
    }

    /// The titles one query of the `search` command answers.
    async fn ask(&self, text: &str) -> Vec<String> {
        let providers = self.registry.providers();
        let [provider] = providers.as_slice() else { panic!("{} providers", providers.len()) };
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: true,
            cancel: CancellationToken::new(),
        };
        let batches: Vec<_> = provider.query(ctx).collect().await;
        batches.into_iter().flat_map(|b| b.items).map(|item| item.title).collect()
    }

    fn star(&self, item: &str) -> Result<Option<String>, String> {
        self.backend.custom_action("echo", "search", item, "star")
    }
}

#[tokio::test]
async fn a_long_running_extension_serves_queries_and_custom_actions_from_one_process() {
    let python = python_or_skip!();
    let ext = host(python);
    assert_eq!(ext.ask("a").await, ["echo a"]);
    let pid = ext.pid();
    // A custom action blocks this (runtime) thread; the session runs elsewhere, so no deadlock.
    assert_eq!(ext.star("echo a"), Ok(Some("Starred echo a".to_owned())));
    assert_eq!(ext.ask("pid").await, [pid]);
    assert_eq!(ext.starts(), 1);
    let types: Vec<_> = ext
        .read("requests.log")
        .lines()
        .map(|line| line.split(' ').nth(1).unwrap().to_owned())
        .collect();
    assert_eq!(types, ["query", "action", "query"]);
}

#[test]
fn a_session_outlives_the_runtime_that_started_it() {
    let python = python_or_skip!();
    let ext = host(python);
    let runtime = || tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    // The first query starts the process from a runtime that is gone afterwards.
    assert_eq!(runtime().block_on(ext.ask("a")), ["echo a"]);
    let pid = ext.pid();
    // An action from a thread without any runtime reaches the same process.
    let backend = Arc::clone(&ext.backend);
    let toast = std::thread::spawn(move || backend.custom_action("echo", "search", "i", "star"))
        .join()
        .unwrap();
    assert_eq!(toast, Ok(Some("Starred i".to_owned())));
    assert_eq!(runtime().block_on(ext.ask("pid")), [pid]);
    assert_eq!(ext.starts(), 1);
}

#[tokio::test]
async fn unload_kills_the_process_before_it_returns() {
    let python = python_or_skip!();
    let ext = host(python);
    assert_eq!(ext.ask("a").await, ["echo a"]);
    let pid = ext.pid();
    assert!(process_exists(&pid));
    ext.backend.unload("echo");
    assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
    assert_eq!(ext.star("i"), Err("echo is not loaded".to_owned()));
}

#[tokio::test]
async fn removing_the_extension_kills_its_process() {
    let python = python_or_skip!();
    let ext = host(python);
    assert_eq!(ext.ask("a").await, ["echo a"]);
    let pid = ext.pid();
    std::fs::remove_file(ext.root.path().join("echo").join("extension.toml")).unwrap();
    ext.host.reload_all();
    assert!(ext.registry.providers().is_empty());
    assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
}

#[tokio::test]
async fn a_manifest_edit_restarts_the_process() {
    let python = python_or_skip!();
    let ext = host(python);
    assert_eq!(ext.ask("a").await, ["echo a"]);
    let pid = ext.pid();
    let manifest = ext.root.path().join("echo").join("extension.toml");
    std::fs::write(&manifest, hosted_manifest(python, "echo")).unwrap();
    ext.host.reload("echo");
    assert!(!process_exists(&pid), "the old process {pid} is still there");
    let new_pid = ext.ask("pid").await;
    assert_ne!(new_pid, [pid]);
    assert_eq!(ext.starts(), 2);
    assert_eq!(ext.registry.providers()[0].keyword(), Some("echo"));
}

#[tokio::test]
async fn a_reload_clears_the_failed_state() {
    let python = python_or_skip!();
    let ext = host(python);
    for _ in 0..3 {
        assert_eq!(ext.ask("die").await, ["echo failed"]);
        assert_eq!(ext.ask("alive").await, ["echo alive"]);
    }
    assert_eq!(ext.ask("die").await, ["echo failed"]);
    assert_eq!(ext.ask("alive").await, ["echo keeps crashing"]);
    assert!(ext.star("i").unwrap_err().starts_with("echo: "));

    ext.host.reload("echo");
    assert_eq!(ext.ask("fresh").await, ["echo fresh"]);
    assert_eq!(ext.star("i"), Ok(Some("Starred i".to_owned())));
}

#[tokio::test]
async fn dropping_the_backend_kills_the_process() {
    let python = python_or_skip!();
    let ext = host(python);
    assert_eq!(ext.ask("a").await, ["echo a"]);
    let pid = ext.pid();
    let Hosted { root, backend, registry, host } = ext;
    // The providers and the host hold the backend too; the last one to go drops it.
    drop((registry, host));
    drop(backend);
    assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
    drop(root);
}

#[tokio::test]
async fn a_long_running_extension_answers_through_the_query_engine() {
    let python = python_or_skip!();
    let ext = host(python);
    // The engine gives a provider 100 ms per query; starting Python may take longer on a busy
    // machine, so the process is started before the engine asks.
    assert_eq!(ext.ask("warm").await, ["echo warm"]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink: Sink = Arc::new(move |batch| {
        let _ = tx.send(batch);
    });
    let source = Arc::clone(&ext.registry);
    let engine = QueryEngine::new(Vec::<Arc<dyn Provider>>::new(), sink, Handle::current())
        .with_dynamic(Arc::new(move || source.providers()));

    for (session, text) in [(1, "hello"), (2, "again")] {
        engine.query(session, &format!("e {text}"));
        let mut titles = Vec::new();
        loop {
            let batch =
                tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
            titles.extend(batch.items.into_iter().map(|item| item.title));
            if batch.done {
                break;
            }
        }
        assert_eq!(titles, [format!("echo {text}")]);
    }
    let outcome = engine.run_action("echo again", "star").unwrap().unwrap();
    assert_eq!(outcome.toast.as_deref(), Some("Starred echo again"));
    assert_eq!(ext.starts(), 1);
}
