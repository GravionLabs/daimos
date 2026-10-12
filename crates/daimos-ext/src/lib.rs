//! Extension host: manifest parsing, script host, wasm host and capability checks (milestone M2).

pub mod manifest;

pub use manifest::{
    Capabilities, ClipboardAccess, Command, CommandMode, Manifest, ParseError, Preference,
    PreferenceKind, Runtime, RuntimeTables, ScriptRuntime, WasmRuntime,
};
