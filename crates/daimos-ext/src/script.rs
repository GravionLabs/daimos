//! Running script extensions in one-shot mode: one process per query.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use daimos_core::model;
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::manifest::ScriptRuntime;
use crate::protocol::{self, Request, Response};

/// `CREATE_NO_WINDOW`: no console window flashes when the script starts on Windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long the end of a process waits for its stderr to drain.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// The id of the one request a one-shot process receives.
const REQUEST_ID: u64 = 1;

/// Why a script run did not end well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The process could not be started.
    Spawn(String),
    /// The process ran longer than the timeout and was killed.
    Timeout(Duration),
    /// The process exited with an error; carries the last non-empty line it wrote to stderr.
    Exit { code: Option<i32>, stderr: Option<String> },
    /// The process wrote something that is not a protocol message.
    Protocol(String),
}

/// What a running query reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A batch of items, in the order the script sent them.
    Items(Vec<model::Item>),
    /// The run failed; items sent before stay valid. Always the last event.
    Failed(Failure),
}

/// What the run of one process reports to its consumer.
enum Step {
    Message(Response),
    Failed(Failure),
}

/// Runs the script of an extension once per request.
#[derive(Debug, Clone)]
pub struct OneShot {
    dir: PathBuf,
    command: Vec<String>,
    timeout: Duration,
}

impl OneShot {
    /// A runner for `runtime` of the extension in `dir`; the process starts in `dir`.
    pub fn new(dir: PathBuf, runtime: &ScriptRuntime) -> Self {
        Self { dir, command: runtime.command.clone(), timeout: runtime.timeout() }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Starts the script for one query and streams what it answers. The process starts when the
    /// stream is first polled; it ends at `done: true`, at its exit, at the timeout (a
    /// [`Failure::Timeout`]) or when `cancel` fires (nothing more is yielded). The process is
    /// always killed and reaped before the stream ends.
    pub fn query(
        &self,
        command: &str,
        text: &str,
        preferences: BTreeMap<String, String>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Event> {
        let request = Request::Query {
            id: REQUEST_ID,
            command: command.to_owned(),
            text: text.to_owned(),
            preferences,
        };
        self.run(request, cancel, |response| matches!(response, Response::Items { done: true, .. }))
            .filter_map(|step| async move {
                match step {
                    Step::Failed(failure) => Some(Event::Failed(failure)),
                    Step::Message(Response::Items { id: REQUEST_ID, items, .. }) => {
                        let items: Vec<model::Item> = items.into_iter().map(Into::into).collect();
                        (!items.is_empty()).then_some(Event::Items(items))
                    }
                    Step::Message(_) => None,
                }
            })
            .boxed()
    }

    /// Streams the messages of one process. The stream ends right after a message for which `stop`
    /// is true.
    fn run(
        &self,
        request: Request,
        cancel: CancellationToken,
        stop: fn(&Response) -> bool,
    ) -> BoxStream<'static, Step> {
        let pending = Pending {
            spec: self.clone(),
            request,
            cancel,
            stop,
            deadline: Instant::now() + self.timeout,
        };
        stream::unfold(State::Start(Box::new(pending)), advance).boxed()
    }

    /// The program path: a relative path with a separator is relative to the extension directory
    /// on every platform.
    fn program(&self) -> PathBuf {
        let program = Path::new(&self.command[0]);
        if program.is_relative() && self.command[0].contains(['/', '\\']) {
            self.dir.join(program)
        } else {
            program.to_path_buf()
        }
    }

    fn spawn(&self) -> std::io::Result<Child> {
        let mut command = Command::new(self.program());
        command
            .args(&self.command[1..])
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        command.spawn()
    }
}

struct Pending {
    spec: OneShot,
    request: Request,
    cancel: CancellationToken,
    stop: fn(&Response) -> bool,
    deadline: Instant,
}

struct Running {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
    stderr: JoinHandle<Option<String>>,
    timeout: Duration,
    deadline: Instant,
    cancel: CancellationToken,
    stop: fn(&Response) -> bool,
    /// The last message ended the run: kill and reap on the next poll.
    stopping: bool,
}

enum State {
    Start(Box<Pending>),
    Running(Box<Running>),
    Done,
}

async fn advance(state: State) -> Option<(Step, State)> {
    match state {
        State::Done => None,
        State::Start(pending) => start(*pending).await,
        State::Running(run) => step(*run).await,
    }
}

async fn start(pending: Pending) -> Option<(Step, State)> {
    if pending.cancel.is_cancelled() {
        return None;
    }
    let mut child = match pending.spec.spawn() {
        Ok(child) => child,
        Err(error) => {
            let failure =
                Failure::Spawn(format!("cannot start {}: {error}", pending.spec.command[0]));
            return Some((Step::Failed(failure), State::Done));
        }
    };
    // A script that does not read its input closes the pipe early; that is not an error here.
    if let Some(mut stdin) = child.stdin.take() {
        let line = protocol::encode_line(&pending.request);
        let _ = stdin.write_all(line.as_bytes()).await;
        let _ = stdin.flush().await;
    }
    let stdout = child.stdout.take().map(BufReader::new)?;
    let stderr = child.stderr.take();
    let stderr = tokio::spawn(async move {
        let mut last = None;
        if let Some(stderr) = stderr {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    last = Some(line.trim().to_owned());
                }
            }
        }
        last
    });
    let run = Running {
        child,
        lines: stdout.lines(),
        stderr,
        timeout: pending.spec.timeout,
        deadline: pending.deadline,
        cancel: pending.cancel,
        stop: pending.stop,
        stopping: false,
    };
    step(run).await
}

impl Running {
    /// Kills the process and waits for it, so no zombie remains.
    async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }

    /// The last stderr line once the pipe has closed (or after a short grace period).
    async fn last_stderr(self) -> Option<String> {
        tokio::time::timeout(STDERR_GRACE, self.stderr).await.ok()?.ok()?
    }
}

async fn step(mut run: Running) -> Option<(Step, State)> {
    if run.stopping {
        run.kill().await;
        return None;
    }
    loop {
        tokio::select! {
            biased;
            () = run.cancel.cancelled() => {
                run.kill().await;
                return None;
            }
            () = tokio::time::sleep_until(run.deadline) => {
                run.kill().await;
                let failure = Failure::Timeout(run.timeout);
                return Some((Step::Failed(failure), State::Done));
            }
            line = run.lines.next_line() => match line {
                Ok(Some(line)) => match protocol::parse_line::<Response>(&line) {
                    Ok(None) => {}
                    Ok(Some(response)) => {
                        run.stopping = (run.stop)(&response);
                        return Some((Step::Message(response), State::Running(Box::new(run))));
                    }
                    Err(error) => {
                        run.kill().await;
                        return Some((Step::Failed(Failure::Protocol(error.to_string())), State::Done));
                    }
                },
                Ok(None) => return finish(run).await,
                Err(error) => {
                    run.kill().await;
                    let failure = Failure::Protocol(format!("cannot read the output: {error}"));
                    return Some((Step::Failed(failure), State::Done));
                }
            },
        }
    }
}

/// The output ended: wait for the exit (within the deadline) and report a bad status.
async fn finish(mut run: Running) -> Option<(Step, State)> {
    let waited = tokio::select! {
        biased;
        () = run.cancel.cancelled() => None,
        () = tokio::time::sleep_until(run.deadline) => Some(Err(())),
        status = run.child.wait() => Some(Ok(status)),
    };
    match waited {
        None => {
            run.kill().await;
            None
        }
        Some(Err(())) => {
            run.kill().await;
            let failure = Failure::Timeout(run.timeout);
            Some((Step::Failed(failure), State::Done))
        }
        Some(Ok(Ok(status))) if status.success() => None,
        Some(Ok(status)) => {
            let code = status.ok().and_then(|status| status.code());
            let stderr = run.last_stderr().await;
            Some((Step::Failed(Failure::Exit { code, stderr }), State::Done))
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::Path;
    use std::process::Command;
    use std::sync::OnceLock;

    /// The Python interpreter of this machine, resolved once; `None` when there is none.
    pub(crate) fn python() -> Option<&'static str> {
        static PYTHON: OnceLock<Option<&'static str>> = OnceLock::new();
        *PYTHON.get_or_init(|| {
            let candidates: &[&str] =
                if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
            candidates.iter().copied().find(|name| {
                Command::new(name)
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success())
            })
        })
    }

    /// Writes `source` as `main.py` into `dir`.
    pub(crate) fn write_script(dir: &Path, source: &str) {
        std::fs::write(dir.join("main.py"), source).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant as StdInstant;

    use super::testutil::{python, write_script};
    use super::*;

    /// Skips the test cleanly when this machine has no Python.
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

    const PRELUDE: &str = "import json, sys, time, os\n\
        request = json.loads(sys.stdin.readline())\n\
        def send(message):\n    print(json.dumps(message), flush=True)\n\
        def items(*ids, done=False):\n    send({'id': request['id'], 'type': 'items', \
        'items': [{'id': i, 'title': i.upper()} for i in ids], 'done': done})\n";

    fn runner(dir: &Path, python: &str, timeout_ms: u64) -> OneShot {
        OneShot {
            dir: dir.to_path_buf(),
            command: vec![python.to_owned(), "main.py".to_owned()],
            timeout: Duration::from_millis(timeout_ms),
        }
    }

    async fn collect(shot: &OneShot, text: &str) -> Vec<Event> {
        shot.query("search", text, BTreeMap::new(), CancellationToken::new()).collect().await
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

    #[tokio::test]
    async fn items_stream_out_in_order_and_done_ends_the_query_early() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        // The tail would run for a minute if done did not end the process.
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}items('a', 'b')\nitems('c')\nitems('d', done=True)\ntime.sleep(60)\n"
            ),
        );
        let started = StdInstant::now();
        let events = collect(&runner(dir.path(), python, 30_000), "x").await;
        assert_eq!(titles(&events), ["A", "B", "C", "D"]);
        assert_eq!(events.len(), 3);
        assert!(started.elapsed() < Duration::from_secs(20), "done did not end the query");
    }

    #[tokio::test]
    async fn the_request_has_the_command_text_and_preferences_and_the_cwd_is_the_extension() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}sys.stdin.read()\n\
                 send({{'id': request['id'], 'type': 'items', 'done': True, 'items': [{{'id': 'r', \
                 'title': request['type'] + ':' + request['command'] + ':' + request['text'], \
                 'subtitle': json.dumps(request['preferences']) + '|' + os.getcwd()}}]}})\n"
            ),
        );
        let shot = runner(dir.path(), python, 10_000);
        let preferences = BTreeMap::from([("token".to_owned(), "t".to_owned())]);
        let events: Vec<_> =
            shot.query("search", "lau", preferences, CancellationToken::new()).collect().await;
        let [Event::Items(items)] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(items[0].title, "query:search:lau");
        let (preferences, cwd) = items[0].subtitle.as_deref().unwrap().split_once('|').unwrap();
        assert_eq!(preferences, r#"{"token": "t"}"#);
        assert_eq!(std::fs::canonicalize(cwd).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
    }

    #[tokio::test]
    async fn blank_lines_and_other_request_ids_are_ignored() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}print('', flush=True)\n\
                 send({{'id': 99, 'type': 'items', 'items': [{{'id': 'z', 'title': 'Z'}}]}})\n\
                 items('a')\n"
            ),
        );
        let events = collect(&runner(dir.path(), python, 10_000), "").await;
        assert_eq!(titles(&events), ["A"]);
    }

    #[tokio::test]
    async fn a_script_that_ends_without_done_ends_the_query_at_exit() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), &format!("{PRELUDE}items('a')\n"));
        let events = collect(&runner(dir.path(), python, 10_000), "").await;
        assert_eq!(events, [Event::Items(vec![model::Item::new("a", "A")])]);
    }

    #[tokio::test]
    async fn a_hanging_process_is_killed_at_the_timeout_and_items_before_it_stay() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), &format!("{PRELUDE}items('a')\ntime.sleep(60)\n"));
        let started = StdInstant::now();
        let events = collect(&runner(dir.path(), python, 400), "").await;
        assert!(started.elapsed() < Duration::from_secs(20));
        assert_eq!(titles(&events), ["A"]);
        assert_eq!(
            events.last(),
            Some(&Event::Failed(Failure::Timeout(Duration::from_millis(400))))
        );
    }

    #[tokio::test]
    async fn a_non_zero_exit_is_a_failure_with_the_code_and_the_last_stderr_line() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}items('a')\nsys.stderr.write('first\\n\\nboom happened\\n\\n')\nsys.exit(3)\n"
            ),
        );
        let events = collect(&runner(dir.path(), python, 10_000), "").await;
        assert_eq!(titles(&events), ["A"]);
        assert_eq!(
            events.last(),
            Some(&Event::Failed(Failure::Exit {
                code: Some(3),
                stderr: Some("boom happened".into())
            }))
        );
    }

    #[tokio::test]
    async fn a_malformed_line_ends_the_run_with_a_protocol_failure() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!("{PRELUDE}items('a')\nprint('not json', flush=True)\nitems('b')\n"),
        );
        let events = collect(&runner(dir.path(), python, 10_000), "").await;
        assert_eq!(titles(&events), ["A"]);
        let Some(Event::Failed(Failure::Protocol(message))) = events.last() else {
            panic!("{events:?}")
        };
        assert!(message.contains("not json"), "{message}");
    }

    #[tokio::test]
    async fn a_program_that_does_not_exist_is_a_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let shot = OneShot {
            dir: dir.path().to_path_buf(),
            command: vec!["daimos-no-such-program".into()],
            timeout: Duration::from_secs(5),
        };
        let events = collect(&shot, "").await;
        let [Event::Failed(Failure::Spawn(message))] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert!(message.contains("daimos-no-such-program"), "{message}");
    }

    #[tokio::test]
    async fn cancel_stops_the_stream_and_kills_the_process() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}open('pid', 'w').write(str(os.getpid()))\nitems('a')\ntime.sleep(60)\n"
            ),
        );
        let shot = runner(dir.path(), python, 60_000);
        let cancel = CancellationToken::new();
        let mut events = shot.query("search", "", BTreeMap::new(), cancel.clone());
        assert!(matches!(events.next().await, Some(Event::Items(_))));
        let started = StdInstant::now();
        cancel.cancel();
        assert_eq!(events.next().await, None);
        assert!(started.elapsed() < Duration::from_secs(20));
        let pid = std::fs::read_to_string(dir.path().join("pid")).unwrap();
        assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
    }

    #[tokio::test]
    async fn a_query_cancelled_before_it_starts_spawns_nothing() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), &format!("{PRELUDE}open('ran', 'w')\nitems('a', done=True)\n"));
        let cancel = CancellationToken::new();
        cancel.cancel();
        let shot = runner(dir.path(), python, 10_000);
        let events: Vec<_> = shot.query("search", "", BTreeMap::new(), cancel).collect().await;
        assert!(events.is_empty());
        assert!(!dir.path().join("ran").exists());
    }

    #[tokio::test]
    async fn a_finished_query_leaves_no_zombie() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}open('pid', 'w').write(str(os.getpid()))\nitems('a', done=True)\ntime.sleep(60)\n"
            ),
        );
        collect(&runner(dir.path(), python, 60_000), "").await;
        let pid = std::fs::read_to_string(dir.path().join("pid")).unwrap();
        assert!(!process_exists(&pid), "process {pid} is still there (or a zombie)");
    }

    /// Whether `pid` still has a process table entry; a zombie counts. Only checked on Unix.
    fn process_exists(pid: &str) -> bool {
        if cfg!(unix) {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        } else {
            false
        }
    }

    #[test]
    fn a_relative_program_with_a_separator_is_relative_to_the_extension() {
        let shot = OneShot {
            dir: PathBuf::from("ext"),
            command: vec!["bin/run".into()],
            timeout: Duration::from_secs(1),
        };
        assert_eq!(shot.program(), Path::new("ext").join("bin/run"));
        let shot = OneShot { command: vec!["python3".into()], ..shot };
        assert_eq!(shot.program(), Path::new("python3"));
    }

    #[test]
    fn the_runner_takes_its_timeout_from_the_manifest() {
        let runtime =
            ScriptRuntime { command: vec!["x".into()], long_running: false, timeout_ms: 1234 };
        let shot = OneShot::new(PathBuf::from("."), &runtime);
        assert_eq!(shot.timeout(), Duration::from_millis(1234));
    }
}
