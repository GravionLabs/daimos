//! Extension host: manifest parsing, script host, wasm host and capability checks (milestone M2).

pub mod dir;
pub mod error;
pub mod manifest;

pub use error::ManifestError;
pub use manifest::{
    Capabilities, ClipboardAccess, Command, CommandMode, Manifest, Preference, PreferenceKind,
    Runtime, RuntimeTables, ScriptRuntime, WasmRuntime,
};
