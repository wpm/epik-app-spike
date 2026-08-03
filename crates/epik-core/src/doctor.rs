//! Locating and version-checking the `claude` engine.
//!
//! The app treats Claude Code the way uv treats Python: an engine it finds (and
//! could fetch) rather than a dependency the user must pre-assemble. The doctor
//! reports what it found and what it would do about it.
//!
//! Resolution must not depend on `PATH`, because the app's most common launch
//! condition is the one where `PATH` is useless. A process started from Finder,
//! the Dock, or Spotlight on macOS inherits `launchd`'s environment — roughly
//! `/usr/bin:/bin:/usr/sbin:/sbin` — not the shell's, so a `claude` installed in
//! `~/.local/bin` is invisible to a `PATH` search even though it works fine in a
//! terminal. `PATH` is therefore one source among several rather than *the*
//! source, and [`EngineSearch`] makes the whole search explicit so the
//! empty-`PATH` behaviour is a property of this module that can be tested,
//! rather than a property of the machine the tests happen to run on.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(feature = "host")]
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Oldest CLI this app's protocol layer is known to work against (the version
/// the spike was verified on).
pub const MIN_SUPPORTED: (u32, u32) = (2, 1);

/// Where `claude` turns up in practice, beyond `PATH`: the official installer
/// uses `~/.local/bin`; Homebrew links into `/opt/homebrew/bin` (Apple Silicon)
/// or `/usr/local/bin` (Intel); `~/.claude/local` is the self-managed install
/// used by `claude install` / `migrate-installer`.
pub fn known_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs
}

/// The engine search, with every input explicit.
#[derive(Debug, Clone, Default)]
pub struct EngineSearch {
    /// The `PATH` to search, if there is one.
    pub path_env: Option<OsString>,
    /// Directories searched regardless of `PATH`, in order.
    pub known_dirs: Vec<PathBuf>,
    /// Binary name to look for.
    pub binary: String,
}

impl EngineSearch {
    pub fn from_env() -> Self {
        Self {
            path_env: std::env::var_os("PATH"),
            known_dirs: known_dirs(),
            binary: "claude".to_owned(),
        }
    }

    /// First match on `PATH`, then the first known directory that has it.
    /// `PATH` goes first so a user who deliberately puts a particular `claude`
    /// ahead of the installed one still gets theirs.
    pub fn resolve(&self) -> Option<PathBuf> {
        let named = |dir: &Path| {
            let candidate = dir.join(&self.binary);
            candidate.is_file().then_some(candidate)
        };
        if let Some(paths) = &self.path_env {
            for dir in std::env::split_paths(paths) {
                if let Some(found) = named(&dir) {
                    return Some(found);
                }
            }
        }
        self.known_dirs.iter().find_map(|dir| named(dir))
    }

    /// Every directory this search would look in, in order.
    pub fn searched_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self
            .path_env
            .as_ref()
            .map(|p| std::env::split_paths(p).collect())
            .unwrap_or_default();
        dirs.extend(self.known_dirs.iter().cloned());
        dirs
    }
}

/// What the doctor screen has to distinguish. A UI needs to say something
/// different in each of these cases, so they are a type rather than a bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatus {
    /// Found, ran, and new enough.
    Ok,
    /// Found and ran, but older than [`MIN_SUPPORTED`].
    TooOld,
    /// Found, but `--version` failed or its output was unintelligible.
    Unusable,
    /// Not on `PATH` and not in any known install location.
    NotFound,
}

impl EngineStatus {
    /// Whether a session can be started. The window shows chat when this is
    /// true and the doctor report when it is false.
    pub fn is_usable(self) -> bool {
        self == Self::Ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineReport {
    /// Resolved binary, if any.
    pub path: Option<PathBuf>,
    /// Raw `--version` output, if the binary ran.
    pub version_raw: Option<String>,
    /// Parsed (major, minor).
    pub version: Option<(u32, u32)>,
    pub status: EngineStatus,
    /// Oldest version this build supports, so a frontend can name it without
    /// hard-coding a number the core would later change.
    pub min_supported: (u32, u32),
    /// Directories that were searched, so a "not found" report can explain
    /// itself rather than just asserting.
    pub searched: Vec<PathBuf>,
}

impl EngineReport {
    pub fn supported(&self) -> bool {
        self.status.is_usable()
    }

    /// One-line summary.
    pub fn headline(&self) -> String {
        let (major, minor) = self.min_supported;
        match self.status {
            EngineStatus::Ok => "Claude Code is installed and supported.".to_owned(),
            EngineStatus::TooOld => format!(
                "Claude Code {} is older than the minimum supported {major}.{minor}.",
                self.version_raw.as_deref().unwrap_or("(unknown)")
            ),
            EngineStatus::Unusable => "Claude Code was found but would not run.".to_owned(),
            EngineStatus::NotFound => "No `claude` binary found.".to_owned(),
        }
    }

    /// What the user should do about it, or `None` when nothing is wrong.
    pub fn remedy(&self) -> Option<String> {
        match self.status {
            EngineStatus::Ok => None,
            EngineStatus::TooOld => Some(
                "Run `claude update`, or reinstall from https://claude.com/claude-code.".to_owned(),
            ),
            EngineStatus::Unusable => Some(
                "The install looks broken. Reinstall from https://claude.com/claude-code."
                    .to_owned(),
            ),
            EngineStatus::NotFound => Some(
                "Install Claude Code from https://claude.com/claude-code. \
                 (A future epik-app could fetch and manage its own engine here, \
                 the way uv manages Python.)"
                    .to_owned(),
            ),
        }
    }

    /// The full report as text — the same content the former `epik-app doctor`
    /// subcommand printed. The window renders this when the engine is not
    /// usable, so there is one description of an engine problem rather than a
    /// CLI one and a GUI one that drift.
    pub fn report_text(&self) -> String {
        let mut out = String::new();
        match &self.path {
            Some(path) => out.push_str(&format!("engine:  {}\n", path.display())),
            None => out.push_str("engine:  not found\n"),
        }
        out.push_str(&format!(
            "version: {}\n",
            self.version_raw.as_deref().unwrap_or("unknown")
        ));
        let (major, minor) = self.min_supported;
        out.push_str(&format!("minimum: {major}.{minor}\n"));
        out.push_str(&format!("status:  {}\n", self.headline()));
        if self.path.is_none() && !self.searched.is_empty() {
            out.push_str("\nsearched:\n");
            for dir in &self.searched {
                out.push_str(&format!("  {}\n", dir.display()));
            }
        }
        if let Some(remedy) = self.remedy() {
            out.push_str(&format!("\n{remedy}\n"));
        }
        out
    }
}

/// Inspect the engine using the process environment.
#[cfg(feature = "host")]
pub fn inspect() -> EngineReport {
    inspect_with(&EngineSearch::from_env())
}

/// Inspect the engine using an explicit search. `search` decides where to look;
/// this decides what the result means.
#[cfg(feature = "host")]
pub fn inspect_with(search: &EngineSearch) -> EngineReport {
    let path = search.resolve();
    let version_raw = path.as_ref().and_then(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    });
    let version = version_raw.as_deref().and_then(parse_version);
    let status = match (&path, version) {
        (None, _) => EngineStatus::NotFound,
        (Some(_), None) => EngineStatus::Unusable,
        (Some(_), Some(v)) if v >= MIN_SUPPORTED => EngineStatus::Ok,
        (Some(_), Some(_)) => EngineStatus::TooOld,
    };
    EngineReport {
        path,
        version_raw,
        version,
        status,
        min_supported: MIN_SUPPORTED,
        searched: search.searched_dirs(),
    }
}

/// Parse "2.1.220 (Claude Code)" → (2, 1).
pub fn parse_version(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.split_whitespace().next()?.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Human-readable doctor output; exit-worthy problems return Err. Kept for the
/// examples and for anything that wants the report on a terminal.
#[cfg(feature = "host")]
pub fn run_doctor() -> anyhow::Result<()> {
    let report = inspect();
    print!("{}", report.report_text());
    if report.supported() {
        Ok(())
    } else {
        anyhow::bail!("{}", report.headline())
    }
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;

    #[test]
    fn parses_cli_version_output() {
        assert_eq!(parse_version("2.1.220 (Claude Code)"), Some((2, 1)));
        assert_eq!(parse_version("3.0.1"), Some((3, 0)));
        assert_eq!(parse_version("nonsense"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn version_ordering_matches_support_check() {
        assert!((2, 1) >= MIN_SUPPORTED);
        assert!((2, 0) < MIN_SUPPORTED);
        assert!((3, 0) >= MIN_SUPPORTED);
    }

    /// A directory holding a fake `claude`, removed on drop.
    #[cfg(unix)]
    struct FakeInstall {
        dir: PathBuf,
    }

    #[cfg(unix)]
    impl FakeInstall {
        fn new(name: &str, script: &str) -> Self {
            use std::io::Write;
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("epik-doctor-{name}"));
            std::fs::create_dir_all(&dir).expect("create fake install dir");
            let bin = dir.join("claude");
            let mut file = std::fs::File::create(&bin).expect("create fake claude");
            write!(file, "#!/bin/sh\n{script}").expect("write fake claude");
            file.flush().expect("flush fake claude");
            drop(file);
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake claude");

            // Absorb the ETXTBSY race here, so `inspect_with` cannot lose to
            // it later and misreport a fine engine as Unusable. Tests run
            // concurrently in one process; if another test forks while this
            // script is open for writing above, the forked child holds the
            // write fd until its own exec completes, and exec of a file
            // anyone holds open for writing fails with `Text file busy`. One
            // successful run proves the writers are gone — after that the
            // file has none and can never ETXTBSY again.
            let mut delay = std::time::Duration::from_millis(10);
            loop {
                match Command::new(&bin).arg("--version").output() {
                    Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        assert!(delay < std::time::Duration::from_secs(20), "{err}");
                        std::thread::sleep(delay);
                        delay *= 2;
                    }
                    _ => break,
                }
            }

            Self { dir }
        }

        /// A search with *no* `PATH` at all — the Finder-launch condition, only
        /// more extreme than reality.
        fn search_without_path(&self) -> EngineSearch {
            EngineSearch {
                path_env: None,
                known_dirs: vec![self.dir.clone()],
                binary: "claude".to_owned(),
            }
        }
    }

    #[cfg(unix)]
    impl Drop for FakeInstall {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolves_with_no_path_at_all() {
        let install = FakeInstall::new("no-path", "echo '2.1.220 (Claude Code)'\n");
        let search = install.search_without_path();
        assert_eq!(
            search.path_env, None,
            "the point of this test is that there is no PATH"
        );

        let report = inspect_with(&search);
        assert_eq!(report.path, Some(install.dir.join("claude")));
        assert_eq!(report.version, Some((2, 1)));
        assert_eq!(report.status, EngineStatus::Ok);
        assert!(report.supported());
    }

    #[cfg(unix)]
    #[test]
    fn resolves_with_the_path_a_gui_launch_actually_gets() {
        // launchd's PATH, verbatim. `claude` is in none of these.
        let install = FakeInstall::new("finder-path", "echo '2.1.220 (Claude Code)'\n");
        let search = EngineSearch {
            path_env: Some(OsString::from("/usr/bin:/bin:/usr/sbin:/sbin")),
            ..install.search_without_path()
        };
        let report = inspect_with(&search);
        assert_eq!(
            report.path,
            Some(install.dir.join("claude")),
            "a GUI-launched app must still find an engine PATH cannot see"
        );
        assert!(report.supported());
    }

    #[cfg(unix)]
    #[test]
    fn path_wins_over_known_locations() {
        let on_path = FakeInstall::new("on-path", "echo '9.9.9 (Claude Code)'\n");
        let known = FakeInstall::new("known", "echo '2.1.220 (Claude Code)'\n");
        let search = EngineSearch {
            path_env: Some(on_path.dir.clone().into_os_string()),
            known_dirs: vec![known.dir.clone()],
            binary: "claude".to_owned(),
        };
        assert_eq!(search.resolve(), Some(on_path.dir.join("claude")));
    }

    #[test]
    fn not_found_is_reported_with_the_directories_searched() {
        let search = EngineSearch {
            path_env: Some(OsString::from("/nonexistent-a:/nonexistent-b")),
            known_dirs: vec![PathBuf::from("/nonexistent-c")],
            binary: "claude".to_owned(),
        };
        let report = inspect_with(&search);
        assert_eq!(report.status, EngineStatus::NotFound);
        assert!(!report.supported());
        assert_eq!(
            report.searched,
            vec![
                PathBuf::from("/nonexistent-a"),
                PathBuf::from("/nonexistent-b"),
                PathBuf::from("/nonexistent-c"),
            ]
        );
        // The report has to explain itself to someone who does not know where
        // the app looks.
        let text = report.report_text();
        assert!(text.contains("not found"), "{text}");
        assert!(text.contains("/nonexistent-c"), "{text}");
        assert!(text.contains("claude.com/claude-code"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn an_engine_below_the_floor_is_too_old_not_missing() {
        let install = FakeInstall::new("too-old", "echo '1.9.0 (Claude Code)'\n");
        let report = inspect_with(&install.search_without_path());
        assert_eq!(report.status, EngineStatus::TooOld);
        assert!(!report.supported());
        assert!(report.report_text().contains("older than"));
    }

    #[cfg(unix)]
    #[test]
    fn an_engine_that_will_not_run_is_unusable_not_missing() {
        let install = FakeInstall::new("broken", "exit 1\n");
        let report = inspect_with(&install.search_without_path());
        assert_eq!(report.status, EngineStatus::Unusable);
        assert!(report.path.is_some(), "it was found, just not usable");
        assert!(report.report_text().contains("would not run"));
    }

    #[test]
    fn report_round_trips_through_json() {
        // The doctor screen is rendered by the frontend, so the report crosses
        // IPC like every other value the host produces.
        let report = inspect_with(&EngineSearch::default());
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(serde_json::from_str::<EngineReport>(&json).unwrap(), report);
    }
}
