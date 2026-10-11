//! Applications from freedesktop `.desktop` entries (XDG, Flatpak, Snap).
//!
//! Follows the Desktop Entry Specification: `NoDisplay`, `Hidden`, `OnlyShowIn`, `NotShowIn` and
//! `TryExec` decide whether an entry is listed, `Name`, `GenericName` and `Keywords` are
//! localized, field codes are stripped from `Exec`, and icons are resolved through the icon theme
//! lookup.

use std::collections::HashSet;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use daimos_core::apps::{AppAction, AppEntry, AppSource};
use freedesktop_desktop_entry::DesktopEntry;

const FALLBACK_ICON: &str = "application-x-executable";
const ICON_SIZE: u16 = 64;

/// Terminal emulators and the flag that makes them run a command, in order of preference when
/// neither `$TERMINAL` nor `x-terminal-emulator` is available.
const TERMINALS: [(&str, &str); 9] = [
    ("gnome-terminal", "--"),
    ("kgx", "--"),
    ("konsole", "-e"),
    ("xfce4-terminal", "-x"),
    ("alacritty", "-e"),
    ("kitty", ""),
    ("wezterm", "start --"),
    ("foot", ""),
    ("xterm", "-e"),
];

pub struct LinuxAppSource {
    /// Directories that contain `.desktop` files, highest priority first.
    dirs: Vec<PathBuf>,
    /// Locales in order of preference, for example `de_DE`, `de`.
    locales: Vec<String>,
    /// `XDG_CURRENT_DESKTOP` split at `:`, lowercase.
    desktops: Vec<String>,
    /// Directories searched for `TryExec`.
    path: Vec<PathBuf>,
    icon_theme: Option<String>,
}

impl LinuxAppSource {
    /// The directories, locale and desktop of the running session.
    pub fn new() -> Self {
        let mut dirs: Vec<PathBuf> = freedesktop_desktop_entry::default_paths().collect();
        // Flatpak and Snap export their entries outside the XDG data directories unless the
        // session adds them (many do not).
        dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
        }
        dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
        Self::with_dirs(dirs)
    }

    /// Scans `dirs` only; the locale, desktop and `PATH` still come from the environment.
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        let desktops = freedesktop_desktop_entry::current_desktop().unwrap_or_default();
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        Self {
            dirs,
            locales: freedesktop_desktop_entry::get_languages_from_env(),
            desktops,
            path,
            icon_theme: freedesktop_icons::default_theme_gtk(),
        }
    }

    pub fn locales(mut self, locales: Vec<String>) -> Self {
        self.locales = locales;
        self
    }

    pub fn desktops(mut self, desktops: Vec<String>) -> Self {
        self.desktops = desktops.into_iter().map(|d| d.to_lowercase()).collect();
        self
    }

    pub fn search_path(mut self, path: Vec<PathBuf>) -> Self {
        self.path = path;
        self
    }

    fn entry_for(&self, id: &str, path: &Path) -> Option<AppEntry> {
        let entry = DesktopEntry::from_path(path, Some(&self.locales)).ok()?;
        if entry.type_() != Some("Application") || entry.no_display() || entry.hidden() {
            return None;
        }
        if !self.shown_in_this_desktop(&entry) || !self.try_exec_found(&entry) {
            return None;
        }
        let name = entry.name(&self.locales)?.trim().to_owned();
        let exec = strip_field_codes(entry.exec()?);
        if name.is_empty() || exec.is_empty() {
            return None;
        }
        let generic_name = entry
            .generic_name(&self.locales)
            .map(|g| g.trim().to_owned())
            .filter(|g| !g.is_empty() && *g != name);
        let keywords = entry
            .keywords(&self.locales)
            .map(|k| k.iter().map(|k| k.trim().to_owned()).filter(|k| !k.is_empty()).collect())
            .unwrap_or_default();
        let actions = entry
            .actions()
            .into_iter()
            .flatten()
            .filter(|a| !a.is_empty())
            .filter_map(|action| {
                let name = entry.action_name(action, &self.locales)?;
                let exec = strip_field_codes(entry.action_exec(action)?);
                (!exec.is_empty()).then(|| AppAction {
                    id: action.to_owned(),
                    name: name.trim().to_owned(),
                    exec,
                })
            })
            .collect();
        Some(AppEntry {
            id: format!("linux:{id}"),
            name,
            generic_name,
            keywords,
            exec,
            terminal: entry.terminal(),
            icon: self.icon(entry.icon()),
            actions,
        })
    }

    /// `OnlyShowIn` lists the desktops that show the entry, `NotShowIn` those that do not.
    fn shown_in_this_desktop(&self, entry: &DesktopEntry) -> bool {
        let lists = |names: Vec<&str>| {
            names.iter().any(|n| self.desktops.iter().any(|d| d.eq_ignore_ascii_case(n)))
        };
        if let Some(only) = entry.only_show_in()
            && !only.is_empty()
            && !lists(only)
        {
            return false;
        }
        !entry.not_show_in().is_some_and(lists)
    }

    /// `TryExec` names a program that must exist, as an absolute path or on `PATH`.
    fn try_exec_found(&self, entry: &DesktopEntry) -> bool {
        let Some(try_exec) = entry.try_exec().map(str::trim).filter(|t| !t.is_empty()) else {
            return true;
        };
        let candidate = Path::new(try_exec);
        if candidate.is_absolute() {
            return is_executable(candidate);
        }
        self.path.iter().any(|dir| is_executable(&dir.join(try_exec)))
    }

    /// An absolute `Icon` is used as is; a name goes through the theme lookup, then falls back to
    /// the generic application icon. The result is the real file: icon themes are full of
    /// (relative) symlinks, and the web view's asset protocol cannot follow those.
    fn icon(&self, icon: Option<&str>) -> Option<PathBuf> {
        let found = (|| {
            if let Some(icon) = icon.map(str::trim).filter(|i| !i.is_empty()) {
                let path = Path::new(icon);
                if path.is_absolute() {
                    if path.is_file() {
                        return Some(path.to_owned());
                    }
                } else if let Some(found) = self.lookup(icon) {
                    return Some(found);
                }
            }
            self.lookup(FALLBACK_ICON)
        })()?;
        Some(std::fs::canonicalize(&found).unwrap_or(found))
    }

    fn lookup(&self, name: &str) -> Option<PathBuf> {
        let mut lookup = freedesktop_icons::lookup(name).with_size(ICON_SIZE).with_cache();
        if let Some(theme) = &self.icon_theme {
            lookup = lookup.with_theme(theme);
        }
        lookup.find()
    }
}

impl Default for LinuxAppSource {
    fn default() -> Self {
        Self::new()
    }
}

impl AppSource for LinuxAppSource {
    fn id(&self) -> &str {
        "linux"
    }

    fn scan(&self) -> Vec<AppEntry> {
        let mut seen = HashSet::new();
        let mut apps = Vec::new();
        // Earlier directories win: a user entry hides the system entry with the same id.
        for dir in &self.dirs {
            for (id, path) in desktop_files(dir) {
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(app) = self.entry_for(&id, &path) {
                    apps.push(app);
                }
            }
        }
        apps.sort_by(|a, b| {
            a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id))
        });
        apps
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        self.dirs.iter().filter(|d| d.is_dir()).cloned().collect()
    }

    fn launch(&self, app: &AppEntry, action: Option<&str>) -> io::Result<()> {
        let (exec, terminal) = match action {
            Some(id) => {
                let action = app.actions.iter().find(|a| a.id == id).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, format!("no action {id}"))
                })?;
                (action.exec.as_str(), app.terminal)
            }
            None => (app.exec.as_str(), app.terminal),
        };
        let mut argv = split_exec(exec);
        if argv.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty Exec line"));
        }
        if terminal {
            argv = self.in_terminal(argv)?;
        }
        spawn_detached(&argv)
    }
}

impl LinuxAppSource {
    /// Prefixes `argv` with the terminal emulator that runs it. The emulator is resolved against
    /// this source's search path, so it is found even when the process `PATH` differs.
    fn in_terminal(&self, argv: Vec<String>) -> io::Result<Vec<String>> {
        let mut terminal = terminal_prefix(&self.path, std::env::var("TERMINAL").ok().as_deref())
            .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no terminal emulator found")
        })?;
        if !Path::new(&terminal[0]).is_absolute()
            && let Some(found) =
                self.path.iter().map(|d| d.join(&terminal[0])).find(|p| is_executable(p))
        {
            terminal[0] = found.to_string_lossy().into_owned();
        }
        Ok(terminal.into_iter().chain(argv).collect())
    }
}

/// The terminal emulator command that runs the arguments appended to it: `$TERMINAL`,
/// `x-terminal-emulator`, or the first known emulator on `PATH`.
pub fn terminal_prefix(path: &[PathBuf], terminal_env: Option<&str>) -> Option<Vec<String>> {
    let exists = |name: &str| path.iter().any(|d| is_executable(&d.join(name)));
    if let Some(term) = terminal_env.map(str::trim).filter(|t| !t.is_empty()) {
        let argv = split_exec(term);
        if argv.first().is_some_and(|program| exists(program) || is_executable(Path::new(program)))
        {
            return Some(with_exec_flag(argv));
        }
    }
    if exists("x-terminal-emulator") {
        return Some(vec!["x-terminal-emulator".into(), "-e".into()]);
    }
    TERMINALS.iter().find(|(name, _)| exists(name)).map(|(name, flag)| {
        std::iter::once((*name).to_owned())
            .chain(flag.split_whitespace().map(str::to_owned))
            .collect()
    })
}

/// `$TERMINAL` holds only the program in most setups; `-e` is what nearly every emulator accepts.
fn with_exec_flag(mut argv: Vec<String>) -> Vec<String> {
    if argv.len() == 1 {
        argv.push("-e".into());
    }
    argv
}

fn spawn_detached(argv: &[String]) -> io::Result<()> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Its own process group: closing Daimos must not take the application with it.
    command.process_group(0);
    command.spawn().map(drop)
}

/// The `.desktop` files below `dir` with their desktop file ids (`a/b.desktop` is `a-b.desktop`).
fn desktop_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_owned(), String::new())];
    while let Some((current, prefix)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&current) else { continue };
        let mut children: Vec<_> = read.flatten().collect();
        children.sort_by_key(|c| c.file_name());
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            let path = child.path();
            if path.is_dir() {
                stack.push((path, format!("{prefix}{name}-")));
            } else if name.ends_with(".desktop") {
                out.push((format!("{prefix}{name}"), path));
            }
        }
    }
    out
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Removes the field codes of the Desktop Entry Specification from an `Exec` line and unescapes
/// `%%`. `%i` expands to arguments the launcher does not have, so it goes as well.
pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.trim().chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('%') {
            out.push('%');
        }
    }
    // A removed code can leave two spaces or a trailing one.
    out.split(' ').filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Splits an `Exec` line into arguments: whitespace separates, double quotes group, and a
/// backslash escapes the next character inside quotes.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                    in_arg = true;
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_codes_are_stripped_and_percent_is_unescaped() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(strip_field_codes("app --file=%f %F %U %d %D %n %N %k %v %m"), "app --file=");
        assert_eq!(strip_field_codes("app %i --name %c"), "app --name");
        assert_eq!(strip_field_codes("printf 100%% done"), "printf 100% done");
        assert_eq!(strip_field_codes("  app   %u  "), "app");
        assert_eq!(strip_field_codes("%u"), "");
    }

    #[test]
    fn exec_lines_split_like_the_specification_says() {
        assert_eq!(split_exec("a b  c"), ["a", "b", "c"]);
        assert_eq!(
            split_exec(r#"sh -c "echo \"hi there\"" x"#),
            ["sh", "-c", r#"echo "hi there""#, "x"]
        );
        assert_eq!(split_exec(r#""/opt/My App/run" --flag"#), ["/opt/My App/run", "--flag"]);
        assert_eq!(split_exec(r#"a "" b"#), ["a", "", "b"]);
        assert!(split_exec("   ").is_empty());
    }

    fn fake_path(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = vec![dir.path().to_owned()];
        (dir, path)
    }

    #[test]
    fn the_terminal_is_taken_from_the_environment_first() {
        let (_dir, path) = fake_path(&["alacritty", "xterm", "x-terminal-emulator", "mine"]);
        assert_eq!(terminal_prefix(&path, Some("mine")).unwrap(), ["mine", "-e"]);
        assert_eq!(
            terminal_prefix(&path, Some("mine --title x")).unwrap(),
            ["mine", "--title", "x"]
        );
        // A missing $TERMINAL falls through to the system default.
        assert_eq!(terminal_prefix(&path, Some("gone")).unwrap(), ["x-terminal-emulator", "-e"]);
    }

    #[test]
    fn known_emulators_get_their_own_exec_flag() {
        let (_dir, path) = fake_path(&["konsole", "xterm"]);
        assert_eq!(terminal_prefix(&path, None).unwrap(), ["konsole", "-e"]);
        let (_dir, path) = fake_path(&["gnome-terminal"]);
        assert_eq!(terminal_prefix(&path, None).unwrap(), ["gnome-terminal", "--"]);
        let (_dir, path) = fake_path(&["wezterm"]);
        assert_eq!(terminal_prefix(&path, None).unwrap(), ["wezterm", "start", "--"]);
    }

    #[test]
    fn no_terminal_gives_none() {
        let (_dir, path) = fake_path(&[]);
        assert!(terminal_prefix(&path, None).is_none());
    }
}
