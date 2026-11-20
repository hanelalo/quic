use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::spec::AgentSpec;

/// A child process (claude/codex/opencode) attached to a PTY, plus a terminal
/// emulator that turns its output into a screen we can draw with ratatui.
pub struct AgentSession {
    pub id: u64,
    pub name: String,
    pub term: vt100::Parser,
    pub rx: Receiver<(u64, Vec<u8>)>,
    pub rows: u16,
    pub cols: u16,
    pub exited: Option<String>,
    /// Whether the child has painted anything visible yet (used for a
    /// "starting" hint). Many TUIs emit terminal queries long before drawing.
    pub has_drawn: bool,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl AgentSession {
    pub fn spawn(
        id: u64,
        spec: &AgentSpec,
        root: &Path,
        rows: u16,
        cols: u16,
    ) -> Result<Self> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // Run through a shell so agent specs can carry arguments, and `exec`
        // so the PTY child is the agent itself (kill() reaches it directly).
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(format!("exec {}", spec.command));
        cmd.cwd(root);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "quic");
        // Don't let an outer agent session leak into the hosted one.
        cmd.env_remove("CLAUDECODE");
        cmd.env_remove("CLAUDE_CODE_CHILD_SESSION");

        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawning `{}`", spec.command))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("cloning pty reader")?;
        let writer = pair.master.take_writer().context("taking pty writer")?;

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send((id, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            id,
            name: spec.name.clone(),
            term: vt100::Parser::new(nonzero(rows), nonzero(cols), 0),
            rx,
            rows,
            cols,
            exited: None,
            has_drawn: false,
            writer,
            master: pair.master,
            child,
        })
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 || (rows == self.rows && cols == self.cols) {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.term.set_size(nonzero(rows), nonzero(cols));
        self.rows = rows;
        self.cols = cols;
    }

    pub fn write_input(&mut self, bytes: &[u8]) {
        if self.exited.is_some() || bytes.is_empty() {
            return;
        }
        self.write_raw(bytes);
    }

    fn screen_has_content(&self) -> bool {
        let screen = self.term.screen();
        for row in 0..self.rows {
            for col in 0..self.cols {
                if let Some(cell) = screen.cell(row, col)
                    && cell.has_contents()
                    && cell.contents().chars().any(|c| !c.is_whitespace())
                {
                    return true;
                }
            }
        }
        false
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        if self.writer.write_all(bytes).is_ok() {
            let _ = self.writer.flush();
        }
    }

    /// Feed a chunk of agent output into the emulator and answer the few
    /// terminal queries that would otherwise make a full-screen app hang.
    pub fn process(&mut self, bytes: &[u8]) {
        self.term.process(bytes);
        if !self.has_drawn && self.screen_has_content() {
            self.has_drawn = true;
        }

        let mut responses: Vec<u8> = Vec::new();

        // DSR (cursor position request).
        if contains(bytes, b"\x1b[6n") {
            let (row, col) = self.term.screen().cursor_position();
            responses.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
        }
        // Primary device attributes.
        if contains(bytes, b"\x1b[c") || contains(bytes, b"\x1b[0c") {
            responses.extend_from_slice(b"\x1b[?1;2c");
        }
        // Secondary device attributes (claim to be an xterm).
        if contains(bytes, b"\x1b[>c") || contains(bytes, b"\x1b[>0c") {
            responses.extend_from_slice(b"\x1b[>0;276;0c");
        }

        if !responses.is_empty() {
            self.write_raw(&responses);
        }
    }

    /// Returns a message the first time the child has exited.
    pub fn check_exit(&mut self) -> Option<String> {
        if self.exited.is_some() {
            return None;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let msg = format!("exited (code {})", status.exit_code());
                self.exited = Some(msg.clone());
                Some(msg)
            }
            Ok(None) => None,
            Err(err) => {
                let msg = format!("exited ({err})");
                self.exited = Some(msg.clone());
                Some(msg)
            }
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.exited.is_none() {
            self.exited = Some("stopped".to_string());
        }
    }
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap_or(NonZeroU16::MIN)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
