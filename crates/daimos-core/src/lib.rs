//! Domain logic of Daimos without Tauri: providers, matcher, ranker, index, actions, settings.
//! It runs headless (CLI, benchmarks, CI), so it must never depend on `tauri`.

pub mod app_provider;
pub mod apps;
pub mod frecency;
pub mod index;
pub mod ipc;
pub mod model;
pub mod provider;
pub mod query;
pub mod ranker;
pub mod settings;

pub use model::{Action, ActionKind, Detail, Icon, Item, ItemBatch, View};
pub use provider::{Provider, QueryContext};
pub use query::{QueryEngine, ResultBatch};
