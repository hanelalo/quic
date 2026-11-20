//! Headless smoke test: runs the TUI inside a PTY, sends keys, and dumps the
//! resulting screen as plain text.
//!
//! cargo run --example smoke -- [BIN] [ROOT] [AGENT_SPEC] [--type] [--quick]

use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const ROWS: u16 = 40;
const COLS: u16 = 140;

fn main() -> Result<()> {
    let mut positional: Vec<String> = Vec::new();
    let mut do_type = false;
    let mut quick = false;
    let mut do_switch = false;
    let mut do_reveal = false;
    let mut do_add = false;
    let mut do_multi = false;
    let mut do_stop = false;
    let mut pick: Option<u8> = None;
    let mut settle_ms: Option<u64> = None;

    for arg in std::env::args().skip(1) {
        if let Some(value) = arg.strip_prefix("--pick=") {
            pick = value.parse().ok();
            continue;
        }
        if let Some(value) = arg.strip_prefix("--settle=") {
            settle_ms = value.parse().ok();
            continue;
        }
        match arg.as_str() {
            "--type" => do_type = true,
            "--quick" => quick = true,
            "--switch" => do_switch = true,
            "--reveal" => do_reveal = true,
            "--add" => do_add = true,
            "--multi" => do_multi = true,
            "--stop" => do_stop = true,
            _ => positional.push(arg),
        }
    }

    let mut positional = positional.into_iter();
    let bin = std::fs::canonicalize(
        positional
            .next()
            .unwrap_or_else(|| "target/debug/quic".to_string()),
    )?;
    let root = std::fs::canonicalize(
        positional
            .next()
            .unwrap_or_else(|| ".".to_string()),
    )?;
    let agents: Vec<String> = positional.collect();

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(&bin);
    cmd.arg("--root");
    cmd.arg(&root);
    for agent in &agents {
        cmd.arg("--agent");
        cmd.arg(agent);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.cwd(&root);

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 16384];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let mut parser = vt100::Parser::new(
        std::num::NonZeroU16::new(ROWS).unwrap(),
        std::num::NonZeroU16::new(COLS).unwrap(),
        400,
    );
    let start = Instant::now();

    if !quick {
        wait_and_drain(&mut parser, &rx, Duration::from_millis(1800))?;
    }
    dump(&parser, 0, "initial");

    let mut script: Vec<(Vec<u8>, &str, u64)> = if do_stop {
        vec![
            (b"\r".to_vec(), "start agent one", 1300),
            (b"\x1bOS".to_vec(), "F4 picker", 300),
            (b"\x1b[B\r".to_vec(), "start agent two", 1300),
            (b"\x1bOS".to_vec(), "F4 picker", 300),
            (b"d".to_vec(), "stop highlighted session (two)", 500),
            (b"\x1b".to_vec(), "Esc close picker", 300),
            (b"\x1b[17~".to_vec(), "F6 to remaining session", 500),
            (b"\x11".to_vec(), "Ctrl+Q quit", 250),
        ]
    } else if do_multi {
        vec![
            // Both agents stay alive: each session keeps its own screen.
            (b"\r".to_vec(), "start agent one", 1300),
            (b"\x1bOS".to_vec(), "F4 picker", 300),
            (b"\x1b[B\r".to_vec(), "start agent two (one keeps running)", 1300),
            (b"hello\r".to_vec(), "type hello into two", 500),
            (b"\x1b[18~".to_vec(), "F7 previous session (one)", 500),
            (b"world\r".to_vec(), "type world into one", 500),
            (b"\x1b[17~".to_vec(), "F6 next session (two, hello still there)", 500),
            (b"\x1bOS".to_vec(), "F4 picker with both running", 400),
            (b"\x1b".to_vec(), "Esc close picker", 300),
            (b"\x11".to_vec(), "Ctrl+Q quit", 250),
        ]
    } else {
        let mut script: Vec<(Vec<u8>, &str, u64)> = vec![
            // The app opens the in-app agent picker on launch.
            (b"\r".to_vec(), "choose agent", 1200),
        ];
        if do_add {
            script.push((b"\x1bOP".to_vec(), "F1 focus files", 250));
            script.push((b"\x1b[B".to_vec(), "select src", 200));
            script.push((b"a".to_vec(), "add src/ to agent", 400));
            script.push((b"j".to_vec(), "select Cargo.lock", 200));
            script.push((b"a".to_vec(), "add Cargo.lock to agent", 400));
        }
        script.push((b"\x1bOQ".to_vec(), "F2 focus changes", 450));
        if do_reveal {
            script.push((b"\x1b[B\x1b[B".to_vec(), "changes down x2", 300));
            script.push((b"\r".to_vec(), "reveal in tree", 450));
        }
        script.extend([
            (b"\x1bOP".to_vec(), "F1 focus files", 450),
            (b"\x1bOS".to_vec(), "F4 agent picker", 450),
        ]);
        if do_switch {
            script.push((b"\x1b[B".to_vec(), "picker down", 450));
            script.push((b"\r".to_vec(), "picker select", 1500));
        } else {
            script.push((b"\x1b".to_vec(), "Esc close picker", 450));
        }
        script.push((b"\x1bOR".to_vec(), "F3 focus agent", 450));
        script.push((
            if do_type {
                b"hello\r".to_vec()
            } else {
                Vec::new()
            },
            "type into agent",
            450,
        ));
        script.push((b"\x11".to_vec(), "Ctrl+Q quit", 250));
        script
    };

    if let Some(pick) = pick
        && let Some(first) = script.first_mut()
    {
        first.0 = pick.to_string().into_bytes();
    }
    if let Some(settle) = settle_ms {
        script.insert(1, (Vec::new(), "settle", settle));
    }

    for (index, (bytes, label, delay)) in script.iter().enumerate() {
        wait_and_drain(&mut parser, &rx, Duration::from_millis(250))?;
        if !bytes.is_empty() {
            writer.write_all(bytes)?;
            writer.flush()?;
        }
        wait_and_drain(&mut parser, &rx, Duration::from_millis(*delay))?;
        dump(&parser, index + 1, label);
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        while let Ok(bytes) = rx.try_recv() {
            parser.process(&bytes);
        }
        if let Some(status) = child.try_wait()? {
            println!("=== child exited: {status:?} after {:?} ===", start.elapsed());
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    println!("=== child did not exit, killing ===");
    let _ = child.kill();
    Ok(())
}

fn wait_and_drain(
    parser: &mut vt100::Parser,
    rx: &mpsc::Receiver<Vec<u8>>,
    duration: Duration,
) -> Result<()> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        while let Ok(bytes) = rx.try_recv() {
            parser.process(&bytes);
        }
        thread::sleep(Duration::from_millis(20));
    }
    while let Ok(bytes) = rx.try_recv() {
        parser.process(&bytes);
    }
    Ok(())
}

fn dump(parser: &vt100::Parser, step: usize, label: &str) {
    let contents = parser.screen().contents();
    let lines: Vec<&str> = contents.lines().collect();
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    println!("\n----- step {step}: {label} -----");
    for line in &lines[..end] {
        println!("{}", line.trim_end());
    }
    println!("----- end step {step} -----");
}
