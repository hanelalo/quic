mod agent;
mod app;
mod git;
mod keys;
mod tree;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
};

use app::{App, Config};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }

    let config = Config::from_args(&args)?;

    let mut terminal = ratatui::init();
    ratatui::crossterm::execute!(io::stdout(), EnableBracketedPaste)?;

    let mut app = match App::new(config) {
        Ok(app) => app,
        Err(err) => {
            let _ = ratatui::crossterm::execute!(io::stdout(), DisableBracketedPaste);
            ratatui::restore();
            return Err(err);
        }
    };

    let result = run(&mut terminal, &mut app);

    let _ = ratatui::crossterm::execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    app.shutdown();

    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.pump();
        terminal.draw(|frame| ui::draw(frame, app))?;

        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.on_key(key),
                Event::Paste(text) => app.on_paste(text),
                _ => {}
            }
        }

        app.tick();
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"quic — a terminal workspace for coding agents

USAGE:
    quic [ROOT] [OPTIONS]

ARGS:
    ROOT                 Project directory to open (default: current directory)

OPTIONS:
    -C, --root <PATH>    Project directory to open
        --agent <SPEC>   Define an extra agent for the in-app switcher.
                         SPEC is "name=command" or just "command"; repeatable.
                         This never selects an agent at startup.
                         Built-ins: claude (--dangerously-skip-permissions),
                         codex, opencode, antigravity (agy), pi.
                         A spec with the same name overrides that built-in.
        --refresh-ms <N> Auto-refresh interval for tree + git status (default: 2000)
    -h, --help           Show this help

EXAMPLES:
    quic
    quic ~/src/my-project
    quic --agent "gpt=codex --model gpt-5.5"   # adds a switcher entry only
    quic --agent claude                        # claude with normal permissions

KEYS:
    F1 / F2 / F3   Focus file tree / change list / agent
    F4             Agent switcher (start / switch; d stops a session)
    F5             Refresh tree and git status
    F6 / F7        Previous / next agent session
    ^Q             Quit (anywhere)
    ^C             Quit, unless the agent pane has focus

    Started agents keep running when you switch away. Start another one from
    F4 and it runs alongside the others.
    FILES:   j/k move, Enter toggle dir, h/l collapse/expand, . hidden, r refresh,
             a insert @mention of the selection into the agent's input
    CHANGES: j/k move, g/G top/bottom, r refresh
    AGENT:   every other key goes to the agent; Enter restarts it if it exited
"#
    );
}
