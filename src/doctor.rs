//! M4 — locating and version-checking the `claude` engine.
//!
//! The app treats Claude Code the way uv treats Python: an engine it finds
//! (and could fetch) rather than a dependency the user must pre-assemble.
//! `epik-app doctor` reports what it found and what it would do about it.

use std::path::PathBuf;
use std::process::Command;

/// Oldest CLI this app's protocol layer is known to work against (the version
/// the spike was verified on).
pub const MIN_SUPPORTED: (u32, u32) = (2, 1);

/// Where `claude` turns up in practice, beyond $PATH: the official installer
/// uses `~/.local/bin`; Homebrew links into `/opt/homebrew/bin` (Apple
/// Silicon) or `/usr/local/bin` (Intel); `~/.claude/local` is the
/// self-managed install used by `claude install`/migrate-installer.
fn known_locations() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".local/bin/claude"));
        candidates.push(home.join(".claude/local/claude"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/claude"));
    candidates.push(PathBuf::from("/usr/local/bin/claude"));
    candidates
}

#[derive(Debug)]
pub struct EngineReport {
    /// Resolved binary, if any: first `claude` on PATH, else first known
    /// location that exists.
    pub path: Option<PathBuf>,
    /// Raw `--version` output, if the binary ran.
    pub version_raw: Option<String>,
    /// Parsed (major, minor).
    pub version: Option<(u32, u32)>,
    pub supported: bool,
}

pub fn inspect() -> EngineReport {
    let path = which_claude();
    let version_raw = path.as_ref().and_then(|p| {
        Command::new(p)
            .arg("--version")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    });
    let version = version_raw.as_deref().and_then(parse_version);
    let supported = version.is_some_and(|v| v >= MIN_SUPPORTED);
    EngineReport {
        path,
        version_raw,
        version,
        supported,
    }
}

fn which_claude() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("claude");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    known_locations().into_iter().find(|p| p.is_file())
}

/// Parse "2.1.220 (Claude Code)" → (2, 1).
pub fn parse_version(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.split_whitespace().next()?.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Human-readable doctor output; exit-worthy problems return Err.
pub fn run_doctor() -> anyhow::Result<()> {
    let report = inspect();
    match (&report.path, &report.version_raw) {
        (Some(path), Some(raw)) => {
            println!("engine: {}", path.display());
            println!("version: {raw}");
            if report.supported {
                println!("status: OK (>= {}.{})", MIN_SUPPORTED.0, MIN_SUPPORTED.1);
                Ok(())
            } else {
                anyhow::bail!(
                    "claude {raw} is older than the minimum supported {}.{} — \
                     run `claude update` or reinstall: https://claude.com/claude-code",
                    MIN_SUPPORTED.0,
                    MIN_SUPPORTED.1
                );
            }
        }
        (Some(path), None) => anyhow::bail!(
            "found {} but `--version` failed — the install looks broken; \
             reinstall: https://claude.com/claude-code",
            path.display()
        ),
        (None, _) => anyhow::bail!(
            "no `claude` binary found on PATH or in known install locations \
             (~/.local/bin, ~/.claude/local, /opt/homebrew/bin, /usr/local/bin).\n\
             Install Claude Code first: https://claude.com/claude-code\n\
             (A future epik-app could fetch and manage its own engine here, \
             the way uv manages Python.)"
        ),
    }
}

#[cfg(test)]
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
}
