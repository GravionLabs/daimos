//! Running script extensions in one-shot mode: one process per query.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use daimos_core::model::{self, ItemBatch};
use daimos_core::provider::QueryContext;
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::dir::{self, LoadedExtension};
use crate::manifest::{Manifest, Runtime, ScriptRuntime};
use crate::preferences::Preferences;
use crate::protocol::{self, Request, Response};
use crate::provider::Backend;

/// `CREATE_NO_WINDOW`: no console window flashes when the script starts on Windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How long the end of a process waits for its stderr to drain.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// A log file larger than this is moved to `<name>.log.1` before the next run appends to it.
const LOG_LIMIT: u64 = 1024 * 1024;

/// Queries are debounced: the script starts this long after the last keystroke.
pub const DEBOUNCE: Duration = Duration::from_millis(150);

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

impl Failure {
    /// What the user reads under the error item.
    pub fn message(&self) -> String {
        match self {
            Self::Spawn(message) | Self::Protocol(message) => message.clone(),
            Self::Timeout(limit) => format!("timed out after {}", format_duration(*limit)),
            Self::Exit { stderr: Some(line), .. } => line.clone(),
            Self::Exit { code: Some(code), .. } => format!("exited with code {code}"),
            Self::Exit { code: None, .. } => "was stopped before it finished".to_owned(),
        }
    }
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_millis() == 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

/// What the run of one process reports to its consumer.
enum Step {
    Message(Response),
    Failed(Failure),
}

/// Runs the script of an extension once per request.
#[derive(Debug, Clone)]
pub struct OneShot {
    extension: String,
    dir: PathBuf,
    command: Vec<String>,
    timeout: Duration,
    /// Where stderr is appended; `None` drops it.
    log: Option<PathBuf>,
}

impl OneShot {
    /// A runner for `runtime` of the extension called `extension` in `dir`; the process starts in
    /// `dir`.
    pub fn new(extension: &str, dir: PathBuf, runtime: &ScriptRuntime) -> Self {
        Self {
            extension: extension.to_owned(),
            dir,
            command: runtime.command.clone(),
            timeout: runtime.timeout(),
            log: None,
        }
    }

    /// Appends the stderr of every run to `path` (`<data dir>/logs/<extension>.log`).
    #[must_use]
    pub fn with_log(mut self, path: PathBuf) -> Self {
        self.log = Some(path);
        self
    }

    /// Like [`query`](Self::query), as batches for a provider: a failure becomes one error item
    /// after the items that came before it.
    pub fn query_batches(
        &self,
        command: &str,
        text: &str,
        preferences: BTreeMap<String, String>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, ItemBatch> {
        let extension = self.extension.clone();
        self.query(command, text, preferences, cancel)
            .map(move |event| match event {
                Event::Items(items) => ItemBatch::from(items),
                Event::Failed(failure) => ItemBatch::from(vec![failure_item(&extension, &failure)]),
            })
            .boxed()
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

    /// Starts the script for a custom action and waits for its `result`. The answer is the toast,
    /// if the script sent one. A script that exits without a result succeeded without a toast.
    /// The same timeout and cancel rules as for a query apply.
    pub async fn action(
        &self,
        command: &str,
        item: &str,
        action: &str,
        cancel: CancellationToken,
    ) -> Result<Option<String>, Failure> {
        let request = Request::Action {
            id: REQUEST_ID,
            command: command.to_owned(),
            item: item.to_owned(),
            action: action.to_owned(),
        };
        let mut steps =
            self.run(request, cancel, |response| matches!(response, Response::Result { .. }));
        let mut toast = None;
        while let Some(step) = steps.next().await {
            match step {
                Step::Failed(failure) => return Err(failure),
                Step::Message(Response::Result { id: REQUEST_ID, toast: sent, .. }) => toast = sent,
                Step::Message(_) => {}
            }
        }
        Ok(toast)
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
    stderr: Option<JoinHandle<Option<String>>>,
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
    let log = pending.spec.log.clone();
    let stderr = tokio::spawn(async move {
        match stderr {
            Some(stderr) => pump_stderr(stderr, log).await,
            None => None,
        }
    });
    let run = Running {
        child,
        lines: stdout.lines(),
        stderr: Some(stderr),
        timeout: pending.spec.timeout,
        deadline: pending.deadline,
        cancel: pending.cancel,
        stop: pending.stop,
        stopping: false,
    };
    step(run).await
}

/// Runs the one-shot script extensions (`runtime = "script"`, `long_running = false`). Queries of
/// other extensions (wasm, long-running scripts) get no answer until their hosts exist.
#[derive(Default)]
pub struct ScriptBackend {
    /// By extension name.
    runners: RwLock<BTreeMap<String, OneShot>>,
    /// Where `logs/<extension>.log` goes; no logging without it.
    data_dir: Option<PathBuf>,
    /// The manifests of the loaded extensions, for their preferences.
    manifests: RwLock<BTreeMap<String, Manifest>>,
    /// Where the preference values come from; queries carry none without it.
    preferences: Option<Arc<Preferences>>,
}

impl ScriptBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// A backend that writes the stderr of scripts to `<data_dir>/logs/<extension>.log`.
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self { data_dir: Some(data_dir), ..Self::default() }
    }

    /// Fills the `preferences` of every query from `preferences`, passwords read from the keyring
    /// at call time.
    #[must_use]
    pub fn with_preferences(mut self, preferences: Arc<Preferences>) -> Self {
        self.preferences = Some(preferences);
        self
    }

    fn runner(&self, extension: &str) -> Option<OneShot> {
        self.runners.read().unwrap_or_else(|e| e.into_inner()).get(extension).cloned()
    }
}

impl Backend for ScriptBackend {
    fn query(
        &self,
        extension: &str,
        command: &str,
        ctx: QueryContext,
    ) -> BoxStream<'static, ItemBatch> {
        let Some(runner) = self.runner(extension) else {
            return stream::empty().boxed();
        };
        let extension = extension.to_owned();
        let manifest =
            self.manifests.read().unwrap_or_else(|e| e.into_inner()).get(&extension).cloned();
        let store = self.preferences.clone();
        let command = command.to_owned();
        let QueryContext { text, cancel, .. } = ctx;
        // A newer keystroke cancels this query during the delay, so no process starts for it.
        let wait = {
            let cancel = cancel.clone();
            async move {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => false,
                    () = tokio::time::sleep(DEBOUNCE) => true,
                }
            }
        };
        stream::once(async move {
            if !wait.await {
                return stream::empty().boxed();
            }
            let preferences = match (store, manifest) {
                (Some(store), Some(manifest)) if !manifest.preferences.is_empty() => {
                    // The keyring can block, so it is read off the async threads.
                    match tokio::task::spawn_blocking(move || store.resolve(&manifest)).await {
                        Ok(Ok(values)) => values,
                        Ok(Err(error)) => {
                            let item = crate::provider::error_item(
                                format!("{extension}.preferences"),
                                format!("{extension} needs its preferences"),
                                &error.to_string(),
                                &extension,
                            );
                            return stream::iter([ItemBatch::from(vec![item])]).boxed();
                        }
                        Err(_) => BTreeMap::new(),
                    }
                }
                _ => BTreeMap::new(),
            };
            runner.query_batches(&command, &text, preferences, cancel)
        })
        .flatten()
        .boxed()
    }

    fn custom_action(
        &self,
        extension: &str,
        command: &str,
        item: &str,
        action: &str,
    ) -> Result<Option<String>, String> {
        let runner = self.runner(extension).ok_or_else(|| format!("{extension} is not loaded"))?;
        let (command, item, action) = (command.to_owned(), item.to_owned(), action.to_owned());
        // A thread of its own with its own runtime: this call blocks, and may be made from a
        // thread that has no runtime or from one that must not block one.
        let outcome = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime
                .block_on(runner.action(&command, &item, &action, CancellationToken::new()))
                .map_err(|failure| format!("{}: {}", runner.extension, failure.message()))
        })
        .join();
        outcome.unwrap_or_else(|_| Err(format!("{extension} stopped unexpectedly")))
    }

    fn load(&self, extension: &LoadedExtension) {
        let manifest = &extension.manifest;
        let name = &manifest.extension.name;
        self.manifests
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), manifest.clone());
        let mut runners = self.runners.write().unwrap_or_else(|e| e.into_inner());
        runners.remove(name);
        let script = match (&manifest.runtime.script, manifest.extension.runtime) {
            (Some(script), Runtime::Script) if !script.long_running => script,
            _ => return,
        };
        let mut runner = OneShot::new(name, extension.dir.clone(), script);
        if let Some(data_dir) = &self.data_dir {
            runner = runner.with_log(dir::log_path_in(data_dir, name));
        }
        runners.insert(name.clone(), runner);
    }

    fn unload(&self, name: &str) {
        self.runners.write().unwrap_or_else(|e| e.into_inner()).remove(name);
        self.manifests.write().unwrap_or_else(|e| e.into_inner()).remove(name);
    }
}

/// The error item of a failed run.
pub fn failure_item(extension: &str, failure: &Failure) -> model::Item {
    let title = match failure {
        Failure::Timeout(_) => format!("{extension} took too long"),
        _ => format!("{extension} failed"),
    };
    crate::provider::error_item(
        format!("{extension}.script-error"),
        title,
        &failure.message(),
        extension,
    )
}

/// Appends the lines of `stderr` to the log (when there is one) and returns the last non-empty
/// line. A log that cannot be written is given up on; the run does not notice.
async fn pump_stderr(stderr: impl AsyncRead + Unpin, log: Option<PathBuf>) -> Option<String> {
    let mut file = match log {
        Some(path) => open_log(&path).await,
        None => None,
    };
    let mut last = None;
    let mut lines = BufReader::new(stderr).lines();
    let mut written = 0_u64;
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(open) = file.as_mut() {
            written += line.len() as u64 + 1;
            // One run cannot grow the log past the limit, however much it writes.
            if written > LOG_LIMIT || open.write_all(format!("{line}\n").as_bytes()).await.is_err()
            {
                file = None;
            }
        }
        if !line.trim().is_empty() {
            last = Some(line.trim().to_owned());
        }
    }
    last
}

/// Opens the log for appending, creating its folder and moving an oversized log aside.
async fn open_log(path: &Path) -> Option<tokio::fs::File> {
    let result = async {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if tokio::fs::metadata(path).await.is_ok_and(|meta| meta.len() > LOG_LIMIT) {
            let mut old = path.as_os_str().to_owned();
            old.push(".1");
            tokio::fs::rename(path, old).await?;
        }
        tokio::fs::OpenOptions::new().create(true).append(true).open(path).await
    }
    .await;
    result.map_err(|error| log::warn!("cannot write {}: {error}", path.display())).ok()
}

impl Running {
    /// Kills the process and waits for it, so no zombie remains, then lets the stderr pump finish
    /// so the log is complete when the stream ends.
    async fn kill(&mut self) {
        let _ = self.child.kill().await;
        self.last_stderr().await;
    }

    /// The last stderr line once the pipe has closed (or after a short grace period).
    async fn last_stderr(&mut self) -> Option<String> {
        let handle = self.stderr.take()?;
        tokio::time::timeout(STDERR_GRACE, handle).await.ok()?.ok()?
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
    use std::sync::Arc;
    use std::time::Instant as StdInstant;

    use daimos_core::provider::Provider;

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
            extension: "demo".into(),
            dir: dir.to_path_buf(),
            command: vec![python.to_owned(), "main.py".to_owned()],
            timeout: Duration::from_millis(timeout_ms),
            log: None,
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
            extension: "demo".into(),
            dir: dir.path().to_path_buf(),
            command: vec!["daimos-no-such-program".into()],
            timeout: Duration::from_secs(5),
            log: None,
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
            extension: "demo".into(),
            dir: PathBuf::from("ext"),
            command: vec!["bin/run".into()],
            timeout: Duration::from_secs(1),
            log: None,
        };
        assert_eq!(shot.program(), Path::new("ext").join("bin/run"));
        let shot = OneShot { command: vec!["python3".into()], ..shot };
        assert_eq!(shot.program(), Path::new("python3"));
    }

    #[test]
    fn the_runner_takes_its_timeout_from_the_manifest() {
        let runtime =
            ScriptRuntime { command: vec!["x".into()], long_running: false, timeout_ms: 1234 };
        let shot = OneShot::new("demo", PathBuf::from("."), &runtime);
        assert_eq!(shot.timeout(), Duration::from_millis(1234));
    }

    async fn batches(shot: &OneShot) -> Vec<ItemBatch> {
        shot.query_batches("search", "", BTreeMap::new(), CancellationToken::new()).collect().await
    }

    #[tokio::test]
    async fn stderr_lines_land_in_the_log_file_across_runs() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}sys.stderr.write('hello ' + request['text'] + '\\n')\nitems('a', done=True)\n"
            ),
        );
        let log = dir.path().join("data").join("logs").join("demo.log");
        let shot = runner(dir.path(), python, 10_000).with_log(log.clone());
        shot.query("search", "one", BTreeMap::new(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;
        shot.query("search", "two", BTreeMap::new(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(std::fs::read_to_string(log).unwrap(), "hello one\nhello two\n");
    }

    #[tokio::test]
    async fn an_oversized_log_is_moved_aside_before_appending() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!("{PRELUDE}sys.stderr.write('fresh\\n')\nitems('a', done=True)\n"),
        );
        let log = dir.path().join("demo.log");
        std::fs::write(&log, vec![b'x'; LOG_LIMIT as usize + 1]).unwrap();
        let shot = runner(dir.path(), python, 10_000).with_log(log.clone());
        shot.query("search", "", BTreeMap::new(), CancellationToken::new())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "fresh\n");
        assert_eq!(std::fs::metadata(dir.path().join("demo.log.1")).unwrap().len(), LOG_LIMIT + 1);
    }

    #[tokio::test]
    async fn a_log_that_cannot_be_written_does_not_fail_the_query() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!("{PRELUDE}sys.stderr.write('x\\n')\nitems('a', done=True)\n"),
        );
        // The parent of the log is a file, so the folder cannot be created.
        std::fs::write(dir.path().join("blocked"), "").unwrap();
        let shot = runner(dir.path(), python, 10_000).with_log(dir.path().join("blocked/demo.log"));
        let events = collect(&shot, "").await;
        assert_eq!(titles(&events), ["A"]);
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn a_non_zero_exit_yields_exactly_one_error_item_with_the_last_stderr_line() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(
            dir.path(),
            &format!(
                "{PRELUDE}items('a')\nsys.stderr.write('warming up\\nkaboom\\n')\nsys.exit(1)\n"
            ),
        );
        let batches = batches(&runner(dir.path(), python, 10_000)).await;
        assert_eq!(batches.len(), 2, "the streamed item stays, then one error");
        assert_eq!(batches[0].items[0].title, "A");
        let [error] = batches[1].items.as_slice() else { panic!("{batches:?}") };
        assert_eq!(error.title, "demo failed");
        assert_eq!(error.subtitle.as_deref(), Some("kaboom"));
        assert_eq!(error.accessories, ["demo"]);
    }

    #[tokio::test]
    async fn a_timeout_yields_exactly_one_timeout_error_item() {
        let python = python_or_skip!();
        let dir = tempfile::tempdir().unwrap();
        write_script(dir.path(), &format!("{PRELUDE}time.sleep(60)\n"));
        let batches = batches(&runner(dir.path(), python, 300)).await;
        let [batch] = batches.as_slice() else { panic!("{batches:?}") };
        let [error] = batch.items.as_slice() else { panic!("{batch:?}") };
        assert_eq!(error.title, "demo took too long");
        assert_eq!(error.subtitle.as_deref(), Some("timed out after 300 ms"));
    }

    #[test]
    fn failures_have_a_readable_message() {
        let exit =
            |code, stderr: Option<&str>| Failure::Exit { code, stderr: stderr.map(Into::into) };
        assert_eq!(Failure::Timeout(Duration::from_secs(5)).message(), "timed out after 5 s");
        assert_eq!(exit(Some(2), Some("bad")).message(), "bad");
        assert_eq!(exit(Some(2), None).message(), "exited with code 2");
        assert_eq!(exit(None, None).message(), "was stopped before it finished");
        assert_eq!(Failure::Spawn("no".into()).message(), "no");
    }

    /// An extension directory with `main.py` and a manifest; returns the loaded extension.
    fn install(root: &Path, python: &str, long_running: bool, source: &str) -> LoadedExtension {
        let dir = root.join("demo");
        std::fs::create_dir_all(&dir).unwrap();
        write_script(&dir, source);
        let manifest = format!(
            "[extension]\nname = \"demo\"\ntitle = \"Demo\"\nversion = \"1.0.0\"\n\
             runtime = \"script\"\n\n[[commands]]\nname = \"search\"\ntitle = \"Search\"\n\
             keywords = [\"d\"]\n\n[runtime.script]\ncommand = [{python:?}, \"main.py\"]\n\
             long_running = {long_running}\n"
        );
        std::fs::write(dir.join("extension.toml"), manifest).unwrap();
        crate::consent::confirm(&dir).unwrap();
        dir::load_extension(&dir).unwrap()
    }

    fn query_of(
        backend: &ScriptBackend,
        text: &str,
        cancel: &CancellationToken,
    ) -> BoxStream<'static, ItemBatch> {
        let ctx = QueryContext { text: text.into(), keyword_routed: true, cancel: cancel.clone() };
        backend.query("demo", "search", ctx)
    }

    /// Counts runs by appending a line to `runs` and answers with the text.
    fn counting_script() -> String {
        format!(
            "{PRELUDE}open('runs', 'a').write('x\\n')\n\
             send({{'id': request['id'], 'type': 'items', 'done': True, \
             'items': [{{'id': 't', 'title': request['text']}}]}})\n"
        )
    }

    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("demo").join("runs"))
            .map_or(0, |text| text.lines().count())
    }

    #[tokio::test(start_paused = true)]
    async fn queries_cancelled_during_the_debounce_never_start_a_process() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = ScriptBackend::new();
        backend.load(&install(root.path(), python, false, &counting_script()));
        let mut handles = Vec::new();
        for text in ["a", "ab", "abc"] {
            let cancel = CancellationToken::new();
            let stream = query_of(&backend, text, &cancel);
            handles.push(tokio::spawn(async move { stream.collect::<Vec<_>>().await }));
            tokio::time::sleep(Duration::from_millis(20)).await;
            // The next keystroke replaces this query.
            cancel.cancel();
        }
        for handle in handles {
            assert!(handle.await.unwrap().is_empty());
        }
        assert_eq!(runs(root.path()), 0);
    }

    #[tokio::test]
    async fn rapid_keystrokes_spawn_at_most_one_process() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = ScriptBackend::new();
        backend.load(&install(root.path(), python, false, &counting_script()));
        let mut previous: Option<CancellationToken> = None;
        let mut handles = Vec::new();
        for text in ["a", "ab", "abc", "abcd"] {
            if let Some(cancel) = previous.take() {
                cancel.cancel();
            }
            let cancel = CancellationToken::new();
            let stream = query_of(&backend, text, &cancel);
            handles.push(tokio::spawn(async move { stream.collect::<Vec<_>>().await }));
            previous = Some(cancel);
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let mut answered = Vec::new();
        for handle in handles {
            answered.extend(handle.await.unwrap());
        }
        assert_eq!(runs(root.path()), 1);
        let [batch] = answered.as_slice() else { panic!("{answered:?}") };
        assert_eq!(batch.items[0].title, "abcd");
    }

    #[tokio::test]
    async fn script_items_appear_through_the_command_provider() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        install(root.path(), python, false, &counting_script());
        let registry = Arc::new(crate::registry::Registry::new());
        let host = crate::watch::ExtensionHost::new(
            root.path().to_path_buf(),
            Arc::new(ScriptBackend::new()),
            Arc::clone(&registry),
        );
        host.reload_all();
        let providers = registry.providers();
        assert_eq!(providers[0].id(), "demo.search");
        let batches: Vec<_> = providers[0]
            .query(QueryContext {
                text: "hi".into(),
                keyword_routed: true,
                cancel: CancellationToken::new(),
            })
            .collect()
            .await;
        let [batch] = batches.as_slice() else { panic!("{batches:?}") };
        assert_eq!(batch.items[0].title, "hi");
        assert_eq!(batch.items[0].accessories, ["Demo"], "labeled with the extension title");
    }

    /// Installs `demo` with a required password and a text preference; the script answers with
    /// the preferences it received.
    fn install_with_preferences(root: &Path, python: &str) -> LoadedExtension {
        let loaded = install(
            root,
            python,
            false,
            &format!(
                "{PRELUDE}send({{'id': request['id'], 'type': 'items', 'done': True, 'items': \
                 [{{'id': 't', 'title': json.dumps(request['preferences'], sort_keys=True)}}]}})\n"
            ),
        );
        let path = loaded.dir.join("extension.toml");
        let mut manifest = std::fs::read_to_string(&path).unwrap();
        manifest.push_str(
            "\n[[preferences]]\nname = \"token\"\ntitle = \"T\"\ntype = \"password\"\nrequired = true\n\
             \n[[preferences]]\nname = \"user\"\ntitle = \"U\"\ntype = \"text\"\n",
        );
        std::fs::write(&path, manifest).unwrap();
        crate::consent::confirm(&loaded.dir).unwrap();
        dir::load_extension(&loaded.dir).unwrap()
    }

    #[tokio::test]
    async fn a_query_carries_the_preferences_with_the_password_from_the_keyring() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let secrets = Arc::new(crate::secrets::MockSecretStore::new());
        let preferences = Arc::new(Preferences::new(
            daimos_core::settings::SettingsStore::new(root.path().join("config")),
            secrets.clone(),
        ));
        let backend = ScriptBackend::new().with_preferences(Arc::clone(&preferences));
        let loaded = install_with_preferences(root.path(), python);
        backend.load(&loaded);
        let cancel = CancellationToken::new();

        // Required and unset: no process runs, one error item names the preference.
        let batches: Vec<_> = query_of(&backend, "x", &cancel).collect().await;
        let [batch] = batches.as_slice() else { panic!("{batches:?}") };
        assert_eq!(batch.items[0].title, "demo needs its preferences");
        assert!(batch.items[0].subtitle.as_deref().unwrap().contains("token"));

        preferences.set(&loaded.manifest, "token", "s3cret").unwrap();
        preferences.set(&loaded.manifest, "user", "ada").unwrap();
        let batches: Vec<_> = query_of(&backend, "x", &cancel).collect().await;
        let [batch] = batches.as_slice() else { panic!("{batches:?}") };
        assert_eq!(batch.items[0].title, r#"{"token": "s3cret", "user": "ada"}"#);

        // A keyring that disappears is reported, not ignored.
        let broken = Arc::new(Preferences::new(
            daimos_core::settings::SettingsStore::new(root.path().join("config")),
            Arc::new(crate::secrets::MockSecretStore::unavailable()),
        ));
        let backend = ScriptBackend::new().with_preferences(broken);
        backend.load(&loaded);
        let batches: Vec<_> = query_of(&backend, "x", &cancel).collect().await;
        assert!(batches[0].items[0].subtitle.as_deref().unwrap().contains("OS keyring"));
    }

    #[tokio::test]
    async fn long_running_unknown_and_unloaded_extensions_are_unaffected() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = ScriptBackend::new();
        let cancel = CancellationToken::new();
        // Never loaded.
        assert!(query_of(&backend, "x", &cancel).collect::<Vec<_>>().await.is_empty());
        // Long-running: stays a stub until its own host exists.
        backend.load(&install(root.path(), python, true, &counting_script()));
        assert!(query_of(&backend, "x", &cancel).collect::<Vec<_>>().await.is_empty());
        // One-shot, then unloaded again.
        backend.load(&install(root.path(), python, false, &counting_script()));
        assert_eq!(query_of(&backend, "x", &cancel).collect::<Vec<_>>().await.len(), 1);
        backend.unload("demo");
        assert!(query_of(&backend, "x", &cancel).collect::<Vec<_>>().await.is_empty());
        assert_eq!(runs(root.path()), 1);
    }

    #[test]
    fn the_stderr_log_goes_below_the_data_dir() {
        let backend = ScriptBackend::with_data_dir(PathBuf::from("data"));
        let root = tempfile::tempdir().unwrap();
        // The manifest need not run for this: only the loaded runner is looked at.
        backend.load(&install(root.path(), "python3", false, ""));
        let log = backend.runner("demo").unwrap().log;
        assert_eq!(log, Some(Path::new("data").join("logs").join("demo.log")));
        assert_eq!(ScriptBackend::new().runner("demo").map(|r| r.log), None);
    }

    /// A script that answers a query with one item and an action with a toast.
    fn actions_script(action_branch: &str) -> String {
        format!(
            "{PRELUDE}open('runs', 'a').write(request['type'] + '\\n')\n\
             if request['type'] == 'query':\n    send({{'id': request['id'], 'type': 'items', 'done': True, \
             'items': [{{'id': 'r1', 'title': 'Repo', 'actions': [\
             {{'id': 'open', 'title': 'Open', 'kind': 'open', 'url': 'https://example.com'}}, \
             {{'id': 'star', 'title': 'Star', 'kind': 'custom'}}]}}]}})\n\
             else:\n{action_branch}"
        )
    }

    fn backend_for(
        root: &Path,
        python: &str,
        action_branch: &str,
        timeout_ms: u64,
    ) -> ScriptBackend {
        let backend = ScriptBackend::new();
        let loaded = install(root, python, false, &actions_script(action_branch));
        let manifest = std::fs::read_to_string(loaded.dir.join("extension.toml")).unwrap();
        let manifest = manifest.replace(
            "long_running = false",
            &format!("long_running = false\ntimeout_ms = {timeout_ms}"),
        );
        std::fs::write(loaded.dir.join("extension.toml"), manifest).unwrap();
        backend.load(&dir::load_extension(&loaded.dir).unwrap());
        backend
    }

    #[test]
    fn a_custom_action_reaches_the_script_and_its_toast_comes_back() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let branch = "    send({'id': request['id'], 'type': 'result', 'toast': 'Starred ' + request['item'] + ' ' + request['action'] + ' ' + request['command'], 'view': None})\n";
        let backend = backend_for(root.path(), python, branch, 10_000);
        assert_eq!(
            backend.custom_action("demo", "search", "r1", "star"),
            Ok(Some("Starred r1 star search".to_owned()))
        );
    }

    #[test]
    fn a_script_that_sends_no_result_succeeds_without_a_toast() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = backend_for(root.path(), python, "    pass\n", 10_000);
        assert_eq!(backend.custom_action("demo", "search", "r1", "star"), Ok(None));
    }

    #[test]
    fn a_failing_custom_action_reports_an_error_with_the_last_stderr_line() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let branch = "    sys.stderr.write('token expired\\n')\n    sys.exit(2)\n";
        let backend = backend_for(root.path(), python, branch, 10_000);
        assert_eq!(
            backend.custom_action("demo", "search", "r1", "star"),
            Err("demo: token expired".to_owned())
        );
    }

    #[test]
    fn a_hanging_custom_action_times_out_instead_of_hanging() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = backend_for(root.path(), python, "    time.sleep(60)\n", 300);
        let started = StdInstant::now();
        let error = backend.custom_action("demo", "search", "r1", "star").unwrap_err();
        assert!(error.contains("timed out after 300 ms"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_custom_action_of_an_unloaded_extension_is_an_error() {
        let error = ScriptBackend::new().custom_action("gone", "c", "i", "a").unwrap_err();
        assert!(error.contains("gone"), "{error}");
    }

    #[tokio::test]
    async fn a_built_in_action_never_starts_the_script() {
        let python = python_or_skip!();
        let root = tempfile::tempdir().unwrap();
        let backend = Arc::new(backend_for(root.path(), python, "    pass\n", 10_000));
        let recorder = Arc::new(crate::provider::tests::Recorder::default());
        let manifest = install_manifest(root.path());
        let provider =
            crate::provider::CommandProvider::new(&manifest, &manifest.commands[0], backend)
                .with_opener(recorder.clone());
        let batches: Vec<_> = provider
            .query(QueryContext {
                text: "x".into(),
                keyword_routed: true,
                cancel: CancellationToken::new(),
            })
            .collect()
            .await;
        assert_eq!(batches.len(), 1);
        assert_eq!(runs(root.path()), 1, "the query started the script once");

        let outcome = provider.run_action_outcome("r1", "open").unwrap().unwrap();
        assert_eq!(outcome.effect, daimos_core::ActionEffect::Hide);
        assert_eq!(*recorder.calls.lock().unwrap(), ["open https://example.com"]);
        assert_eq!(runs(root.path()), 1, "a built-in action did not start the script");

        let custom = provider.run_action_outcome("r1", "star").unwrap().unwrap();
        assert_eq!(custom.toast, None);
        assert_eq!(runs(root.path()), 2, "a custom action did");
    }

    fn install_manifest(root: &Path) -> crate::manifest::Manifest {
        dir::load_extension(&root.join("demo")).unwrap().manifest
    }
}
