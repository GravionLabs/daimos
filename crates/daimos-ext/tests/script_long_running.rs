//! The long-running script protocol end to end: the Python fixture runs in loop mode behind a
//! `Session`. The tests skip when there is no Python.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use daimos_ext::dir::load_extension;
use daimos_ext::script::Event;
use daimos_ext::{Failure, RestartPolicy, Session};
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

fn quick(max_restarts: usize, window_ms: u64) -> RestartPolicy {
    RestartPolicy {
        max_restarts,
        window: Duration::from_millis(window_ms),
        backoff: Duration::from_millis(20),
    }
}

fn exit_failure(events: &[Event]) -> Option<&Failure> {
    match events.last() {
        Some(Event::Failed(failure)) => Some(failure),
        _ => None,
    }
}

fn crashed(events: &[Event]) -> bool {
    matches!(
        exit_failure(events),
        Some(Failure::Exit { code: Some(2), stderr: Some(line) }) if line == "crashing on request"
    )
}

#[tokio::test]
async fn a_crash_fails_the_request_in_flight_and_the_next_one_restarts_the_process() {
    let python = python_or_skip!();
    let ext = install(python, "");
    assert_eq!(ext.titles("a").await.len(), 3);
    assert!(crashed(&ext.query("crash").await));
    let started = Instant::now();
    assert_eq!(ext.titles("b").await, ["echo b", "again b", "last b"]);
    // The first restart waits 250 ms after the crash (the python start is on top of it).
    assert!(started.elapsed() >= Duration::from_millis(150), "{:?}", started.elapsed());
    assert_eq!(ext.starts(), 2);
}

#[tokio::test]
async fn three_restarts_in_a_minute_work_and_the_fourth_crash_shows_one_error_item() {
    let python = python_or_skip!();
    let ext = install(python, "");
    for _ in 0..3 {
        assert!(crashed(&ext.query("crash").await));
    }
    // The third restart works.
    assert_eq!(ext.titles("ok").await.len(), 3);
    assert_eq!(ext.starts(), 4);
    // The fourth crash is not restarted.
    assert!(crashed(&ext.query("crash").await));
    let batches: Vec<_> = ext
        .session
        .query_batches("search", "ok", Default::default(), CancellationToken::new())
        .collect()
        .await;
    let [batch] = batches.as_slice() else { panic!("{batches:?}") };
    let [error] = batch.items.as_slice() else { panic!("{batch:?}") };
    assert_eq!(error.title, "echo keeps crashing");
    assert_eq!(error.subtitle.as_deref(), Some("crashing on request"));
    assert_eq!(ext.starts(), 4, "no process starts in the failed state");
    // Every query gets the same single error item.
    let again = ext.query("anything").await;
    assert_eq!(again, [Event::Failed(Failure::KeepsCrashing(Some("crashing on request".into())))]);
    assert_eq!(ext.starts(), 4);
}

#[tokio::test]
async fn the_window_slides_so_old_crashes_expire() {
    let python = python_or_skip!();
    let ext = install(python, "");
    let session = ext.session.clone().with_restart_policy(quick(1, 600));
    let ext = Installed { session, ..ext };
    assert!(crashed(&ext.query("crash").await));
    tokio::time::sleep(Duration::from_millis(700)).await;
    // One restart is allowed per 600 ms: the crash above has expired.
    assert!(crashed(&ext.query("crash").await));
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(ext.titles("ok").await.len(), 3);
    assert_eq!(ext.starts(), 3);
}

#[tokio::test]
async fn a_second_crash_in_the_window_fails_until_the_session_is_replaced() {
    let python = python_or_skip!();
    let ext = install(python, "");
    let session = ext.session.clone().with_restart_policy(quick(1, 60_000));
    let ext = Installed { session, ..ext };
    assert!(crashed(&ext.query("crash").await));
    assert!(crashed(&ext.query("crash").await));
    assert!(matches!(ext.query("ok").await.as_slice(), [Event::Failed(Failure::KeepsCrashing(_))]));
    // A reload builds a fresh session: no failed state.
    ext.session.shutdown();
    let loaded = load_extension(&ext.dir()).unwrap();
    let fresh = Session::new("echo", ext.dir(), loaded.manifest.runtime.script.as_ref().unwrap());
    let ext = Installed { session: fresh, ..ext };
    assert_eq!(ext.titles("ok").await.len(), 3);
}

#[tokio::test]
async fn deliberate_stops_do_not_count_as_crashes() {
    let python = python_or_skip!();
    let ext = install(python, "timeout_ms = 300");
    let session = ext.session.clone().with_restart_policy(quick(1, 60_000));
    let ext = Installed { session, ..ext };
    // Three timeout kills would fail a session that counted them (one restart is allowed).
    for _ in 0..3 {
        let events = ext.query("hang").await;
        assert!(matches!(exit_failure(&events), Some(Failure::Timeout(_))), "{events:?}");
    }
    assert_eq!(ext.titles("ok").await.len(), 3);
}

#[tokio::test]
async fn an_idle_process_is_stopped_and_the_next_query_starts_it_again() {
    let python = python_or_skip!();
    let ext = install(python, "idle_timeout_ms = 400");
    // No restarts allowed: an idle stop that counted as a crash would fail the session.
    let session = ext.session.clone().with_restart_policy(quick(0, 60_000));
    let ext = Installed { session, ..ext };
    for round in 1..=2 {
        assert_eq!(ext.titles("a").await.len(), 3);
        let pid = ext.pid();
        assert_eq!(ext.starts(), round);
        if cfg!(unix) {
            eventually("the idle process to end", || !process_exists(&pid)).await;
        } else {
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }
    assert_eq!(ext.titles("last").await.len(), 3);
    assert_eq!(ext.starts(), 3);
}

#[tokio::test]
async fn a_request_during_the_idle_time_resets_it() {
    let python = python_or_skip!();
    let ext = install(python, "idle_timeout_ms = 1500");
    assert_eq!(ext.titles("a").await.len(), 3);
    let pid = ext.pid();
    // Each wait is shorter than the idle time, together they are longer.
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(ext.titles("b").await.len(), 3);
    }
    assert_eq!(ext.starts(), 1);
    assert_eq!(ext.pid(), pid);
    if cfg!(unix) {
        assert!(process_exists(&pid));
        eventually("the idle process to end", || !process_exists(&pid)).await;
    }
}

#[tokio::test]
async fn a_request_in_flight_keeps_the_process_alive() {
    let python = python_or_skip!();
    let ext = install(python, "idle_timeout_ms = 100\ntimeout_ms = 20000");
    // The fixture needs 300 ms to answer, three times the idle time.
    assert_eq!(ext.titles("slow").await, ["slow answer"]);
    assert_eq!(ext.starts(), 1);
}

mod backend {
    use std::sync::Arc;

    use daimos_core::model::ItemBatch;
    use daimos_core::provider::QueryContext;
    use daimos_ext::{Backend, ExtensionHost, Registry, ScriptBackend};

    use super::*;

    /// A directory of extensions with the fixture installed as `echo`, loaded by a host that
    /// drives a `ScriptBackend`.
    struct Host {
        root: tempfile::TempDir,
        backend: Arc<ScriptBackend>,
        host: ExtensionHost,
    }

    impl Host {
        fn dir(&self) -> PathBuf {
            self.root.path().join("echo")
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.dir().join(name)).unwrap_or_default()
        }

        fn starts(&self) -> usize {
            self.read("runs.log").lines().count()
        }

        fn manifest(python: &str, keyword: &str, long_running: bool) -> String {
            format!(
                "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
                 [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = [\"{keyword}\"]\n\n\
                 [runtime.script]\ncommand = [{python:?}, \"echo.py\", \"loop\"]\nlong_running = {long_running}\n"
            )
        }

        fn rewrite(&self, python: &str, keyword: &str) {
            std::fs::write(
                self.dir().join("extension.toml"),
                Self::manifest(python, keyword, true),
            )
            .unwrap();
            self.host.reload("echo");
        }

        async fn titles(&self, text: &str) -> Vec<String> {
            let ctx = QueryContext {
                text: text.into(),
                keyword_routed: true,
                cancel: CancellationToken::new(),
            };
            let batches: Vec<ItemBatch> = self.backend.query("echo", "search", ctx).collect().await;
            batches
                .iter()
                .flat_map(|batch| batch.items.iter().map(|item| item.title.clone()))
                .collect()
        }
    }

    fn host(python: &str) -> Host {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("echo");
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
        std::fs::copy(fixture, dir.join("echo.py")).unwrap();
        std::fs::write(dir.join("extension.toml"), Host::manifest(python, "e", true)).unwrap();
        // The empty set of capabilities is confirmed, so the extension may run.
        std::fs::write(dir.join(".consent.json"), r#"{"version":"1.0.0","capabilities":{}}"#)
            .unwrap();
        let backend = Arc::new(ScriptBackend::with_data_dir(root.path().join("data")));
        let host = ExtensionHost::new(
            root.path().to_path_buf(),
            backend.clone() as Arc<dyn Backend>,
            Arc::new(Registry::new()),
        );
        host.reload_all();
        Host { root, backend, host }
    }

    #[tokio::test]
    async fn a_long_running_extension_loads_and_answers_queries_and_custom_actions() {
        let python = python_or_skip!();
        let host = host(python);
        assert_eq!(host.titles("a").await, ["echo a", "again a", "last a"]);
        assert_eq!(host.titles("b").await, ["echo b", "again b", "last b"]);
        assert_eq!(
            host.backend.custom_action("echo", "search", "item-7", "star"),
            Ok(Some("Starred item-7".to_owned()))
        );
        assert_eq!(host.starts(), 1, "queries and the action share one process");
    }

    #[tokio::test]
    async fn a_long_running_query_is_not_debounced() {
        let python = python_or_skip!();
        let host = host(python);
        assert_eq!(host.titles("warm").await.len(), 3);
        let started = Instant::now();
        assert_eq!(host.titles("again").await.len(), 3);
        // A warm query is far below the 150 ms that one-shot queries wait.
        assert!(started.elapsed() < Duration::from_millis(140), "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn unload_kills_the_process() {
        let python = python_or_skip!();
        let host = host(python);
        assert_eq!(host.titles("a").await.len(), 3);
        let pid = host.read("pid.txt");
        host.backend.unload("echo");
        if cfg!(unix) {
            eventually("the process to end", || !process_exists(&pid)).await;
        }
        assert!(host.titles("b").await.is_empty(), "an unloaded extension gets no answer");
        assert_eq!(host.starts(), 1);
    }

    #[tokio::test]
    async fn a_manifest_edit_restarts_the_process() {
        let python = python_or_skip!();
        let host = host(python);
        assert_eq!(host.titles("a").await.len(), 3);
        let pid = host.read("pid.txt");
        host.rewrite(python, "changed");
        if cfg!(unix) {
            eventually("the old process to end", || !process_exists(&pid)).await;
        }
        assert_eq!(host.titles("b").await.len(), 3);
        assert_eq!(host.starts(), 2);
        assert_ne!(host.read("pid.txt"), pid);
    }

    #[tokio::test]
    async fn a_reload_clears_the_failed_state() {
        let python = python_or_skip!();
        let host = host(python);
        // The default policy: the fourth crash in a minute fails the session.
        for _ in 0..4 {
            let _ = host.titles("crash").await;
        }
        let failed = host.titles("ok").await;
        assert_eq!(failed, ["echo keeps crashing"]);
        host.rewrite(python, "e");
        assert_eq!(host.titles("ok").await.len(), 3);
    }

    #[tokio::test]
    async fn one_shot_extensions_still_run_one_process_per_query() {
        let python = python_or_skip!();
        let host = host(python);
        let one_shot = Host::manifest(python, "e", false).replace(", \"loop\"", "");
        std::fs::write(host.dir().join("extension.toml"), one_shot).unwrap();
        host.host.reload("echo");
        assert_eq!(host.titles("a").await, ["echo a", "again a", "last a"]);
        assert_eq!(host.titles("b").await, ["echo b", "again b", "last b"]);
        assert_eq!(host.read("runs.log").lines().collect::<Vec<_>>(), ["query", "query"]);
    }
}
