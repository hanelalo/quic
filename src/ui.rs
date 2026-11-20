use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::agent::AgentSession;
use crate::app::{App, Focus};

const ACCENT: Color = Color::Rgb(122, 162, 247);
const DIM: Color = Color::Rgb(110, 116, 132);
const DIR: Color = Color::Rgb(122, 162, 247);
const GREEN: Color = Color::Rgb(158, 206, 106);
const YELLOW: Color = Color::Rgb(224, 175, 104);
const RED: Color = Color::Rgb(247, 118, 142);
const PURPLE: Color = Color::Rgb(187, 154, 247);
const CYAN: Color = Color::Rgb(115, 218, 202);
const ORANGE: Color = Color::Rgb(255, 158, 100);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < 20 || area.height < 6 {
        let line = Line::from(Span::styled(
            "terminal too small",
            Style::new().fg(RED),
        ));
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }

    let [header_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, app, header_area);

    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(main_area);
    let [tree_area, changes_area] =
        Layout::vertical([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(left_area);

    draw_tree(frame, app, tree_area);
    draw_changes(frame, app, changes_area);
    draw_agent(frame, app, right_area);
    draw_footer(frame, app, footer_area);

    if let Some(selected) = app.picker {
        draw_picker(frame, app, selected);
    }
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::new().fg(ACCENT)
    } else {
        Style::new().fg(Color::Indexed(240))
    }
}

fn block_title(text: impl Into<String>, focused: bool) -> Line<'static> {
    Line::from(Span::styled(
        text.into(),
        Style::new()
            .fg(if focused { ACCENT } else { DIM })
            .add_modifier(Modifier::BOLD),
    ))
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans: Vec<Span> = vec![
        Span::styled(
            " quic ",
            Style::new()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            abbreviate_home(&app.root),
            Style::new().fg(Color::Reset),
        ),
    ];

    if let Some(branch) = &app.git.branch {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("⎇ {branch}"), Style::new().fg(DIM)));
    }

    let right = header_right(app);
    let left_width: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let right_width: usize = right
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let padding = (area.width as usize).saturating_sub(left_width + right_width);
    if padding > 0 {
        spans.push(Span::raw(" ".repeat(padding)));
    }
    spans.extend(right);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn header_right(app: &App) -> Vec<Span<'static>> {
    let active = app.current.and_then(|index| app.agents.get(index));
    let (dot, name) = match (app.active_session(), active) {
        (Some(session), _) if session.exited.is_none() => (GREEN, session.name.clone()),
        (Some(session), _) => (RED, format!("{} (stopped)", session.name)),
        (None, Some(agent)) => (ORANGE, format!("{} (stopped)", agent.name)),
        (None, None) => (DIM, "no agent".to_string()),
    };
    let focus = match app.focus {
        Focus::Tree => "files",
        Focus::Changes => "changes",
        Focus::Agent => "agent",
    };
    let mut spans = vec![Span::styled(format!("● {name}"), Style::new().fg(dot))];
    let running = app.running_count();
    if running > 1 {
        spans.push(Span::styled(
            format!("  · {running} running"),
            Style::new().fg(GREEN),
        ));
    }
    spans.push(Span::styled(
        format!("  · focus: {focus} "),
        Style::new().fg(DIM),
    ));
    spans
}

fn abbreviate_home(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

fn draw_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Tree;
    let count = if app.tree.truncated {
        format!("{}+", app.tree.total)
    } else {
        app.tree.total.to_string()
    };

    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(focused))
        .title(block_title(format!(" FILES · {count} "), focused));
    if app.tree.show_hidden {
        block = block.title_bottom(
            Line::from(Span::styled(" hidden ", Style::new().fg(YELLOW)))
                .alignment(Alignment::Right),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    app.tree_height = inner.height as usize;
    let height = inner.height as usize;
    let width = inner.width as usize;

    if app.tree.rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " empty ",
                Style::new().fg(DIM),
            ))),
            inner,
        );
        return;
    }

    let offset = viewport(
        app.tree.selected,
        height,
        &mut app.tree_offset,
        app.tree.rows.len(),
    );
    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for (index, row) in app.tree.rows.iter().enumerate().skip(offset).take(height) {
        let selected = index == app.tree.selected;
        let arrow = if row.is_dir {
            if row.expanded { "▾ " } else { "▸ " }
        } else {
            "· "
        };
        let name_style = if row.is_dir {
            Style::new().fg(DIR)
        } else {
            Style::new().fg(Color::Reset)
        };
        let spans = vec![
            Span::styled("  ".repeat(row.depth), Style::new().fg(DIM)),
            Span::styled(arrow, Style::new().fg(if row.is_dir { DIR } else { DIM })),
            Span::styled(row.name.clone(), name_style),
        ];
        lines.push(pad_line(spans, width, selected));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_changes(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Changes;
    let count = if app.git.is_repo {
        app.git.changes.len().to_string()
    } else {
        "–".to_string()
    };

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(focused))
        .title(block_title(format!(" CHANGES · {count} "), focused));

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    app.changes_height = inner.height as usize;
    let height = inner.height as usize;
    let width = inner.width as usize;

    if !app.git.is_repo {
        let message = app
            .git
            .error
            .clone()
            .unwrap_or_else(|| "not a git repository".to_string());
        let lines = vec![Line::from(Span::styled(message, Style::new().fg(DIM)))];
        frame.render_widget(
            Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: true }),
            inner,
        );
        return;
    }

    if app.git.changes.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " working tree clean ",
                Style::new().fg(DIM),
            ))),
            inner,
        );
        return;
    }

    let offset = viewport(
        app.changes_sel,
        height,
        &mut app.changes_offset,
        app.git.changes.len(),
    );

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for (index, change) in app
        .git
        .changes
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
    {
        let selected = index == app.changes_sel;
        let status = change.status();
        let badge = format!("{}{}", change.x, change.y);
        let mut spans = vec![
            Span::styled(
                badge,
                Style::new()
                    .fg(status_color(status))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(change.path.clone(), Style::new().fg(Color::Reset)),
        ];
        if let Some(original) = &change.orig {
            spans.push(Span::styled(
                format!("  ← {original}"),
                Style::new().fg(DIM),
            ));
        }
        lines.push(pad_line(spans, width, selected));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn status_color(status: char) -> Color {
    match status {
        'M' => YELLOW,
        'A' => GREEN,
        'D' => RED,
        'R' | 'C' => PURPLE,
        'U' => ORANGE,
        '?' => CYAN,
        _ => DIM,
    }
}

fn draw_agent(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Agent;

    // The block title doubles as a tab bar for every started session.
    let mut spans: Vec<Span<'static>> = vec![Span::styled(
        " AGENT ",
        Style::new()
            .fg(if focused { ACCENT } else { DIM })
            .add_modifier(Modifier::BOLD),
    )];
    let mut any = false;
    for (index, agent) in app.agents.iter().enumerate() {
        let Some(session) = app.sessions.get(index).and_then(|slot| slot.as_ref()) else {
            continue;
        };
        any = true;
        let active = app.current == Some(index);
        let running = session.exited.is_none();
        let unseen = !active && app.unseen.get(index).copied().unwrap_or(false);

        spans.push(Span::styled("│", Style::new().fg(Color::Indexed(238))));
        if active {
            spans.push(Span::styled(
                format!(" {} {} ", agent.name, if running { "●" } else { "○" }),
                Style::new()
                    .bg(ACCENT)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            let (name_style, dot_color) = if !running {
                (Style::new().fg(DIM), RED)
            } else if unseen {
                (Style::new().fg(YELLOW).add_modifier(Modifier::BOLD), YELLOW)
            } else {
                (Style::new().fg(Color::Reset), GREEN)
            };
            spans.push(Span::raw(" "));
            spans.push(Span::styled(agent.name.clone(), name_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                if running { "●" } else { "○" },
                Style::new().fg(dot_color),
            ));
            spans.push(Span::raw(" "));
        }
    }
    if !any {
        spans.push(Span::styled(" none — F4 to choose ", Style::new().fg(DIM)));
    }

    let exited = app
        .active_session()
        .map(|session| session.exited.is_some())
        .unwrap_or(false);

    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border_style(focused))
        .title(Line::from(spans));
    if exited {
        block = block.title_bottom(
            Line::from(Span::styled(
                " stopped — ↵ restart · F4 switch ",
                Style::new().fg(RED),
            ))
            .alignment(Alignment::Right),
        );
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Every session gets the pane's size so background agents reflow too and
    // switching tabs never shows a stale geometry.
    app.pty_rows = inner.height;
    app.pty_cols = inner.width;
    for session in app.sessions.iter_mut().flatten() {
        session.resize(inner.height, inner.width);
    }

    match app.active_session() {
        Some(session) if !session.has_drawn && session.exited.is_none() => {
            draw_starting(frame, session, inner)
        }
        Some(session) => draw_terminal(frame, session, inner, focused),
        None if app.picker.is_none() => draw_placeholder(frame, app, inner),
        None => {}
    }
}

fn draw_starting(frame: &mut Frame, session: &AgentSession, inner: Rect) {
    let line = Line::from(Span::styled(
        format!("Starting {}…", session.name),
        Style::new().fg(DIM),
    ));
    let rect = Rect {
        x: inner.x,
        y: inner.y + inner.height / 2,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), rect);
}

fn draw_terminal(frame: &mut Frame, session: &AgentSession, inner: Rect, focused: bool) {
    let screen = session.term.screen();

    {
        let buffer = frame.buffer_mut();
        for row in 0..inner.height {
            for col in 0..inner.width {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let bg = cell.bgcolor();
                let has_bg = !matches!(bg, vt100::Color::Default);
                let has_contents = cell.has_contents();
                if !has_contents && !has_bg {
                    continue;
                }
                let text: &str = if has_contents { cell.contents() } else { " " };
                if text.is_empty() {
                    continue;
                }

                let fg = convert_color(cell.fgcolor());
                let bg = convert_color(bg);
                let mut style = Style::new().fg(fg).bg(bg);
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    style = style.add_modifier(Modifier::REVERSED);
                }

                let max_width = (inner.width - col) as usize;
                buffer.set_stringn(inner.x + col, inner.y + row, text, max_width, style);
            }
        }
    }

    if focused && !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position(Position::new(inner.x + col, inner.y + row));
        }
    }
}

fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn draw_placeholder(frame: &mut Frame, app: &App, inner: Rect) {
    let mut lines = vec![
        Line::from(Span::styled(
            "No agent running",
            Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Press F4 to choose an agent",
            Style::new().fg(DIM),
        )),
    ];
    if let Some(message) = &app.message
        && message.error
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            message.text.clone(),
            Style::new().fg(RED),
        )));
    }

    let content_height = lines.len() as u16;
    let height = content_height.min(inner.height);
    let y = inner.y + inner.height.saturating_sub(height) / 2;
    let rect = Rect {
        x: inner.x,
        y,
        width: inner.width,
        height,
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), rect);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hints: String = match app.focus {
        Focus::Tree => {
            " j/k · ↵ toggle · a add to agent · h/l expand · . hidden · r refresh · F4 agents · ^Q quit".to_string()
        }
        Focus::Changes => {
            " j/k move · ↵ reveal in tree · g/G top/bottom · r refresh · F1 files · ^Q quit".to_string()
        }
        Focus::Agent => format!(
            " keys → {} · F1 files · F2 changes · F4 agents · F6/F7 switch · ^Q quit",
            app.current_agent_name()
        ),
    };

    let line = if let Some(message) = &app.message {
        Line::from(Span::styled(
            format!(" {}", message.text),
            Style::new().fg(if message.error { RED } else { YELLOW }),
        ))
    } else {
        Line::from(Span::styled(hints, Style::new().fg(DIM)))
    };

    frame.render_widget(Paragraph::new(line), area);
}

fn draw_picker(frame: &mut Frame, app: &App, selected: usize) {
    let area = frame.area();
    let width = 54.min(area.width.saturating_sub(4));
    let height = (app.agents.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let title = if app.current.is_some() || app.sessions.iter().any(|slot| slot.is_some()) {
        " Switch agent "
    } else {
        " Choose agent "
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::from(Span::styled(
            title,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )))
        .title_bottom(
            Line::from(Span::styled(
                " ↵ select · d stop · esc cancel ",
                Style::new().fg(DIM),
            ))
            .alignment(Alignment::Right),
        );
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = inner.width as usize;
    let mut lines = Vec::new();
    for (index, agent) in app.agents.iter().enumerate() {
        let is_current = app.current == Some(index);
        let marker = if is_current { "●" } else { " " };
        let session = app.sessions.get(index).and_then(|slot| slot.as_ref());
        let availability = match session {
            Some(session) if session.exited.is_none() => {
                Span::styled("running", Style::new().fg(GREEN))
            }
            Some(_) => Span::styled("stopped", Style::new().fg(RED)),
            None if agent.available => Span::styled("available", Style::new().fg(DIM)),
            None => Span::styled("not found", Style::new().fg(RED)),
        };
        let name_style = if agent.available {
            Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(DIM)
        };
        let mut command: String = agent.command.chars().take(20).collect();
        if command.chars().count() < agent.command.chars().count() {
            command.pop();
            command.push('…');
        }
        let spans = vec![
            Span::styled(
                format!(" {marker} "),
                Style::new().fg(if is_current { GREEN } else { DIM }),
            ),
            Span::styled(format!("{}. ", index + 1), Style::new().fg(DIM)),
            Span::styled(format!("{:<12}", agent.name), name_style),
            Span::styled(format!("{command:<21}"), Style::new().fg(DIM)),
            availability,
        ];
        lines.push(pad_line(spans, width, index == selected));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn viewport(selected: usize, height: usize, offset: &mut usize, len: usize) -> usize {
    if len == 0 || height == 0 {
        *offset = 0;
        return 0;
    }
    let selected = selected.min(len - 1);
    let max_offset = len.saturating_sub(height);
    if *offset > max_offset {
        *offset = max_offset;
    }
    if selected < *offset {
        *offset = selected;
    } else if selected >= *offset + height {
        *offset = selected + 1 - height;
    }
    *offset
}

fn pad_line(spans: Vec<Span<'static>>, width: usize, selected: bool) -> Line<'static> {
    let spans: Vec<Span<'static>> = if selected {
        spans
            .into_iter()
            .map(|span| {
                Span::styled(
                    span.content.into_owned(),
                    Style::new().add_modifier(Modifier::REVERSED),
                )
            })
            .collect()
    } else {
        spans
    };

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let mut spans = spans;
    if used < width {
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        spans.push(Span::styled(" ".repeat(width - used), style));
    }
    Line::from(spans)
}
