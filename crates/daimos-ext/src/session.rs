//! Running a script extension in long-running mode: one process that serves many requests.
//!
//! The process starts at the first request and stays alive. Every request carries an id from one
//! counter per session; a reader task routes each response to the request that waits for it, so
//! interleaved answers reach the right query. A query that is superseded sends `cancel` and its
//! late answers are dropped. The tasks of a process run on a thread of the session, so a request
//! can be made from any runtime or, as a blocking call, from a thread that has none.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use daimos_core::model::{self, ItemBatch};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

use crate::manifest::ScriptRuntime;
use crate::protocol::{self, Request, Response};
use crate::script::{Event, Failure, OneShot, batches, pump_stderr_into};

/// How long the end of a process waits for its stderr to drain.
const STDERR_GRACE: Duration = Duration::from_millis(500);

/// An idle process is given this long to exit after its input is closed, then it is killed.
const IDLE_GRACE: Duration = Duration::from_millis(500);

/// A closed output pipe is given this long to be followed by the exit before the process is killed.
const EOF_GRACE: Duration = Duration::from_millis(200);

/// How often a request retries when the process it picked turned out to be dying.
const ATTEMPTS: usize = 3;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// How a process ended.
#[derive(Debug, Clone, Copy)]
struct Exit {
    code: Option<i32>,
}

/// When a crashed process is started again and when the session gives up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    /// How many restarts are allowed within `window`; the crash after that fails the session.
    pub max_restarts: usize,
    /// The rolling period the restarts are counted in.
    pub window: Duration,
    /// The wait before the first restart; it doubles with every crash within the window.
    pub backoff: Duration,
}

impl Default for RestartPolicy {
    /// Three restarts per rolling minute, after 250 ms, 500 ms and 1 s.
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window: Duration::from_secs(60),
            backoff: Duration::from_millis(250),
        }
    }
}

/// The crashes of a session and what follows from them.
#[derive(Debug, Default)]
struct Health {
    /// When the process crashed, within the window.
    crashes: VecDeque<Instant>,
    /// The session gave up; carries the last stderr line of the last crash.
    failed: Option<Option<String>>,
    /// The earliest instant the next process may start.
    restart_at: Option<Instant>,
}

impl Health {
    /// Counts a crash at `now`: the crash after `max_restarts` fails the session, any other one
    /// delays the next start by `backoff` doubled once per crash within the window.
    fn record_crash(&mut self, policy: &RestartPolicy, now: Instant, stderr: Option<String>) {
        while self.crashes.front().is_some_and(|&at| now.duration_since(at) >= policy.window) {
            self.crashes.pop_front();
        }
        self.crashes.push_back(now);
        if self.crashes.len() > policy.max_restarts {
            self.failed = Some(stderr);
            return;
        }
        let doublings = u32::try_from(self.crashes.len() - 1).unwrap_or(u32::MAX);
        self.restart_at =
            Some(now + policy.backoff.saturating_mul(2_u32.saturating_pow(doublings)));
    }
}

/// The requests that wait for an answer, by id; `None` once the output of the process has ended.
type Pending = Option<HashMap<u64, UnboundedSender<Response>>>;

/// The requests in flight and the time of the last traffic.
struct Activity {
    /// Set when the process is gone or about to be: no request may register any more.
    dead: bool,
    in_flight: usize,
    last: Instant,
}

/// One running process and the handles to talk to it.
struct Proc {
    pending: Mutex<Pending>,
    writer: UnboundedSender<String>,
    /// Kill the process (the supervisor does it).
    kill: CancellationToken,
    /// Close the standard input of the process.
    close_input: CancellationToken,
    /// Whether the end of the process was asked for (by a timeout, a shutdown), as opposed to a
    /// crash.
    deliberate: AtomicBool,
    activity: Mutex<Activity>,
    exit: watch::Receiver<Option<Exit>>,
    stderr_last: Arc<Mutex<Option<String>>>,
    protocol_error: Mutex<Option<String>>,
}

impl Proc {
    /// Registers request `id`; `None` when the process can take no more requests.
    fn begin(&self, id: u64) -> Option<UnboundedReceiver<Response>> {
        let mut activity = lock(&self.activity);
        if activity.dead {
            return None;
        }
        let mut pending = lock(&self.pending);
        let (tx, rx) = unbounded_channel();
        pending.as_mut()?.insert(id, tx);
        activity.in_flight += 1;
        activity.last = Instant::now();
        Some(rx)
    }

    /// Forgets request `id`: its late answers are dropped.
    fn end(&self, id: u64) {
        if let Some(map) = lock(&self.pending).as_mut() {
            map.remove(&id);
        }
        let mut activity = lock(&self.activity);
        activity.in_flight = activity.in_flight.saturating_sub(1);
        activity.last = Instant::now();
    }

    fn send(&self, request: &Request) {
        // A closed channel means the process is gone; the request learns it from its own channel.
        let _ = self.writer.send(protocol::encode_line(request));
    }

    fn route(&self, response: Response) {
        lock(&self.activity).last = Instant::now();
        let id = response.id();
        let last = match &response {
            Response::Items { done, .. } => *done,
            Response::Result { .. } => true,
        };
        let mut pending = lock(&self.pending);
        let Some(map) = pending.as_mut() else { return };
        // An unknown id is a late answer to a cancelled request: dropped.
        if let Some(tx) = map.get(&id)
            && (tx.send(response).is_err() || last)
        {
            map.remove(&id);
        }
    }

    /// The output has ended: nobody waits for an answer any more.
    fn close_pending(&self) {
        *lock(&self.pending) = None;
    }

    /// When the process is next due for an idleness check: `idle` after the last traffic, or
    /// `idle` from now while a request is in flight.
    fn idle_wake(&self, idle: Duration) -> Instant {
        let activity = lock(&self.activity);
        if activity.in_flight > 0 { Instant::now() + idle } else { activity.last + idle }
    }

    /// Marks the process as stopped for idleness when no request has been in flight for `idle`;
    /// no request registers after that. Not a crash.
    fn idle_stop(&self, idle: Duration) -> bool {
        let mut activity = lock(&self.activity);
        if activity.dead || activity.in_flight > 0 || Instant::now() < activity.last + idle {
            return false;
        }
        activity.dead = true;
        self.deliberate.store(true, Ordering::SeqCst);
        true
    }

    /// Asks for the end of the process.
    fn stop(&self) {
        self.deliberate.store(true, Ordering::SeqCst);
        self.kill.cancel();
    }

    /// Waits until the supervisor has seen the process end.
    async fn wait_exit(&self) -> Exit {
        let mut exit = self.exit.clone();
        match exit.wait_for(Option::is_some).await {
            Ok(value) => (*value).unwrap_or(Exit { code: None }),
            Err(_) => Exit { code: None },
        }
    }

    async fn failure(&self) -> Failure {
        let exit = self.wait_exit().await;
        if let Some(message) = lock(&self.protocol_error).clone() {
            return Failure::Protocol(message);
        }
        Failure::Exit { code: exit.code, stderr: lock(&self.stderr_last).clone() }
    }
}

/// The tasks that belong to a process: write requests, read responses, and own the child.
async fn write_loop(
    mut stdin: ChildStdin,
    mut lines: UnboundedReceiver<String>,
    close: CancellationToken,
) {
    loop {
        let line = tokio::select! {
            biased;
            () = close.cancelled() => break,
            line = lines.recv() => line,
        };
        let Some(line) = line else { break };
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
    // `stdin` drops here: the script sees the end of its input.
}

async fn read_loop(stdout: ChildStdout, proc: Arc<Proc>, eof: CancellationToken) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        match protocol::parse_line::<Response>(&line) {
            Ok(None) => {}
            Ok(Some(response)) => proc.route(response),
            Err(error) => {
                *lock(&proc.protocol_error) = Some(error.to_string());
                proc.stop_as_crash();
                break;
            }
        }
    }
    proc.close_pending();
    eof.cancel();
}

impl Proc {
    /// Kills the process without calling it deliberate: the script broke the protocol.
    fn stop_as_crash(&self) {
        self.kill.cancel();
    }
}

/// What the supervisor of a process needs to know about its session.
struct Rules {
    health: Arc<Mutex<Health>>,
    policy: RestartPolicy,
    idle: Duration,
}

async fn supervise(
    mut child: Child,
    proc: Arc<Proc>,
    eof: CancellationToken,
    stderr: JoinHandle<()>,
    exit: watch::Sender<Option<Exit>>,
    rules: Rules,
) {
    let Rules { health, policy, idle } = rules;
    let status = loop {
        let wake = proc.idle_wake(idle);
        tokio::select! {
            status = child.wait() => break status,
            () = proc.kill.cancelled() => {
                let _ = child.kill().await;
                break child.wait().await;
            }
            () = eof.cancelled() => break match timeout(EOF_GRACE, child.wait()).await {
                Ok(status) => status,
                Err(_) => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            },
            () = sleep_until(wake) => {
                if !proc.idle_stop(idle) {
                    continue;
                }
                // Closing the input lets a well-behaved script end on its own.
                proc.close_input.cancel();
                break match timeout(IDLE_GRACE, child.wait()).await {
                    Ok(status) => status,
                    Err(_) => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                };
            }
        }
    };
    let _ = timeout(STDERR_GRACE, stderr).await;
    // An end nobody asked for is a crash. It is counted before the process counts as gone, so the
    // next request finds the backoff or the failed state.
    if !proc.deliberate.load(Ordering::SeqCst) {
        let last = lock(&proc.stderr_last).clone();
        lock(&health).record_crash(&policy, Instant::now(), last);
    }
    lock(&proc.activity).dead = true;
    proc.close_pending();
    let code = status.ok().and_then(|status| status.code());
    let _ = exit.send(Some(Exit { code }));
}

/// The thread and runtime that run the tasks of the processes of one session.
struct HostThread {
    handle: Handle,
    /// Dropping the sender ends the thread and with it the runtime and its processes.
    _stop: tokio::sync::oneshot::Sender<()>,
}

fn host_thread() -> std::io::Result<HostThread> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let handle = runtime.handle().clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    std::thread::Builder::new().name("daimos-script-session".into()).spawn(move || {
        runtime.block_on(async {
            let _ = stopped.await;
        });
    })?;
    Ok(HostThread { handle, _stop: stop })
}

struct Inner {
    spec: OneShot,
    timeout: Duration,
    policy: RestartPolicy,
    idle: Duration,
    health: Arc<Mutex<Health>>,
    next_id: AtomicU64,
    /// Serializes starting a process.
    gate: tokio::sync::Mutex<()>,
    current: Mutex<Option<Arc<Proc>>>,
    closed: AtomicBool,
    host: OnceLock<Option<HostThread>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(proc) = lock(&self.current).take() {
            proc.stop();
        }
    }
}

/// The long-running process of one script extension. Cloning shares the process.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("extension", &self.inner.spec.extension()).finish()
    }
}

impl Session {
    /// A session for `runtime` of the extension called `extension` in `dir`. No process starts
    /// before the first request.
    pub fn new(extension: &str, dir: PathBuf, runtime: &ScriptRuntime) -> Self {
        Self::from_spec(
            OneShot::new(extension, dir, runtime),
            RestartPolicy::default(),
            runtime.idle_timeout(),
        )
    }

    fn from_spec(spec: OneShot, policy: RestartPolicy, idle: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                timeout: spec.timeout(),
                spec,
                policy,
                idle,
                health: Arc::default(),
                next_id: AtomicU64::new(0),
                gate: tokio::sync::Mutex::new(()),
                current: Mutex::new(None),
                closed: AtomicBool::new(false),
                host: OnceLock::new(),
            }),
        }
    }

    /// The name of the extension.
    pub fn extension(&self) -> &str {
        self.inner.spec.extension()
    }

    /// Appends the stderr of the process to `path` (`<data dir>/logs/<extension>.log`).
    #[must_use]
    pub fn with_log(self, path: PathBuf) -> Self {
        Self::from_spec(
            OneShot::clone(&self.inner.spec).with_log(path),
            self.inner.policy,
            self.inner.idle,
        )
    }

    /// Replaces the default restart policy (three restarts per minute).
    #[must_use]
    pub fn with_restart_policy(self, policy: RestartPolicy) -> Self {
        Self::from_spec(OneShot::clone(&self.inner.spec), policy, self.inner.idle)
    }

    /// Stops the process and makes every later request end without an answer.
    pub fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Some(proc) = lock(&self.inner.current).take() {
            proc.stop();
        }
    }

    fn handle(&self) -> Result<Handle, Failure> {
        self.inner
            .host
            .get_or_init(|| {
                host_thread()
                    .map_err(|error| log::warn!("cannot start the script thread: {error}"))
                    .ok()
            })
            .as_ref()
            .map(|host| host.handle.clone())
            .ok_or_else(|| Failure::Spawn("cannot start the script thread".to_owned()))
    }

    /// The running process, started when there is none. `None` after a shutdown.
    async fn ensure(&self) -> Result<Option<Arc<Proc>>, Failure> {
        let _gate = self.inner.gate.lock().await;
        if let Some(proc) = lock(&self.inner.current).clone()
            && !lock(&proc.activity).dead
        {
            return Ok(Some(proc));
        }
        if self.inner.closed.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let restart_at = {
            let health = lock(&self.inner.health);
            if let Some(stderr) = &health.failed {
                return Err(Failure::KeepsCrashing(stderr.clone()));
            }
            health.restart_at
        };
        if let Some(at) = restart_at {
            sleep_until(at).await;
        }
        let proc = self.spawn()?;
        lock(&self.inner.health).restart_at = None;
        let mut current = lock(&self.inner.current);
        if self.inner.closed.load(Ordering::SeqCst) {
            proc.stop();
            return Ok(None);
        }
        *current = Some(proc.clone());
        Ok(Some(proc))
    }

    fn spawn(&self) -> Result<Arc<Proc>, Failure> {
        let handle = self.handle()?;
        // The pipes register with the runtime that is current while the process starts.
        let _entered = handle.enter();
        let spec = &self.inner.spec;
        let mut child = spec.spawn().map_err(|error| {
            Failure::Spawn(format!("cannot start {}: {error}", spec.command_name()))
        })?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(Failure::Spawn(format!("cannot talk to {}", spec.command_name())));
        };
        let (writer, lines) = unbounded_channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        let stderr_last = Arc::new(Mutex::new(None));
        let proc = Arc::new(Proc {
            pending: Mutex::new(Some(HashMap::new())),
            writer,
            kill: CancellationToken::new(),
            close_input: CancellationToken::new(),
            deliberate: AtomicBool::new(false),
            activity: Mutex::new(Activity { dead: false, in_flight: 0, last: Instant::now() }),
            exit: exit_rx,
            stderr_last: stderr_last.clone(),
            protocol_error: Mutex::new(None),
        });
        let eof = CancellationToken::new();
        let log = spec.log_path();
        let stderr = handle.spawn(async move {
            pump_stderr_into(stderr, log, &stderr_last).await;
        });
        handle.spawn(write_loop(stdin, lines, proc.close_input.clone()));
        handle.spawn(read_loop(stdout, proc.clone(), eof.clone()));
        handle.spawn(supervise(
            child,
            proc.clone(),
            eof,
            stderr,
            exit_tx,
            Rules {
                health: self.inner.health.clone(),
                policy: self.inner.policy,
                idle: self.inner.idle,
            },
        ));
        Ok(proc)
    }

    /// Sends the request built by `make` and returns the call that waits for its answer. `None`
    /// when `cancel` fired first or the session was shut down.
    async fn open(
        &self,
        make: impl FnOnce(u64) -> Request,
        cancel: &CancellationToken,
    ) -> Result<Option<Call>, Failure> {
        let deadline = Instant::now() + self.inner.timeout;
        let attempt = async {
            for _ in 0..ATTEMPTS {
                let Some(proc) = self.ensure().await? else { return Ok(None) };
                let id = self.next_id();
                if let Some(rx) = proc.begin(id) {
                    proc.send(&make(id));
                    return Ok(Some(Call {
                        session: self.clone(),
                        proc,
                        id,
                        rx,
                        deadline,
                        cancel: cancel.clone(),
                        finished: false,
                    }));
                }
                // The process is going away; wait for its end, then start a new one.
                proc.wait_exit().await;
            }
            Err(Failure::Exit { code: None, stderr: None })
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => Ok(None),
            () = sleep_until(deadline) => Err(Failure::Timeout(self.inner.timeout)),
            outcome = attempt => outcome,
        }
    }

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::SeqCst) + 1
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
        batches(self.inner.spec.extension(), self.query(command, text, preferences, cancel))
    }

    /// Sends one query to the process and streams what it answers until `done: true`. The process
    /// starts first if it is not running. When `cancel` fires a `cancel` message goes to the
    /// script and nothing more is yielded; a failure is the last event.
    pub fn query(
        &self,
        command: &str,
        text: &str,
        preferences: BTreeMap<String, String>,
        cancel: CancellationToken,
    ) -> BoxStream<'static, Event> {
        let start = QueryStart {
            session: self.clone(),
            command: command.to_owned(),
            text: text.to_owned(),
            preferences,
            cancel,
        };
        stream::unfold(QueryState::Start(Box::new(start)), advance).boxed()
    }

    /// Sends a custom action to the process and waits for its `result`; the answer is the toast.
    /// Cancelling ends the call without an answer.
    pub async fn action(
        &self,
        command: &str,
        item: &str,
        action: &str,
        cancel: CancellationToken,
    ) -> Result<Option<String>, Failure> {
        let make = |id| Request::Action {
            id,
            command: command.to_owned(),
            item: item.to_owned(),
            action: action.to_owned(),
        };
        let Some(mut call) = self.open(make, &cancel).await? else { return Ok(None) };
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    call.cancel();
                    return Ok(None);
                }
                () = sleep_until(call.deadline) => return Err(call.time_out().await),
                response = call.rx.recv() => match response {
                    Some(Response::Result { toast, .. }) => {
                        call.finished = true;
                        return Ok(toast);
                    }
                    Some(Response::Items { .. }) => {}
                    None => return Err(call.proc.failure().await),
                },
            }
        }
    }
}

/// One request in flight.
struct Call {
    session: Session,
    proc: Arc<Proc>,
    id: u64,
    rx: UnboundedReceiver<Response>,
    deadline: Instant,
    cancel: CancellationToken,
    /// The answer is complete: nothing to cancel.
    finished: bool,
}

impl Call {
    /// The request is superseded: tell the script to drop it.
    fn cancel(&mut self) {
        if !self.finished {
            self.finished = true;
            let id = self.session.next_id();
            self.proc.send(&Request::Cancel { id, cancels: self.id });
        }
    }

    /// The request ran out of time: the process is killed, the next request starts a new one.
    async fn time_out(&mut self) -> Failure {
        self.finished = true;
        self.proc.stop();
        let _ = timeout(STDERR_GRACE * 4, self.proc.wait_exit()).await;
        Failure::Timeout(self.session.inner.timeout)
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        self.proc.end(self.id);
    }
}

struct QueryStart {
    session: Session,
    command: String,
    text: String,
    preferences: BTreeMap<String, String>,
    cancel: CancellationToken,
}

enum QueryState {
    Start(Box<QueryStart>),
    Running(Box<Call>),
    Done,
}

async fn advance(state: QueryState) -> Option<(Event, QueryState)> {
    match state {
        QueryState::Done => None,
        QueryState::Start(start) => begin(*start).await,
        QueryState::Running(call) => pump(*call).await,
    }
}

async fn begin(start: QueryStart) -> Option<(Event, QueryState)> {
    let QueryStart { session, command, text, preferences, cancel } = start;
    let make = |id| Request::Query { id, command, text, preferences };
    match session.open(make, &cancel).await {
        Ok(Some(call)) => pump(call).await,
        Ok(None) => None,
        Err(failure) => Some((Event::Failed(failure), QueryState::Done)),
    }
}

async fn pump(mut call: Call) -> Option<(Event, QueryState)> {
    loop {
        tokio::select! {
            biased;
            () = call.cancel.cancelled() => {
                call.cancel();
                return None;
            }
            () = sleep_until(call.deadline) => {
                let failure = call.time_out().await;
                return Some((Event::Failed(failure), QueryState::Done));
            }
            response = call.rx.recv() => match response {
                Some(Response::Items { items, done, .. }) => {
                    let items: Vec<model::Item> = items.into_iter().map(Into::into).collect();
                    if done {
                        call.finished = true;
                    }
                    match (items.is_empty(), done) {
                        (false, true) => return Some((Event::Items(items), QueryState::Done)),
                        (false, false) => {
                            return Some((Event::Items(items), QueryState::Running(Box::new(call))));
                        }
                        (true, true) => return None,
                        (true, false) => {}
                    }
                }
                Some(Response::Result { .. }) => {}
                None => {
                    let failure = call.proc.failure().await;
                    return Some((Event::Failed(failure), QueryState::Done));
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crash(health: &mut Health, policy: &RestartPolicy, at: Instant) {
        health.record_crash(policy, at, Some("boom".into()));
    }

    #[test]
    fn the_backoff_doubles_per_crash_within_the_window() {
        let policy = RestartPolicy::default();
        let mut health = Health::default();
        let t0 = Instant::now();
        let mut delays = vec![];
        for n in 0..3 {
            let at = t0 + Duration::from_secs(n);
            crash(&mut health, &policy, at);
            delays.push(health.restart_at.unwrap().duration_since(at));
        }
        assert_eq!(
            delays,
            [Duration::from_millis(250), Duration::from_millis(500), Duration::from_secs(1)]
        );
        assert_eq!(health.failed, None);
    }

    #[test]
    fn the_fourth_crash_within_a_minute_fails_the_session_with_the_last_stderr_line() {
        let policy = RestartPolicy::default();
        let mut health = Health::default();
        let t0 = Instant::now();
        for n in 0..4 {
            crash(&mut health, &policy, t0 + Duration::from_secs(n * 10));
        }
        assert_eq!(health.failed, Some(Some("boom".to_owned())));
    }

    #[test]
    fn old_crashes_expire_as_the_window_slides() {
        let policy = RestartPolicy::default();
        let mut health = Health::default();
        let t0 = Instant::now();
        for n in 0..3 {
            crash(&mut health, &policy, t0 + Duration::from_secs(n));
        }
        // The first three are more than a minute old: this is the first crash of a new window.
        let later = t0 + Duration::from_secs(63);
        crash(&mut health, &policy, later);
        assert_eq!(health.failed, None);
        assert_eq!(health.crashes.len(), 1);
        assert_eq!(health.restart_at, Some(later + Duration::from_millis(250)));
    }

    #[test]
    fn a_crash_exactly_one_window_old_no_longer_counts() {
        let policy = RestartPolicy { max_restarts: 1, ..RestartPolicy::default() };
        let mut health = Health::default();
        let t0 = Instant::now();
        crash(&mut health, &policy, t0);
        crash(&mut health, &policy, t0 + policy.window);
        assert_eq!(health.failed, None);
    }

    #[test]
    fn no_restarts_allowed_fails_at_the_first_crash() {
        let policy = RestartPolicy { max_restarts: 0, ..RestartPolicy::default() };
        let mut health = Health::default();
        crash(&mut health, &policy, Instant::now());
        assert!(health.failed.is_some());
    }
}
