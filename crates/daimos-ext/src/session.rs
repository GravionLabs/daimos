//! Running a script extension in long-running mode: one process serves many requests.
//!
//! A [`Session`] starts its process on the first request and keeps it. Requests get increasing
//! ids from one counter; the reader task routes every response line to the request with that id,
//! so answers may arrive interleaved or out of order. A superseded query sends `cancel` to the
//! script and ignores whatever it still answers for that id.
//!
//! A process that ends on its own (exit, closed pipes, garbage on stdout) has crashed: the requests
//! in flight fail and the next request starts a fresh process after a backoff. Too many crashes in
//! a short time put the session in a failed state until a new session replaces it. A process the
//! session stops itself (timeout kill, [`Session::shutdown`], drop) never counts.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daimos_core::model;
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::manifest::ScriptRuntime;
use crate::protocol::{self, Request, Response};
use crate::script::{Event, Failure, OneShot, STDERR_GRACE, pump_stderr};

/// The first restart waits this long; every further restart within the window doubles it.
const BACKOFF: Duration = Duration::from_millis(250);
/// Restarts allowed within [`WINDOW`]; the crash after that fails the session.
const MAX_RESTARTS: usize = 3;
/// The rolling window the restarts are counted in.
const WINDOW: Duration = Duration::from_secs(60);

/// The restart bookkeeping of a session: when the next start may happen and whether the process
/// crashes too often to be started again.
#[derive(Default)]
struct Health {
    /// When the restarts happened, oldest first; only those within [`WINDOW`] matter.
    restarts: VecDeque<Instant>,
    /// The next process may start no earlier than this.
    ready_at: Option<Instant>,
    /// Set by the crash that exceeded the limit; carries the last stderr line.
    failed: Option<Option<String>>,
}

impl Health {
    /// Records a crash at `at`: schedules the restart or, past the limit, fails for good.
    fn crashed(&mut self, at: Instant, stderr: Option<String>) {
        if self.failed.is_some() {
            return;
        }
        while self.restarts.front().is_some_and(|&t| at.saturating_duration_since(t) >= WINDOW) {
            self.restarts.pop_front();
        }
        if self.restarts.len() >= MAX_RESTARTS {
            self.failed = Some(stderr);
            return;
        }
        self.ready_at = Some(at + BACKOFF * (1 << self.restarts.len()));
        self.restarts.push_back(at);
    }
}

/// What a request hears from the process.
enum Step {
    Message(Response),
    Failed(Failure),
}

/// The waiting requests by id; `None` once the process has ended, so a late registration fails
/// instead of waiting for an answer that cannot come.
type Waiting = Mutex<Option<HashMap<u64, mpsc::UnboundedSender<Step>>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// One running child process with its reader and writer tasks.
struct Process {
    lines: mpsc::UnboundedSender<String>,
    waiting: Arc<Waiting>,
    /// Asks the supervisor to kill the child.
    kill: CancellationToken,
    /// Fires once the child is reaped and every waiting request has been told.
    exited: CancellationToken,
    /// Set (before `exited` fires) when the process ended without being asked to: when and why.
    crash: Arc<Mutex<Option<(Instant, Failure)>>>,
}

impl Process {
    /// Starts the process of `spec`; must run inside a tokio runtime.
    fn start(spec: &OneShot) -> Result<Arc<Self>, Failure> {
        let mut child = spec.spawn().map_err(|error| {
            Failure::Spawn(format!("cannot start {}: {error}", spec.command[0]))
        })?;
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(Failure::Spawn(format!("cannot talk to {}", spec.command[0])));
        };
        let log = spec.log.clone();
        let stderr = child.stderr.take().map(|stderr| tokio::spawn(pump_stderr(stderr, log)));
        let waiting: Arc<Waiting> = Arc::new(Mutex::new(Some(HashMap::new())));
        let (lines, mut outgoing) = mpsc::unbounded_channel::<String>();
        let kill = CancellationToken::new();
        let exited = CancellationToken::new();
        let crash = Arc::new(Mutex::new(None));

        // The writer is a task of its own, so a script that is busy writing never blocks on us.
        let stop = kill.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    line = outgoing.recv() => line,
                };
                let Some(line) = line else { break };
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        tokio::spawn(supervise(
            child,
            tokio::io::AsyncBufReadExt::lines(BufReader::new(stdout)),
            stderr,
            Arc::clone(&waiting),
            kill.clone(),
            exited.clone(),
            Arc::clone(&crash),
        ));
        Ok(Arc::new(Self { lines, waiting, kill, exited, crash }))
    }

    /// Registers request `id`; `None` when the process has already ended.
    fn register(&self, id: u64) -> Option<mpsc::UnboundedReceiver<Step>> {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.waiting).as_mut()?.insert(id, tx);
        Some(rx)
    }

    fn deregister(&self, id: u64) {
        if let Some(map) = lock(&self.waiting).as_mut() {
            map.remove(&id);
        }
    }

    /// Queues a request line; a process that is gone ignores it.
    fn send(&self, request: &Request) {
        let _ = self.lines.send(protocol::encode_line(request));
    }

    /// Kills the child and waits until it is reaped.
    async fn kill_and_wait(&self) {
        self.kill.cancel();
        self.exited.cancelled().await;
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill.cancel();
    }
}

/// Owns the child: routes its output to the waiting requests until it ends, then fails whoever
/// still waits and reaps it.
async fn supervise(
    mut child: Child,
    mut lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    mut stderr: Option<tokio::task::JoinHandle<Option<String>>>,
    waiting: Arc<Waiting>,
    kill: CancellationToken,
    exited: CancellationToken,
    crash: Arc<Mutex<Option<(Instant, Failure)>>>,
) {
    let mut deliberate = false;
    let mut failure = loop {
        tokio::select! {
            biased;
            () = kill.cancelled() => {
                deliberate = true;
                let _ = child.kill().await;
                break Failure::Exit { code: None, stderr: None };
            }
            line = lines.next_line() => match line {
                Ok(Some(line)) => match protocol::parse_line::<Response>(&line) {
                    Ok(None) => {}
                    Ok(Some(response)) => route(&waiting, response),
                    Err(error) => {
                        let _ = child.kill().await;
                        break Failure::Protocol(error.to_string());
                    }
                },
                Ok(None) => {
                    // Output closed: the process is ending; do not wait for it forever.
                    let status = match tokio::time::timeout(STDERR_GRACE, child.wait()).await {
                        Ok(status) => status.ok(),
                        Err(_) => {
                            let _ = child.kill().await;
                            None
                        }
                    };
                    break Failure::Exit { code: status.and_then(|s| s.code()), stderr: None };
                }
                Err(error) => {
                    let _ = child.kill().await;
                    break Failure::Protocol(format!("cannot read the output: {error}"));
                }
            },
        }
    };
    if let Failure::Exit { stderr: last, .. } = &mut failure
        && let Some(handle) = stderr.take()
    {
        *last =
            tokio::time::timeout(STDERR_GRACE, handle).await.ok().and_then(Result::ok).flatten();
    }
    if !deliberate {
        *lock(&crash) = Some((Instant::now(), failure.clone()));
    }
    kill.cancel();
    let waiters = lock(&waiting).take().unwrap_or_default();
    for sender in waiters.into_values() {
        let _ = sender.send(Step::Failed(failure.clone()));
    }
    exited.cancel();
}

/// Hands a response to the request it answers; an id nobody waits for (cancelled, finished or
/// made up) is dropped.
fn route(waiting: &Waiting, response: Response) {
    if let Some(sender) = lock(waiting).as_ref().and_then(|map| map.get(&response.id())) {
        let _ = sender.send(Step::Message(response));
    }
}

struct Inner {
    spec: OneShot,
    /// The last id handed out; one counter for the whole session, across process restarts.
    last_id: AtomicU64,
    process: Mutex<Option<Arc<Process>>>,
    health: Mutex<Health>,
}

impl Inner {
    fn next_id(&self) -> u64 {
        self.last_id.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// A long-running script extension: one process, many requests. Cheap to clone; clones share the
/// process.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// A session for `runtime` of the extension called `extension` in `dir`. Nothing starts until
    /// the first request.
    pub fn new(extension: &str, dir: PathBuf, runtime: &ScriptRuntime) -> Self {
        Self::from_runner(OneShot::new(extension, dir, runtime))
    }

    /// Appends the stderr of the process to `path` (`<data dir>/logs/<extension>.log`).
    #[must_use]
    pub fn with_log(self, path: PathBuf) -> Self {
        Self::from_runner(self.inner.spec.clone().with_log(path))
    }

    fn from_runner(spec: OneShot) -> Self {
        Self {
            inner: Arc::new(Inner {
                spec,
                last_id: AtomicU64::new(0),
                process: Mutex::new(None),
                health: Mutex::default(),
            }),
        }
    }

    pub fn timeout(&self) -> Duration {
        self.inner.spec.timeout
    }

    /// Whether a process is alive right now.
    pub fn is_running(&self) -> bool {
        lock(&self.inner.process).as_ref().is_some_and(|p| !p.exited.is_cancelled())
    }

    /// Whether the process crashed too often (noticed by the next request): no more starts, every request fails with
    /// [`Failure::KeepsCrashing`]. A new session starts clean.
    pub fn is_failed(&self) -> bool {
        lock(&self.inner.health).failed.is_some()
    }

    /// Kills the process (if any) and waits until it is reaped. The next request starts a new one.
    pub async fn shutdown(&self) {
        let process = lock(&self.inner.process).take();
        if let Some(process) = process {
            process.kill_and_wait().await;
        }
    }

    /// The live process, started if there is none. A process that crashed since the last call is
    /// counted first; the start waits for its backoff.
    async fn process(&self) -> Result<Arc<Process>, Failure> {
        loop {
            let wait_until = {
                let mut slot = lock(&self.inner.process);
                let mut health = lock(&self.inner.health);
                if slot.as_ref().is_some_and(|p| p.exited.is_cancelled())
                    && let Some(process) = slot.take()
                    && let Some((at, failure)) = lock(&process.crash).take()
                {
                    let stderr = match failure {
                        Failure::Exit { stderr, .. } => stderr,
                        other => Some(other.message()),
                    };
                    health.crashed(at, stderr);
                }
                if let Some(stderr) = &health.failed {
                    return Err(Failure::KeepsCrashing { stderr: stderr.clone() });
                }
                if let Some(process) = slot.as_ref() {
                    return Ok(Arc::clone(process));
                }
                match health.ready_at.filter(|&t| t > Instant::now()) {
                    Some(at) => at,
                    None => {
                        let process = Process::start(&self.inner.spec)?;
                        *slot = Some(Arc::clone(&process));
                        return Ok(process);
                    }
                }
            };
            tokio::time::sleep_until(wait_until).await;
        }
    }

    /// Sends the request built by `build` (given its fresh id) and returns the call that waits for
    /// the answer. `stop` says which response ends it.
    async fn begin(
        &self,
        build: impl Fn(u64) -> Request,
        cancel: CancellationToken,
        stop: fn(&Response) -> bool,
    ) -> Result<Call, Failure> {
        // A process that ended between the lookup and the registration is replaced once.
        for _ in 0..2 {
            let process = self.process().await?;
            let id = self.inner.next_id();
            if let Some(answers) = process.register(id) {
                process.send(&build(id));
                return Ok(Call {
                    inner: Arc::clone(&self.inner),
                    process,
                    id,
                    answers,
                    deadline: Instant::now() + self.inner.spec.timeout,
                    cancel,
                    stop,
                    finished: false,
                });
            }
        }
        Err(Failure::Exit { code: None, stderr: None })
    }

    /// Streams the items a query answers. The process starts when the stream is first polled (if
    /// it is not running yet). The stream ends at `done: true`, at a failure (a
    /// [`Failure::Timeout`] kills the process) or when `cancel` fires: then the script is told to
    /// drop the work and nothing more is yielded.
    pub fn query(
        &self,
        command: &str,
        text: &str,
        preferences: BTreeMap<String, String>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Event> {
        struct State {
            session: Session,
            start: Option<(String, String, BTreeMap<String, String>, CancellationToken)>,
            call: Option<Call>,
        }
        let state = State {
            session: self.clone(),
            start: Some((command.to_owned(), text.to_owned(), preferences, cancel)),
            call: None,
        };
        stream::unfold(state, |mut state| async move {
            if let Some((command, text, preferences, cancel)) = state.start.take() {
                if cancel.is_cancelled() {
                    return None;
                }
                let begun = state
                    .session
                    .begin(
                        |id| Request::Query {
                            id,
                            command: command.clone(),
                            text: text.clone(),
                            preferences: preferences.clone(),
                        },
                        cancel,
                        |response| matches!(response, Response::Items { done: true, .. }),
                    )
                    .await;
                match begun {
                    Ok(call) => state.call = Some(call),
                    Err(failure) => return Some((Event::Failed(failure), state)),
                }
            }
            loop {
                match state.call.as_mut()?.next().await? {
                    Step::Failed(failure) => return Some((Event::Failed(failure), state)),
                    Step::Message(Response::Items { items, .. }) if !items.is_empty() => {
                        let items: Vec<model::Item> = items.into_iter().map(Into::into).collect();
                        return Some((Event::Items(items), state));
                    }
                    Step::Message(_) => {}
                }
            }
        })
        .boxed()
    }

    /// Runs a custom action and waits for its `result`; the answer is the toast, if the script
    /// sent one. The same timeout and cancel rules as for a query apply; a cancelled action
    /// answers `Ok(None)`.
    pub async fn action(
        &self,
        command: &str,
        item: &str,
        action: &str,
        cancel: CancellationToken,
    ) -> Result<Option<String>, Failure> {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let mut call = self
            .begin(
                |id| Request::Action {
                    id,
                    command: command.to_owned(),
                    item: item.to_owned(),
                    action: action.to_owned(),
                },
                cancel,
                |response| matches!(response, Response::Result { .. }),
            )
            .await?;
        let mut toast = None;
        while let Some(step) = call.next().await {
            match step {
                Step::Failed(failure) => return Err(failure),
                Step::Message(Response::Result { toast: sent, .. }) => toast = sent,
                Step::Message(_) => {}
            }
        }
        Ok(toast)
    }
}

/// One request in flight.
struct Call {
    inner: Arc<Inner>,
    process: Arc<Process>,
    id: u64,
    answers: mpsc::UnboundedReceiver<Step>,
    deadline: Instant,
    cancel: CancellationToken,
    stop: fn(&Response) -> bool,
    /// No more steps; the id is no longer registered.
    finished: bool,
}

impl Call {
    /// The next thing that happened to the request, or `None` when it is over (answered,
    /// failed or cancelled).
    async fn next(&mut self) -> Option<Step> {
        if self.finished {
            return None;
        }
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => {
                self.abort();
                None
            }
            () = tokio::time::sleep_until(self.deadline) => {
                self.finish();
                self.process.kill_and_wait().await;
                Some(Step::Failed(Failure::Timeout(self.inner.spec.timeout)))
            }
            step = self.answers.recv() => match step {
                Some(Step::Message(response)) => {
                    if (self.stop)(&response) {
                        self.finish();
                    }
                    Some(Step::Message(response))
                }
                Some(Step::Failed(failure)) => {
                    self.finish();
                    Some(Step::Failed(failure))
                }
                None => {
                    self.finish();
                    None
                }
            },
        }
    }

    fn finish(&mut self) {
        self.finished = true;
        self.process.deregister(self.id);
    }

    /// Gives up the request: later answers for it are dropped and the script is told to stop.
    fn abort(&mut self) {
        if self.finished {
            return;
        }
        self.finish();
        let cancel = Request::Cancel { id: self.inner.next_id(), cancels: self.id };
        self.process.send(&cancel);
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health_after(crashes: &[u64]) -> (Health, Instant) {
        let base = Instant::now();
        let mut health = Health::default();
        for &secs in crashes {
            health.crashed(base + Duration::from_secs(secs), Some(format!("at {secs}")));
        }
        (health, base)
    }

    #[tokio::test(start_paused = true)]
    async fn backoff_doubles_with_every_restart_in_the_window() {
        let base = Instant::now();
        let mut health = Health::default();
        for (second, delay_ms) in [(0, 250), (1, 500), (2, 1000)] {
            let at = base + Duration::from_secs(second);
            health.crashed(at, None);
            assert_eq!(health.ready_at, Some(at + Duration::from_millis(delay_ms)));
            assert!(health.failed.is_none());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_fourth_crash_within_a_minute_fails_with_the_last_stderr_line() {
        let (health, _) = health_after(&[0, 10, 20, 30]);
        assert_eq!(health.failed, Some(Some("at 30".to_owned())));
    }

    #[tokio::test(start_paused = true)]
    async fn the_window_slides_so_old_crashes_expire() {
        // The first restart is 60 s old at the fourth crash: it no longer counts.
        let (health, base) = health_after(&[0, 10, 20, 60]);
        assert!(health.failed.is_none());
        assert_eq!(health.restarts.len(), 3);
        // Backoff restarts from the start of the sequence for the two that remain.
        assert_eq!(health.ready_at, Some(base + Duration::from_secs(60) + BACKOFF * 4));
        // 59 s after the first restart it still counts.
        let (health, _) = health_after(&[0, 10, 20, 59]);
        assert!(health.failed.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_session_stays_failed() {
        let (mut health, base) = health_after(&[0, 1, 2, 3]);
        health.crashed(base + Duration::from_secs(1000), Some("later".to_owned()));
        assert_eq!(health.failed, Some(Some("at 3".to_owned())));
    }
}
