use std::path::{Path, PathBuf};

/// An agent CLI that can be hosted in the right-hand pane.
#[derive(Clone)]
pub struct AgentSpec {
    pub name: String,
    pub command: String,
    pub available: bool,
}

pub fn default_agents() -> Vec<AgentSpec> {
    // (display name, command, executable to probe)
    const DEFAULTS: &[(&str, &str, &str)] = &[
        ("claude", "claude --dangerously-skip-permissions", "claude"),
        ("codex", "codex", "codex"),
        ("opencode", "opencode", "opencode"),
        ("antigravity", "agy", "agy"),
        ("pi", "pi", "pi"),
    ];
    DEFAULTS
        .iter()
        .map(|(name, command, binary)| AgentSpec {
            name: name.to_string(),
            command: command.to_string(),
            available: which(binary).is_some(),
        })
        .collect()
}

pub fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Best-effort check that the command in an agent spec can be launched.
pub fn command_available(command: &str) -> bool {
    let mut words = command.split_whitespace();
    let mut first = words.next().unwrap_or("");
    // Skip leading shell variable assignments (`FOO=1 claude`).
    while first.contains('=') && !first.contains('/') {
        match words.next() {
            Some(word) => first = word,
            None => return false,
        }
    }
    if first.is_empty() {
        return false;
    }
    if first.contains('/') {
        return Path::new(first).is_file();
    }
    which(first).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_defaults_to_skip_permissions() {
        let agents = default_agents();
        let claude = agents
            .iter()
            .find(|agent| agent.name == "claude")
            .expect("claude built-in");
        assert_eq!(claude.command, "claude --dangerously-skip-permissions");
        // Availability must be checked on the executable, not the whole spec.
        assert_eq!(claude.available, which("claude").is_some());
    }

    #[test]
    fn pi_runs_whatever_is_on_path() {
        let agents = default_agents();
        let pi = agents
            .iter()
            .find(|agent| agent.name == "pi")
            .expect("pi built-in");
        assert_eq!(pi.command, "pi");
        assert_eq!(pi.available, which("pi").is_some());
    }

    #[test]
    fn command_available_skips_env_assignments() {
        assert_eq!(
            command_available("FOO=1 sh -c 'true'"),
            command_available("sh")
        );
    }
}
