//! Extension host: manifest parsing, script host, wasm host and capability checks (milestone M2).

pub mod builtin;
pub mod consent;
pub mod dir;
pub mod error;
pub mod manifest;
pub mod preferences;
pub mod protocol;
pub mod provider;
pub mod registry;
pub mod script;
pub mod secrets;
pub mod session;
pub mod watch;

pub use error::ManifestError;
pub use manifest::{
    Capabilities, ClipboardAccess, Command, CommandMode, Manifest, Preference, PreferenceKind,
    Runtime, RuntimeTables, ScriptRuntime, WasmRuntime,
};
pub use preferences::{PreferenceError, PreferenceStatus, Preferences};
pub use provider::{
    Backend, CommandProvider, ErrorProvider, StubBackend, error_item, providers_for,
};
pub use registry::{ExtensionRecord, ExtensionState, Registry};
pub use script::{Failure, OneShot, ScriptBackend};
pub use secrets::{MockSecretStore, OsSecretStore, SecretError, SecretStore};
pub use session::{RestartPolicy, Session};
pub use watch::{ExtensionHost, ExtensionWatcher};
