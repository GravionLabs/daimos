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
//! session stops itself (timeout kill, idle stop, [`Session::shutdown`], drop) never counts.
//!
//! A process that has had no request in flight for `idle_timeout` is stopped: stdin is closed so
//! it can exit, and it is killed if it has not after a short grace. The next request starts a
//! fresh one through the normal lazy start.
//!
//! Memory stays bounded whatever the script does: a stdout line longer than [`MAX_LINE`] is a
//! crash (the process is killed as soon as the line passes the limit, request or not), a request
//! with more than 256 unread answers fails with [`Failure::Dropped`], and request lines
//! for a script that does not read its input are capped too.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daimos_core::model;
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdout};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::manifest::ScriptRuntime;
use crate::protocol::{self, Request, Response};
use crate::script::{
    Event, Failure, Line, LineReader, MAX_LINE, OneShot, STDERR_GRACE, deadline_after,
    line_too_long, pump_stderr, utf8_line,
};

/// The first restart waits this long; every further restart within the window doubles it.
const BACKOFF: Duration = Duration::from_millis(250);
/// Restarts allowed within [`WINDOW`]; the crash after that fails the session.
const MAX_RESTARTS: usize = 3;
/// The rolling window the restarts are counted in.
const WINDOW: Duration = Duration::from_secs(60);
/// After stdin is closed for an idle stop, the process gets this long to exit before it is killed.
const IDLE_GRACE: Duration = Duration::from_millis(500);
/// Longest idle timeout honored; keeps the deadline arithmetic far from overflow.
const MAX_IDLE: Duration = Duration::from_secs(365 * 24 * 3600);
/// Answers of one request that may wait unread; one more fails the request.
const MAX_PENDING: usize = 256;
/// Bytes of answers of one request that may wait unread; more fail the request.
const MAX_PENDING_BYTES: usize = 16 * 1024 * 1024;
/// Request lines that may wait for a script that does not read its input.
const MAX_OUTGOING: usize = 64;

/// What the idle watcher needs to know about a process.
struct Activity {
    state: Mutex<ActivityState>,
    /// Set (before `exited` fires) when the process ended without being asked to: when and why.
    crash: Mutex<Option<(Instant, Failure)>>,
    /// Why the session killed the process; what the requests still waiting are told.
    reason: Mutex<Option<Failure>>,
}

struct ActivityState {
    /// Requests registered and not yet finished.
    in_flight: usize,
    /// The last time a request started or finished.
    last: Instant,
    /// The idle watcher decided to stop the process: no request may join it any more.
    stopping: bool,
}

impl Activity {
    fn new() -> Self {
        Self {
            state: Mutex::new(ActivityState {
                in_flight: 0,
                last: Instant::now(),
                stopping: false,
            }),
            crash: Mutex::new(None),
            reason: Mutex::new(None),
        }
    }

    /// Counts a request in; `false` when the process is being stopped.
    fn enter(&self) -> bool {
        let mut state = lock(&self.state);
        if state.stopping {
            return false;
        }
        state.in_flight += 1;
        state.last = Instant::now();
        true
    }

    fn leave(&self) {
        let mut state = lock(&self.state);
        state.in_flight = state.in_flight.saturating_sub(1);
        state.last = Instant::now();
    }

    fn is_stopping(&self) -> bool {
        lock(&self.state).stopping
    }

    /// Marks the process as stopping when it has been idle for `idle`; otherwise says when to look
    /// again.
    fn check(&self, idle: Duration) -> Option<Instant> {
        let mut state = lock(&self.state);
        if state.in_flight > 0 {
            return Some(Instant::now() + idle);
        }
        let due = state.last + idle;
        if Instant::now() >= due {
            state.stopping = true;
            None
        } else {
            Some(due)
        }
    }
}

/// Stops the process once it has been idle for `idle`: closes its stdin (`close`), and kills it
/// if it has not exited after [`IDLE_GRACE`]. Returns when the process is gone.
async fn watch_idle(
    activity: Arc<Activity>,
    idle: Duration,
    close: CancellationToken,
    kill: CancellationToken,
    exited: CancellationToken,
) {
    let idle = idle.min(MAX_IDLE);
    while let Some(look_again) = activity.check(idle) {
        tokio::select! {
            () = exited.cancelled() => return,
            () = tokio::time::sleep_until(look_again) => {}
        }
    }
    close.cancel();
    tokio::select! {
        () = exited.cancelled() => {}
        () = tokio::time::sleep(IDLE_GRACE) => kill.cancel(),
    }
}

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

/// The end of a request that the channel cannot carry: set before its sender is dropped.
type Fate = Arc<Mutex<Option<Failure>>>;

/// The process side of one waiting request.
struct Waiter {
    answers: mpsc::Sender<(Response, usize)>,
    /// Bytes sent and not yet read by the request.
    pending: Arc<AtomicUsize>,
    fate: Fate,
}

/// The waiting requests by id; `None` once the process has ended, so a late registration fails
/// instead of waiting for an answer that cannot come.
type Waiting = Mutex<Option<HashMap<u64, Waiter>>>;

/// The request side of a registration.
struct Registration {
    answers: mpsc::Receiver<(Response, usize)>,
    pending: Arc<AtomicUsize>,
    fate: Fate,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// One running child process with its reader and writer tasks.
struct Process {
    lines: mpsc::Sender<String>,
    waiting: Arc<Waiting>,
    /// Asks the supervisor to kill the child.
    kill: CancellationToken,
    /// Fires once the child is reaped and every waiting request has been told.
    exited: CancellationToken,
    /// Idle bookkeeping and the crash record.
    activity: Arc<Activity>,
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
        let (lines, mut outgoing) = mpsc::channel::<String>(MAX_OUTGOING);
        let kill = CancellationToken::new();
        let exited = CancellationToken::new();
        let activity = Arc::new(Activity::new());
        let close = CancellationToken::new();

        // The writer is a task of its own, so a script that is busy writing never blocks on us.
        let stop = kill.clone();
        let closed = close.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    biased;
                    () = stop.cancelled() => break,
                    () = closed.cancelled() => break,
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
            LineReader::new(stdout, MAX_LINE),
            stderr,
            Arc::clone(&waiting),
            kill.clone(),
            exited.clone(),
            Arc::clone(&activity),
        ));
        tokio::spawn(watch_idle(
            Arc::clone(&activity),
            spec.idle_timeout,
            close,
            kill.clone(),
            exited.clone(),
        ));
        Ok(Arc::new(Self { lines, waiting, kill, exited, activity }))
    }

    /// Registers request `id`; `None` when the process has ended or is being stopped.
    fn register(&self, id: u64) -> Option<Registration> {
        let (tx, rx) = mpsc::channel(MAX_PENDING);
        let pending = Arc::new(AtomicUsize::new(0));
        let fate: Fate = Arc::default();
        let mut waiting = lock(&self.waiting);
        let map = waiting.as_mut()?;
        if !self.activity.enter() {
            return None;
        }
        let waiter = Waiter { answers: tx, pending: Arc::clone(&pending), fate: Arc::clone(&fate) };
        map.insert(id, waiter);
        Some(Registration { answers: rx, pending, fate })
    }

    fn deregister(&self, id: u64) {
        if let Some(map) = lock(&self.waiting).as_mut()
            && map.remove(&id).is_some()
        {
            self.activity.leave();
        }
    }

    /// Queues a request line; `false` when the script has not read the lines before it. A process
    /// that is gone ignores it.
    fn send(&self, request: &Request) -> bool {
        match self.lines.try_send(protocol::encode_line(request)) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => true,
            Err(mpsc::error::TrySendError::Full(_)) => false,
        }
    }

    /// Kills the child and waits until it is reaped; the requests still waiting fail with
    /// `reason`.
    async fn stop(&self, reason: Failure) {
        lock(&self.activity.reason).get_or_insert(reason);
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
    mut lines: LineReader<ChildStdout>,
    mut stderr: Option<tokio::task::JoinHandle<Option<String>>>,
    waiting: Arc<Waiting>,
    kill: CancellationToken,
    exited: CancellationToken,
    activity: Arc<Activity>,
) {
    let mut deliberate = false;
    let mut failure = loop {
        tokio::select! {
            biased;
            () = kill.cancelled() => {
                deliberate = true;
                let _ = child.kill().await;
                let reason = lock(&activity.reason).take();
                break reason.unwrap_or(Failure::Exit { code: None, stderr: None });
            }
            line = lines.next_line() => match line {
                Ok(Line::Text(line)) => {
                    let size = line.len();
                    let parsed = utf8_line(line).and_then(|line| {
                        protocol::parse_line::<Response>(&line)
                            .map_err(|error| Failure::Protocol(error.to_string()))
                    });
                    match parsed {
                        Ok(None) => {}
                        Ok(Some(response)) => route(&waiting, &activity, response, size),
                        Err(failure) => {
                            let _ = child.kill().await;
                            break failure;
                        }
                    }
                }
                Ok(Line::TooLong(_)) => {
                    // Killed at once, waiting request or not: the line is never buffered whole.
                    let _ = child.kill().await;
                    break line_too_long();
                }
                Ok(Line::End) => {
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
    if !deliberate && !activity.is_stopping() {
        *lock(&activity.crash) = Some((Instant::now(), failure.clone()));
    }
    kill.cancel();
    let waiters = lock(&waiting).take().unwrap_or_default();
    for waiter in waiters.into_values() {
        // The answers already sent are read first; the failure comes when they run out.
        *lock(&waiter.fate) = Some(failure.clone());
    }
    exited.cancel();
}

/// Hands a response of `size` bytes to the request it answers; an id nobody waits for
/// (cancelled, finished or made up) is dropped. A request with too much unread fails.
fn route(waiting: &Waiting, activity: &Activity, response: Response, size: usize) {
    let id = response.id();
    let mut waiting = lock(waiting);
    let Some(map) = waiting.as_mut() else { return };
    let Some(waiter) = map.get(&id) else { return };
    let pending = waiter.pending.fetch_add(size, Ordering::SeqCst) + size;
    let delivered =
        pending <= MAX_PENDING_BYTES && waiter.answers.try_send((response, size)).is_ok();
    if !delivered && let Some(waiter) = map.remove(&id) {
        *lock(&waiter.fate) = Some(Failure::Dropped(format!(
            "sent answers faster than they were read (more than {MAX_PENDING} messages or {} MiB)",
            MAX_PENDING_BYTES / (1024 * 1024)
        )));
        activity.leave();
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
        self.stop("the extension was stopped").await;
    }

    /// Like [`shutdown`](Self::shutdown); the requests still waiting fail with
    /// [`Failure::Interrupted`] carrying `reason`.
    pub async fn stop(&self, reason: &str) {
        let process = lock(&self.inner.process).take();
        if let Some(process) = process {
            process.stop(Failure::Interrupted(reason.to_owned())).await;
        }
    }

    /// The live process, started if there is none. A process that crashed since the last call is
    /// counted first; the start waits for its backoff.
    async fn process(&self) -> Result<Arc<Process>, Failure> {
        loop {
            let stopping = {
                let slot = lock(&self.inner.process);
                slot.as_ref()
                    .filter(|p| p.activity.is_stopping() && !p.exited.is_cancelled())
                    .map(|p| p.exited.clone())
            };
            if let Some(exited) = stopping {
                // An idle stop is under way: the new process starts once the old one is gone.
                exited.cancelled().await;
                continue;
            }
            let wait_until = {
                let mut slot = lock(&self.inner.process);
                let mut health = lock(&self.inner.health);
                if slot.as_ref().is_some_and(|p| p.exited.is_cancelled())
                    && let Some(process) = slot.take()
                    && let Some((at, failure)) = lock(&process.activity.crash).take()
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
                    None => match Process::start(&self.inner.spec) {
                        Ok(process) => {
                            *slot = Some(Arc::clone(&process));
                            return Ok(process);
                        }
                        Err(failure) => {
                            // A start that fails counts like a crash: backoff, then the limit.
                            health.crashed(Instant::now(), Some(failure.message()));
                            return Err(failure);
                        }
                    },
                }
            };
            tokio::time::sleep_until(wait_until).await;
        }
    }

    /// Sends the request built by `build` (given its fresh id) and returns the call that waits for
    /// the answer. `stop` says which response ends it. `Ok(None)` when `cancel` fired first: then
    /// nothing was sent.
    async fn begin(
        &self,
        build: impl Fn(u64) -> Request,
        cancel: CancellationToken,
        stop: fn(&Response) -> bool,
    ) -> Result<Option<Call>, Failure> {
        // A process that ends (or is being stopped) between the lookup and the registration is
        // waited out and replaced; a few times at most.
        for _ in 0..3 {
            let process = tokio::select! {
                biased;
                () = cancel.cancelled() => return Ok(None),
                process = self.process() => process?,
            };
            let id = self.inner.next_id();
            let Some(registration) = process.register(id) else {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Ok(None),
                    () = process.exited.cancelled() => continue,
                }
            };
            let call = Call {
                inner: Arc::clone(&self.inner),
                process,
                id,
                answers: registration.answers,
                pending: registration.pending,
                fate: registration.fate,
                deadline: deadline_after(self.inner.spec.timeout),
                cancel,
                stop,
                finished: false,
            };
            if !call.process.send(&build(id)) {
                let mut call = call;
                call.finish();
                return Err(Failure::Dropped("does not read its input".to_owned()));
            }
            return Ok(Some(call));
        }
        Err(Failure::Interrupted("the process kept ending before it took the request".to_owned()))
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
                    Ok(Some(call)) => state.call = Some(call),
                    Ok(None) => return None,
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
        let Some(mut call) = self
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
            .await?
        else {
            return Ok(None);
        };
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
    answers: mpsc::Receiver<(Response, usize)>,
    /// Bytes of answers not yet read; shared with the router.
    pending: Arc<AtomicUsize>,
    /// How the request ended when the channel closed under it.
    fate: Fate,
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
                let reason = Failure::Interrupted(format!(
                    "restarted because another request took longer than {} ms",
                    self.inner.spec.timeout.as_millis()
                ));
                self.process.stop(reason).await;
                Some(Step::Failed(Failure::Timeout(self.inner.spec.timeout)))
            }
            answer = self.answers.recv() => match answer {
                Some((response, size)) => {
                    self.pending.fetch_sub(size, Ordering::SeqCst);
                    if (self.stop)(&response) {
                        self.finish();
                    }
                    Some(Step::Message(response))
                }
                None => {
                    self.finish();
                    lock(&self.fate).take().map(Step::Failed)
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

    struct Watch {
        activity: Arc<Activity>,
        close: CancellationToken,
        kill: CancellationToken,
        exited: CancellationToken,
        task: tokio::task::JoinHandle<()>,
    }

    fn watch(idle: Duration) -> Watch {
        let activity = Arc::new(Activity::new());
        let (close, kill, exited) =
            (CancellationToken::new(), CancellationToken::new(), CancellationToken::new());
        let task = tokio::spawn(watch_idle(
            Arc::clone(&activity),
            idle,
            close.clone(),
            kill.clone(),
            exited.clone(),
        ));
        Watch { activity, close, kill, exited, task }
    }

    const IDLE: Duration = Duration::from_secs(10);

    #[tokio::test(start_paused = true)]
    async fn stdin_is_closed_once_the_idle_time_has_passed() {
        let w = watch(IDLE);
        tokio::time::sleep(IDLE - Duration::from_millis(1)).await;
        assert!(!w.close.is_cancelled());
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(w.close.is_cancelled());
        assert!(w.activity.is_stopping());
        assert!(!w.kill.is_cancelled(), "the process gets a grace period to exit");
    }

    #[tokio::test(start_paused = true)]
    async fn a_process_that_ignores_the_closed_stdin_is_killed_after_the_grace() {
        let w = watch(IDLE);
        tokio::time::sleep(IDLE + IDLE_GRACE - Duration::from_millis(1)).await;
        assert!(!w.kill.is_cancelled());
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(w.kill.is_cancelled());
        w.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_process_that_exits_within_the_grace_is_not_killed() {
        let w = watch(IDLE);
        tokio::time::sleep(IDLE + Duration::from_millis(10)).await;
        w.exited.cancel();
        w.task.await.unwrap();
        assert!(!w.kill.is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_resets_the_idle_time() {
        let w = watch(IDLE);
        tokio::time::sleep(Duration::from_secs(8)).await;
        assert!(w.activity.enter());
        w.activity.leave();
        tokio::time::sleep(Duration::from_secs(8)).await;
        assert!(!w.close.is_cancelled(), "16 s after the start, but only 8 s idle");
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(w.close.is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_in_flight_keeps_the_process_however_long_it_takes() {
        let w = watch(IDLE);
        assert!(w.activity.enter());
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!w.close.is_cancelled());
        w.activity.leave();
        tokio::time::sleep(IDLE - Duration::from_secs(1)).await;
        assert!(!w.close.is_cancelled(), "the idle time counts from the end of the request");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(w.close.is_cancelled());
    }

    #[tokio::test(start_paused = true)]
    async fn no_request_joins_a_process_that_is_being_stopped() {
        let w = watch(IDLE);
        tokio::time::sleep(IDLE + Duration::from_millis(10)).await;
        assert!(!w.activity.enter());
    }

    #[tokio::test]
    async fn a_request_that_cannot_register_waits_for_the_exit_and_uses_a_new_process() {
        let Some(python) = crate::script::testutil::python() else {
            eprintln!("skipped: no python3 or python on PATH");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        crate::script::testutil::write_script(
            dir.path(),
            "import json, sys\nopen('runs', 'a').write('x\\n')\nfor line in sys.stdin:\n\
             \x20   r = json.loads(line)\n\
             \x20   print(json.dumps({'id': r['id'], 'type': 'items', 'done': True, \
             'items': [{'id': 't', 'title': r['text']}]}), flush=True)\n",
        );
        let runtime = ScriptRuntime {
            command: vec![python.to_owned(), "main.py".to_owned()],
            long_running: true,
            timeout_ms: 10_000,
            idle_timeout_ms: 600_000,
        };
        let session = Session::new("demo", dir.path().to_path_buf(), &runtime);
        let ask = |text: &str| {
            session.query("search", text, BTreeMap::new(), CancellationToken::new()).collect()
        };
        let events: Vec<Event> = ask("a").await;
        assert!(matches!(events.as_slice(), [Event::Items(_)]));

        // As if the supervisor were just failing the waiters: the process takes no request, and
        // it is gone a moment later.
        let old = lock(&session.inner.process).clone().unwrap();
        *lock(&old.waiting) = None;
        let kill = old.kill.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            kill.cancel();
        });
        let events: Vec<Event> = ask("b").await;
        let [Event::Items(items)] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(items[0].title, "b");
        assert!(old.exited.is_cancelled());
        let runs = std::fs::read_to_string(dir.path().join("runs")).unwrap();
        assert_eq!(runs.lines().count(), 2, "the request went to a new process");
        assert!(!session.is_failed(), "a deliberate kill is no crash");
    }

    #[tokio::test(start_paused = true)]
    async fn a_process_that_ends_earlier_ends_the_watcher() {
        let w = watch(IDLE);
        w.exited.cancel();
        w.task.await.unwrap();
        assert!(!w.close.is_cancelled());
    }
}
