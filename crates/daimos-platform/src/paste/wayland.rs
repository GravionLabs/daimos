//! Ctrl+V on Wayland through a tool that talks to the compositor: `wtype` (virtual-keyboard
//! protocol: sway, Hyprland, niri, KDE) or `ydotool` (uinput, needs its daemon). GNOME has neither
//! protocol, so there only `ydotool` can work.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use daimos_core::paste::{PasteSupport, Paster};

/// The tools in the order they are tried.
const TOOLS: &[(&str, &[&str])] = &[
    ("wtype", &["-M", "ctrl", "v", "-m", "ctrl"]),
    // KEY_LEFTCTRL = 29, KEY_V = 47
    ("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
];

const HINT: &str = "install wtype (or ydotool) to paste into other windows";

pub struct WaylandPaster {
    path: Vec<PathBuf>,
}

impl WaylandPaster {
    pub fn from_env() -> Self {
        Self::with_path(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect(),
        )
    }

    /// Looks for the tools in `path`.
    pub fn with_path(path: Vec<PathBuf>) -> Self {
        Self { path }
    }

    /// The first installed tool with its arguments.
    pub fn command(&self) -> Option<(PathBuf, &'static [&'static str])> {
        TOOLS.iter().find_map(|(name, args)| {
            self.path
                .iter()
                .map(|dir| dir.join(name))
                .find(|p| is_executable(p))
                .map(|p| (p, *args))
        })
    }
}

impl Paster for WaylandPaster {
    fn id(&self) -> &str {
        "wayland"
    }

    fn support(&self) -> PasteSupport {
        match self.command() {
            Some(_) => PasteSupport::Full,
            None => PasteSupport::Unsupported(HINT.into()),
        }
    }

    fn paste(&self) -> io::Result<()> {
        let (program, args) =
            self.command().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, HINT))?;
        let output = Command::new(&program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(io::Error::other(format!(
                "{} failed: {}",
                program.display(),
                stderr.trim().lines().next().unwrap_or("no message")
            )))
        }
    }
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn dir_with(names: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[test]
    fn wtype_comes_before_ydotool() {
        let dir = dir_with(&["ydotool", "wtype"]);
        let paster = WaylandPaster::with_path(vec![dir.path().to_owned()]);
        let (program, args) = paster.command().unwrap();
        assert_eq!(program, dir.path().join("wtype"));
        assert_eq!(args, ["-M", "ctrl", "v", "-m", "ctrl"]);
        assert_eq!(paster.support(), PasteSupport::Full);
    }

    #[test]
    fn ydotool_sends_the_key_codes_of_ctrl_v() {
        let dir = dir_with(&["ydotool"]);
        let (_, args) = WaylandPaster::with_path(vec![dir.path().to_owned()]).command().unwrap();
        assert_eq!(args, ["key", "29:1", "47:1", "47:0", "29:0"]);
    }

    #[test]
    fn without_a_tool_it_says_what_to_install() {
        let dir = dir_with(&["something-else"]);
        let paster = WaylandPaster::with_path(vec![dir.path().to_owned()]);
        assert_eq!(paster.support(), PasteSupport::Unsupported(HINT.into()));
        assert_eq!(paster.paste().unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_failing_tool_is_an_error_with_its_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wtype");
        std::fs::write(&path, "#!/bin/sh\necho 'no virtual keyboard' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = WaylandPaster::with_path(vec![dir.path().to_owned()]).paste().unwrap_err();
        assert!(error.to_string().contains("no virtual keyboard"), "{error}");
    }
}
