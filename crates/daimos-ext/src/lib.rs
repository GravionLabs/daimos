//! Extension host: manifest parsing, script host, wasm host and capability checks (milestone M2).

pub mod dir;
pub mod error;
pub mod manifest;
pub mod protocol;
pub mod provider;
pub mod registry;
pub mod script;
pub mod watch;

pub use error::ManifestError;
pub use manifest::{
    Capabilities, ClipboardAccess, Command, CommandMode, Manifest, Preference, PreferenceKind,
    Runtime, RuntimeTables, ScriptRuntime, WasmRuntime,
};
pub use provider::{Backend, CommandProvider, ErrorProvider, StubBackend, providers_for};
pub use registry::{ExtensionRecord, ExtensionState, Registry};
pub use watch::{ExtensionHost, ExtensionWatcher};
