//! Installed applications: what an OS-specific source reports and how it launches them.
//!
//! The sources live in `daimos-platform` (one per OS); the index and the app provider in this
//! crate only see this trait.

use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// An extra action of an application, for example "New Private Window".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppAction {
    /// Unique within the application.
    pub id: String,
    pub name: String,
    /// A command line without field codes; how it is run depends on the source.
    pub exec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    /// Stable across reindexes and renames, prefixed with the source (`linux:firefox.desktop`),
    /// so frecency survives a changed display name.
    pub id: String,
    pub name: String,
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    /// The command line with the field codes stripped.
    pub exec: String,
    /// Runs in a terminal window.
    pub terminal: bool,
    /// A PNG or SVG file.
    pub icon: Option<PathBuf>,
    pub actions: Vec<AppAction>,
}

pub trait AppSource: Send + Sync {
    /// `linux`, `windows`, or `macos`.
    fn id(&self) -> &str;

    /// Every launchable application, sorted by name then id.
    fn scan(&self) -> Vec<AppEntry>;

    /// Directories whose changes mean the list may have changed.
    fn watch_paths(&self) -> Vec<PathBuf>;

    /// Starts the application, or one of its actions, detached from this process.
    fn launch(&self, app: &AppEntry, action: Option<&str>) -> io::Result<()>;
}
