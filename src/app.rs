use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::{AgentSession, AgentSpec, command_available, default_agents};
use crate::git::{self, GitInfo};
use crate::keys::key_to_bytes;
use crate::tree::FsTree;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Changes,
    Agent,
}

pub struct Message {
    pub text: String,
    pub at: Instant,
    pub error: bool,
}

pub struct Config {
    pub root: PathBuf,
    pub agents: Vec<AgentSpec>,
    pub refresh: Duration,
}

impl Config {
    pub fn from_args(args: &[String]) -> Result<Self> {
        let mut root: Option<PathBuf> = None;
        let mut agents: Vec<AgentSpec> = Vec::new();
        let mut refresh_ms: u64 = 2000;

        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--root" | "-C" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("--root needs a path"))?;
                    root = Some(PathBuf::from(value));
                }
                "--agent" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("--agent needs name=command"))?;
                    agents.push(parse_agent(value));
                }
                "--refresh-ms" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("--refresh-ms needs a number"))?;
                    refresh_ms = value
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid --refresh-ms: {value}"))?;
                }
                other if other.starts_with('-') => {
                    bail!("unknown option: {other} (try --help)");
                }
                path => {
                    if root.is_none() {
                        root = Some(PathBuf::from(path));
                    } else {
                        bail!("unexpected argument: {path}");
                    }
                }
            }
            index += 1;
        }

        let root = root.unwrap_or(std::env::current_dir()?);
        let root = root
            .canonicalize()
            .map_err(|err| anyhow::anyhow!("cannot open {}: {err}", root.display()))?;
        if !root.is_dir() {
            bail!("not a directory: {}", root.display());
        }

        let agents = merge_agents(default_agents(), agents);

        Ok(Self {
            root,
            agents,
            refresh: Duration::from_millis(refresh_ms.max(250)),
        })
    }
}

fn parse_agent(spec: &str) -> AgentSpec {
    let (name, command) = match spec.split_once('=') {
        Some((name, command)) => (name.trim().to_string(), command.trim().to_string()),
        None => (command_name(spec), spec.trim().to_string()),
    };
    AgentSpec {
        available: command_available(&command),
        name,
        command,
    }
}

/// Custom specs add to the built-ins; a spec whose name matches a built-in
/// overrides it (e.g. `--agent claude` drops the default
/// `--dangerously-skip-permissions` flag).
fn merge_agents(mut defaults: Vec<AgentSpec>, specs: Vec<AgentSpec>) -> Vec<AgentSpec> {
    for spec in specs {
        if let Some(existing) = defaults.iter_mut().find(|agent| agent.name == spec.name) {
            *existing = spec;
        } else {
            defaults.push(spec);
        }
    }
    defaults
}

fn command_name(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .map(|first| {
            Path::new(first)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| first.to_string())
        })
        .unwrap_or_else(|| "agent".to_string())
}

pub struct App {
    pub root: PathBuf,
    pub tree: FsTree,
    pub git: GitInfo,
    pub agents: Vec<AgentSpec>,
    pub current: Option<usize>,
    /// One persistent session per configured agent; background agents keep
    /// running and keep their screen state.
    pub sessions: Vec<Option<AgentSession>>,
    /// Background agents that produced output since they were last viewed.
    pub unseen: Vec<bool>,
    pub focus: Focus,
    pub picker: Option<usize>,
    pub message: Option<Message>,
    pub quit: bool,
    pub refresh: Duration,
    pub changes_sel: usize,
    pub tree_offset: usize,
    pub changes_offset: usize,
    pub tree_height: usize,
    pub changes_height: usize,
    pub pty_rows: u16,
    pub pty_cols: u16,
    next_id: u64,
    last_refresh: Instant,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let tree = FsTree::new(config.root.clone(), false);
        let git = git::load(&config.root);
        let agent_count = config.agents.len();

        let mut app = Self {
            root: config.root,
            tree,
            git,
            agents: config.agents,
            current: None,
            sessions: (0..agent_count).map(|_| None).collect(),
            unseen: vec![false; agent_count],
            focus: Focus::Tree,
            picker: None,
            message: None,
            quit: false,
            refresh: config.refresh,
            changes_sel: 0,
            tree_offset: 0,
            changes_offset: 0,
            tree_height: 10,
            changes_height: 10,
            pty_rows: 24,
            pty_cols: 80,
            next_id: 0,
            last_refresh: Instant::now(),
        };

        if app.tree.truncated {
            app.set_message(
                "file tree truncated (too many entries) — narrow the root or add ignores",
                false,
            );
        }

        // Never pick an agent for the user: the choice happens inside the app.
        // Open the switcher right away so one Enter starts the default agent,
        // or Esc to just browse files first.
        if !app.agents.iter().any(|agent| agent.available) {
            app.set_message(
                "no agent CLI found — install claude/codex/opencode/agy or define one with --agent",
                true,
            );
        }
        app.open_picker();

        Ok(app)
    }

    pub fn set_message(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some(Message {
            text: text.into(),
            at: Instant::now(),
            error,
        });
    }

    pub fn current_agent_name(&self) -> String {
        self.current
            .and_then(|index| self.agents.get(index))
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| "none".to_string())
    }

    pub fn active_session(&self) -> Option<&AgentSession> {
        self.current
            .and_then(|index| self.sessions.get(index))
            .and_then(|session| session.as_ref())
    }

    fn active_session_mut(&mut self) -> Option<&mut AgentSession> {
        let index = self.current?;
        self.sessions.get_mut(index)?.as_mut()
    }

    pub fn running_count(&self) -> usize {
        self.sessions
            .iter()
            .flatten()
            .filter(|session| session.exited.is_none())
            .count()
    }

    /// Start `index` if it isn't running, otherwise focus the existing
    /// session. Other agents keep running in the background.
    fn start_agent(&mut self, index: usize) {
        if index >= self.agents.len() {
            return;
        }

        if matches!(
            self.sessions.get(index),
            Some(Some(session)) if session.exited.is_none()
        ) {
            self.activate(index);
            return;
        }

        if let Some(mut stale) = self.sessions.get_mut(index).and_then(|slot| slot.take()) {
            stale.kill();
        }

        let spec = self.agents[index].clone();
        self.next_id += 1;
        let rows = self.pty_rows.max(5);
        let cols = self.pty_cols.max(20);

        match AgentSession::spawn(self.next_id, &spec, &self.root, rows, cols) {
            Ok(session) => {
                self.sessions[index] = Some(session);
                self.activate(index);
            }
            Err(err) => {
                self.current = Some(index);
                self.set_message(format!("failed to start {}: {err:#}", spec.name), true);
            }
        }
    }

    fn activate(&mut self, index: usize) {
        self.current = Some(index);
        if let Some(unseen) = self.unseen.get_mut(index) {
            *unseen = false;
        }
    }

    fn stop_agent(&mut self, index: usize) {
        if let Some(mut session) = self.sessions.get_mut(index).and_then(|slot| slot.take()) {
            session.kill();
        }
        if let Some(unseen) = self.unseen.get_mut(index) {
            *unseen = false;
        }
        if self.current == Some(index) {
            // Stay on a live session if there is one.
            let next = self.sessions.iter().position(|slot| slot.is_some());
            self.current = next;
            if let Some(next) = next
                && let Some(unseen) = self.unseen.get_mut(next)
            {
                *unseen = false;
            }
        }
    }

    fn cycle_agent(&mut self, delta: isize) {
        let indices: Vec<usize> = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.is_some())
            .map(|(index, _)| index)
            .collect();
        if indices.is_empty() {
            self.set_message("no agent session — press F4 to start one", true);
            return;
        }
        let next = match self
            .current
            .and_then(|current| indices.iter().position(|&index| index == current))
        {
            Some(position) => {
                (position as isize + delta).rem_euclid(indices.len() as isize) as usize
            }
            None => {
                if delta >= 0 {
                    0
                } else {
                    indices.len() - 1
                }
            }
        };
        self.activate(indices[next]);
        self.focus = Focus::Agent;
    }

    pub fn shutdown(&mut self) {
        for slot in &mut self.sessions {
            if let Some(mut session) = slot.take() {
                session.kill();
            }
        }
    }

    pub fn pump(&mut self) {
        let current = self.current;
        let mut exit_notes: Vec<String> = Vec::new();

        for (index, slot) in self.sessions.iter_mut().enumerate() {
            let Some(session) = slot else {
                continue;
            };
            let id = session.id;
            let mut produced = false;
            while let Ok((chunk_id, bytes)) = session.rx.try_recv() {
                if chunk_id == id {
                    session.process(&bytes);
                    produced = true;
                }
            }
            if produced
                && current != Some(index)
                && let Some(unseen) = self.unseen.get_mut(index)
            {
                *unseen = true;
            }
            if let Some(note) = session.check_exit() {
                exit_notes.push(format!("{} {note}", session.name));
            }
        }

        if !exit_notes.is_empty() {
            self.set_message(
                format!("{} — reselect it to restart", exit_notes.join(", ")),
                false,
            );
        }
    }

    pub fn tick(&mut self) {
        if self.last_refresh.elapsed() >= self.refresh {
            self.last_refresh = Instant::now();
            self.refresh_all();
        }
        if let Some(message) = &self.message
            && message.at.elapsed() > Duration::from_secs(5)
        {
            self.message = None;
        }
    }

    pub fn refresh_all(&mut self) {
        let tree_selection = self.tree.selected_path().map(Path::to_path_buf);
        let tree_offset = self.tree_offset;
        self.tree.rebuild();
        if let Some(path) = tree_selection {
            self.tree.select_path(&path);
        }
        self.tree_offset = tree_offset;
        self.tree.clamp();

        let change_selection = self
            .git
            .changes
            .get(self.changes_sel)
            .map(|change| change.path.clone());
        self.git = git::load(&self.root);
        if let Some(path) = change_selection
            && let Some(index) = self
                .git
                .changes
                .iter()
                .position(|change| change.path == path)
        {
            self.changes_sel = index;
        }
        self.clamp_changes();
    }

    fn clamp_changes(&mut self) {
        if self.git.changes.is_empty() {
            self.changes_sel = 0;
            self.changes_offset = 0;
        } else if self.changes_sel >= self.git.changes.len() {
            self.changes_sel = self.git.changes.len() - 1;
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Global escapes.
        if ctrl && matches!(key.code, KeyCode::Char('q')) {
            self.quit = true;
            return;
        }
        if ctrl
            && matches!(key.code, KeyCode::Char('c'))
            && self.focus != Focus::Agent
        {
            self.quit = true;
            return;
        }

        if self.picker.is_some() {
            self.picker_key(key);
            return;
        }

        match key.code {
            KeyCode::F(1) => {
                self.focus = Focus::Tree;
                return;
            }
            KeyCode::F(2) => {
                self.focus = Focus::Changes;
                return;
            }
            KeyCode::F(3) => {
                self.focus = Focus::Agent;
                return;
            }
            KeyCode::F(4) => {
                self.open_picker();
                return;
            }
            KeyCode::F(5) => {
                self.refresh_all();
                self.set_message("refreshed", false);
                return;
            }
            KeyCode::F(6) => {
                self.cycle_agent(1);
                return;
            }
            KeyCode::F(7) => {
                self.cycle_agent(-1);
                return;
            }
            _ => {}
        }

        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Changes => self.changes_key(key),
            Focus::Agent => self.agent_key(key),
        }
    }

    fn open_picker(&mut self) {
        let start = self.current.unwrap_or_else(|| {
            self.agents
                .iter()
                .position(|agent| agent.available)
                .unwrap_or(0)
        });
        self.picker = Some(start);
    }

    fn picker_key(&mut self, key: KeyEvent) {
        let Some(selected) = self.picker else {
            return;
        };
        let count = self.agents.len();
        if count == 0 {
            self.picker = None;
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.picker = None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.picker = Some(selected.saturating_sub(1));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.picker = Some((selected + 1).min(count - 1));
            }
            KeyCode::Enter => {
                self.picker = None;
                self.activate_agent(selected);
            }
            KeyCode::Char(digit @ '1'..='9') => {
                let index = digit as usize - '1' as usize;
                if index < count {
                    self.picker = None;
                    self.activate_agent(index);
                }
            }
            KeyCode::Char('d') => {
                let has_session = self
                    .sessions
                    .get(selected)
                    .map(|slot| slot.is_some())
                    .unwrap_or(false);
                if has_session {
                    let name = self
                        .agents
                        .get(selected)
                        .map(|agent| agent.name.clone())
                        .unwrap_or_default();
                    self.stop_agent(selected);
                    self.set_message(format!("stopped {name}"), false);
                }
            }
            _ => {}
        }
    }

    fn activate_agent(&mut self, index: usize) {
        if index >= self.agents.len() {
            return;
        }
        // An existing session (even an exited one) is shown as-is; use Enter
        // in the agent pane to restart it. Only start new processes on demand.
        if self
            .sessions
            .get(index)
            .map(|slot| slot.is_some())
            .unwrap_or(false)
        {
            self.activate(index);
        } else {
            self.start_agent(index);
        }
        self.focus = Focus::Agent;
    }

    fn tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.tree.down(),
            KeyCode::Up | KeyCode::Char('k') => self.tree.up(),
            KeyCode::Home | KeyCode::Char('g') => self.tree.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.tree.selected = self.tree.rows.len().saturating_sub(1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.tree.toggle(),
            KeyCode::Right | KeyCode::Char('l') => self.tree.expand(),
            KeyCode::Left | KeyCode::Char('h') => self.tree.left(),
            KeyCode::Char('.') => {
                let hidden = !self.tree.show_hidden;
                self.tree.set_hidden(hidden);
            }
            KeyCode::Char('a') => self.add_selection_to_agent(),
            KeyCode::Char('r') => {
                self.refresh_all();
                self.set_message("refreshed", false);
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    /// Insert `@relative/path` for the selected tree entry into the running
    /// agent's input, so it becomes part of the current session context.
    fn add_selection_to_agent(&mut self) {
        let Some(path) = self.tree.selected_path().map(Path::to_path_buf) else {
            return;
        };
        let state = self
            .active_session()
            .map(|session| (session.name.clone(), session.exited.is_some()));
        let (name, exited) = match state {
            Some(state) => state,
            None => {
                self.set_message("no agent running — press F4 to choose one", true);
                return;
            }
        };
        if exited {
            self.set_message(
                "agent has exited — press Enter in the agent pane to restart",
                true,
            );
            return;
        }

        let relative = path.strip_prefix(&self.root).unwrap_or(&path);
        let mut reference = relative.to_string_lossy().to_string();
        if reference.is_empty() {
            reference = ".".to_string();
        }
        if path.is_dir() && !reference.ends_with('/') {
            reference.push('/');
        }
        let mention = if reference.contains(char::is_whitespace) {
            format!("@\"{reference}\" ")
        } else {
            format!("@{reference} ")
        };

        if let Some(session) = self.active_session_mut() {
            session.write_input(mention.as_bytes());
        }
        self.set_message(format!("added {mention}to {name}"), false);
    }

    fn changes_key(&mut self, key: KeyEvent) {
        let count = self.git.changes.len();
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.changes_sel = (self.changes_sel + 1).min(count - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.changes_sel = self.changes_sel.saturating_sub(1);
            }
            KeyCode::Home | KeyCode::Char('g') => self.changes_sel = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.changes_sel = count.saturating_sub(1);
            }
            KeyCode::Enter => {
                let path = self
                    .git
                    .changes
                    .get(self.changes_sel)
                    .map(|change| self.root.join(&change.path));
                if let Some(path) = path
                    && self.tree.reveal(&path)
                {
                    self.focus = Focus::Tree;
                }
            }
            KeyCode::Char('r') => {
                self.refresh_all();
                self.set_message("refreshed", false);
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn agent_key(&mut self, key: KeyEvent) {
        let exited = self
            .active_session()
            .map(|session| session.exited.is_some())
            .unwrap_or(true);

        if exited {
            if matches!(key.code, KeyCode::Enter)
                && let Some(index) = self.current
            {
                self.start_agent(index);
            }
            return;
        }

        let bytes = key_to_bytes(key);
        if let Some(session) = self.active_session_mut() {
            session.write_input(&bytes);
        }
    }

    pub fn on_paste(&mut self, text: String) {
        if self.focus != Focus::Agent || text.is_empty() {
            return;
        }
        let Some(session) = self.active_session_mut() else {
            return;
        };
        if session.exited.is_some() {
            return;
        }
        let mut payload = Vec::with_capacity(text.len() + 12);
        payload.extend_from_slice(b"\x1b[200~");
        payload.extend_from_slice(text.as_bytes());
        payload.extend_from_slice(b"\x1b[201~");
        session.write_input(&payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, command: &str) -> AgentSpec {
        AgentSpec {
            name: name.to_string(),
            command: command.to_string(),
            available: true,
        }
    }

    #[test]
    fn custom_specs_override_builtins_by_name() {
        let merged = merge_agents(
            vec![spec("claude", "claude --dangerously-skip-permissions"), spec("codex", "codex")],
            vec![spec("claude", "claude"), spec("extra", "extra --flag")],
        );
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].command, "claude");
        assert_eq!(merged[1].command, "codex");
        assert_eq!(merged[2].name, "extra");
    }
}
