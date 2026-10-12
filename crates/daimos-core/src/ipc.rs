//! The local socket between the resident process and the `daimos` CLI.
//!
//! One JSON object per line in each direction: the client sends a [`Request`], the server answers
//! with a [`Response`] and closes the connection. The socket is a Unix socket on Linux and macOS
//! and a named pipe on Windows, reachable by the current user only.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use interprocess::local_socket::traits::{ListenerExt as _, Stream as _};
use interprocess::local_socket::{ListenerOptions, Name, Stream};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Toggle,
    Show {
        /// Text to put into the search field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
    },
    Hide,
    Quit,
    /// Scan every application source now and wait for the result.
    IndexRebuild,
    IndexStatus,
    /// List the extensions with their state.
    ExtList,
}

/// A filesystem path an extension asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FilesystemAccess {
    pub path: String,
    /// `false` for read only.
    pub write: bool,
}

/// What an extension asks for or what the user confirmed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CapabilityList {
    pub network: Vec<String>,
    pub filesystem: Vec<FilesystemAccess>,
    /// `read`, `write` or `read-write`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<String>,
    pub keyring: Vec<String>,
}

/// A preference of an extension. The value of a password is never part of this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PreferenceInfo {
    pub name: String,
    pub title: String,
    /// `text`, `password`, `number` or `checkbox`.
    pub kind: String,
    pub required: bool,
    /// Whether a value is stored. A password is only ever reported as set or not.
    pub is_set: bool,
    /// The stored value of a text, number or checkbox preference; always `None` for a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// One extension in the answer to `ext_list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ExtensionInfo {
    pub name: String,
    /// The `title` of the manifest; `None` when the manifest could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `None` when the manifest could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `script` or `wasm`; `None` when the manifest could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// `true` for wasm; a script runs with the user's rights.
    pub sandboxed: bool,
    /// `enabled`, `needs-confirmation` or `error`.
    pub state: String,
    /// Why the extension is disabled, when `state` is `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// What the manifest declares.
    pub capabilities: CapabilityList,
    /// What the user confirmed; `None` before the first confirmation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<CapabilityList>,
    /// What waits for a confirmation: everything before the first one, then what an update added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<CapabilityList>,
    pub preferences: Vec<PreferenceInfo>,
}

/// The answer to `ext_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ExtensionList {
    pub extensions: Vec<ExtensionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok,
    Error {
        message: String,
    },
    /// The answer to `index_rebuild` and `index_status`.
    Index(crate::index::IndexStatus),
    /// The answer to `ext_list`.
    Extensions(ExtensionList),
}

/// Where the socket lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A socket file (Linux, macOS).
    Path(PathBuf),
    /// A named pipe (Windows), for example `daimos` for `\\.\pipe\daimos`.
    Pipe(String),
}

impl Endpoint {
    /// `$XDG_RUNTIME_DIR/daimos.sock` on Linux, `~/Library/Application Support/daimos/daimos.sock`
    /// on macOS, and the pipe `\\.\pipe\daimos` on Windows.
    pub fn default_for_user() -> io::Result<Self> {
        if cfg!(windows) {
            return Ok(Self::Pipe("daimos".into()));
        }
        let dir = if cfg!(target_os = "macos") {
            dirs::data_dir().map(|d| d.join("daimos"))
        } else {
            dirs::runtime_dir().or_else(|| dirs::cache_dir().map(|d| d.join("daimos")))
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no runtime directory"))?;
        Ok(Self::Path(dir.join("daimos.sock")))
    }

    fn name(&self) -> io::Result<Name<'static>> {
        use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ToFsName, ToNsName};
        match self {
            Self::Path(path) => path.clone().to_fs_name::<GenericFilePath>(),
            Self::Pipe(name) => name.clone().to_ns_name::<GenericNamespaced>(),
        }
    }
}

/// How long a stop waits for the accept thread before giving up on it.
const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// What the resident process does with a request.
pub type Handler = dyn Fn(Request) -> Response + Send + Sync + 'static;

/// The listening side. Dropping or [`stop`](Server::stop)ping it removes the socket.
pub struct Server {
    endpoint: Endpoint,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Signalled by the accept thread when it ends.
    finished: std::sync::mpsc::Receiver<()>,
}

impl Server {
    pub fn start(
        endpoint: Endpoint,
        handler: impl Fn(Request) -> Response + Send + Sync + 'static,
    ) -> io::Result<Self> {
        #[cfg(unix)]
        if let Endpoint::Path(path) = &endpoint
            && let Some(dir) = path.parent()
            && !dir.exists()
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }

        // A live listener means another Daimos runs; taking its socket would orphan it.
        if Stream::connect(endpoint.name()?).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, "Daimos is already running"));
        }
        let options = ListenerOptions::new().name(endpoint.name()?);
        // A corpse of a crashed run would make the address look taken.
        let options = options.try_overwrite(true);
        // Only some Unixes can set the mode at creation (macOS answers `Unsupported`).
        #[cfg(target_os = "linux")]
        let options = {
            use interprocess::os::unix::local_socket::ListenerOptionsExt;
            options.mode(0o600)
        };
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            // A protected DACL that gives full access to the owner and SYSTEM, nobody else.
            let sd = SecurityDescriptor::deserialize(widestring::u16cstr!(
                "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
            ))?;
            options.security_descriptor(sd)
        };
        let listener = options.create_sync()?;
        #[cfg(all(unix, not(target_os = "linux")))]
        if let Endpoint::Path(path) = &endpoint {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }

        let stopping = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopping);
        let handler: Arc<Handler> = Arc::new(handler);
        let (done, finished) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new().name("daimos-ipc".into()).spawn(move || {
            for connection in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                match connection {
                    Ok(connection) => {
                        let handler = Arc::clone(&handler);
                        // A client that connects and says nothing must not block the others.
                        std::thread::spawn(move || {
                            if let Err(error) = serve(connection, handler.as_ref()) {
                                log::debug!("ipc connection ended: {error}");
                            }
                        });
                    }
                    Err(error) => log::warn!("ipc accept failed: {error}"),
                }
            }
            let _ = done.send(());
        })?;
        Ok(Self { endpoint, stopping, thread: Some(thread), finished })
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Stops accepting and removes the socket.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        self.stopping.store(true, Ordering::SeqCst);
        // `accept` blocks, so wake it with a connection that is dropped at once. The connect runs
        // on its own thread: on some systems it can block, and quitting must not. A single wake-up
        // can be lost under load (seen with named pipes on Windows), so it is repeated until the
        // accept thread reports back or the wait is over.
        let name = self.endpoint.name().ok();
        let deadline = std::time::Instant::now() + SHUTDOWN_WAIT;
        let mut ended = false;
        while !ended && std::time::Instant::now() < deadline {
            if let Some(name) = name.clone() {
                std::thread::spawn(move || {
                    let _ = Stream::connect(name);
                });
            }
            ended = self.finished.recv_timeout(Duration::from_millis(100)).is_ok();
        }
        // If the accept thread does not end (socket file removed, blocked connect), detach it.
        if ended {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn serve(connection: Stream, handler: &Handler) -> io::Result<()> {
    let mut reader = BufReader::new(connection);
    let mut line = String::new();
    // One request is a short line; cap it so a client cannot grow the buffer without bound.
    (&mut reader).take(MAX_REQUEST_BYTES).read_line(&mut line)?;
    let response = match serde_json::from_str::<Request>(line.trim()) {
        Ok(request) => handler(request),
        Err(error) => Response::Error { message: format!("bad request: {error}") },
    };
    let mut out = serde_json::to_vec(&response).expect("response serializes");
    out.push(b'\n');
    reader.get_mut().write_all(&out)
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Nobody listens on the socket: Daimos is not running.
    #[error("Daimos is not running")]
    NotRunning,
    #[error("cannot talk to Daimos: {0}")]
    Io(#[from] io::Error),
    #[error("Daimos sent an answer that is not understood: {0}")]
    BadResponse(String),
}

/// Sends one request and waits for the answer.
pub fn send(endpoint: &Endpoint, request: &Request) -> Result<Response, ClientError> {
    // Whatever stops the connect (no file, no listener, a missing directory) means nobody is
    // there to talk to; only a permission problem is worth reporting as such.
    let connection = Stream::connect(endpoint.name()?).map_err(|error| match error.kind() {
        io::ErrorKind::PermissionDenied => ClientError::Io(error),
        _ => ClientError::NotRunning,
    })?;
    let mut reader = BufReader::new(connection);
    let mut out = serde_json::to_vec(request).expect("request serializes");
    out.push(b'\n');
    reader.get_mut().write_all(&out)?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str(line.trim()).map_err(|_| ClientError::BadResponse(line))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Instant;

    use super::*;

    fn endpoint(dir: &tempfile::TempDir) -> Endpoint {
        if cfg!(windows) {
            // Tests run in parallel threads of one process: every test needs its own pipe.
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::SeqCst);
            Endpoint::Pipe(format!("daimos-test-{}-{n}", std::process::id()))
        } else {
            Endpoint::Path(dir.path().join("daimos.sock"))
        }
    }

    #[test]
    fn requests_serialize_as_one_flat_object() {
        let json = serde_json::to_string(&Request::Show { query: Some("a b".into()) }).unwrap();
        assert_eq!(json, r#"{"cmd":"show","query":"a b"}"#);
        assert_eq!(serde_json::to_string(&Request::Toggle).unwrap(), r#"{"cmd":"toggle"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"show"}"#).unwrap(),
            Request::Show { query: None }
        );
    }

    #[test]
    fn index_requests_and_the_status_answer_round_trip() {
        assert_eq!(
            serde_json::to_string(&Request::IndexRebuild).unwrap(),
            r#"{"cmd":"index_rebuild"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"index_status"}"#).unwrap(),
            Request::IndexStatus
        );
        let status = crate::index::IndexStatus {
            apps: 3,
            sources: vec![crate::index::SourceStatus { source: "linux".into(), apps: 3 }],
            indexing: false,
            last_indexed: 7,
        };
        let json = serde_json::to_string(&Response::Index(status.clone())).unwrap();
        assert!(json.starts_with(r#"{"status":"index","apps":3"#), "{json}");
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), Response::Index(status));
    }

    #[test]
    fn the_extension_list_round_trips() {
        assert_eq!(serde_json::to_string(&Request::ExtList).unwrap(), r#"{"cmd":"ext_list"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"ext_list"}"#).unwrap(),
            Request::ExtList
        );
        let list = ExtensionList {
            extensions: vec![
                ExtensionInfo {
                    name: "notes".into(),
                    version: Some("0.1.0".into()),
                    runtime: Some("script".into()),
                    state: "enabled".into(),
                    message: None,
                    ..ExtensionInfo::default()
                },
                ExtensionInfo {
                    name: "broken".into(),
                    version: None,
                    runtime: None,
                    state: "error".into(),
                    message: Some("extension.toml:2: bad".into()),
                    ..ExtensionInfo::default()
                },
            ],
        };
        let json = serde_json::to_string(&Response::Extensions(list.clone())).unwrap();
        assert!(
            json.starts_with(r#"{"status":"extensions","extensions":[{"name":"notes""#),
            "{json}"
        );
        assert!(!json.contains("null"), "{json}");
        assert_eq!(serde_json::from_str::<Response>(&json).unwrap(), Response::Extensions(list));
    }

    #[test]
    fn capabilities_sandbox_label_and_preferences_round_trip() {
        let info = ExtensionInfo {
            name: "gh".into(),
            sandboxed: true,
            state: "needs-confirmation".into(),
            capabilities: CapabilityList {
                network: vec!["api.github.com".into()],
                filesystem: vec![FilesystemAccess { path: "/x".into(), write: false }],
                clipboard: Some("write".into()),
                keyring: vec!["token".into()],
            },
            added: Some(CapabilityList::default()),
            preferences: vec![PreferenceInfo {
                name: "token".into(),
                title: "Token".into(),
                kind: "password".into(),
                required: true,
                is_set: true,
                value: None,
            }],
            ..ExtensionInfo::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(
            json.contains(r#""sandboxed":true"#) && json.contains(r#""is_set":true"#),
            "{json}"
        );
        assert!(!json.contains(r#""value""#), "a password has no value on the wire: {json}");
        assert_eq!(serde_json::from_str::<ExtensionInfo>(&json).unwrap(), info);
    }

    #[test]
    fn a_request_reaches_the_handler_and_the_answer_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let server = Server::start(endpoint(&dir), move |request| {
            log.lock().unwrap().push(request);
            Response::Ok
        })
        .unwrap();

        let answer = send(server.endpoint(), &Request::Show { query: Some("firefox".into()) });
        assert_eq!(answer.unwrap(), Response::Ok);
        assert_eq!(*seen.lock().unwrap(), [Request::Show { query: Some("firefox".into()) }]);
        server.stop();
    }

    // Windows pipes have no overwrite, but a second listener there is not refused either; the
    // app keeps a single instance with tauri-plugin-single-instance.
    #[cfg(unix)]
    #[test]
    fn a_second_instance_does_not_take_over_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let first = Server::start(endpoint(&dir), |_| Response::Ok).unwrap();
        let second =
            Server::start(endpoint(&dir), |_| Response::Error { message: "second".into() });
        assert_eq!(second.err().map(|e| e.kind()), Some(io::ErrorKind::AddrInUse));
        assert_eq!(send(first.endpoint(), &Request::Hide).unwrap(), Response::Ok);
    }

    #[test]
    fn nobody_listening_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let result = send(&endpoint(&dir), &Request::Toggle);
        assert!(matches!(result, Err(ClientError::NotRunning)), "{result:?}");
    }

    #[test]
    fn stopping_the_server_makes_it_not_running_again() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(endpoint(&dir), |_| Response::Ok).unwrap();
        let endpoint = server.endpoint().clone();
        server.stop();
        assert!(matches!(send(&endpoint, &Request::Hide), Err(ClientError::NotRunning)));
    }

    #[test]
    fn garbage_gets_an_error_answer_and_the_server_lives_on() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(endpoint(&dir), |_| Response::Ok).unwrap();
        let mut raw = BufReader::new(Stream::connect(server.endpoint().name().unwrap()).unwrap());
        raw.get_mut().write_all(b"not json\n").unwrap();
        let mut line = String::new();
        raw.read_line(&mut line).unwrap();
        assert!(line.contains(r#""status":"error""#), "{line}");
        assert_eq!(send(server.endpoint(), &Request::Hide).unwrap(), Response::Ok);
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_reachable_by_the_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(endpoint(&dir), |_| Response::Ok).unwrap();
        let Endpoint::Path(path) = server.endpoint() else { unreachable!() };
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "socket mode is {mode:o}");
    }

    #[test]
    fn a_round_trip_takes_well_under_ten_milliseconds() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(endpoint(&dir), |_| Response::Ok).unwrap();
        send(server.endpoint(), &Request::Toggle).unwrap(); // warm up
        let runs = 50;
        let start = Instant::now();
        for _ in 0..runs {
            send(server.endpoint(), &Request::Toggle).unwrap();
        }
        let average = start.elapsed() / runs;
        assert!(average.as_millis() < 10, "average round trip {average:?}");
    }
}
