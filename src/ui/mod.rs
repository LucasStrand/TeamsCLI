//! Ratatui rendering. A two-pane layout: chat list on the left, message history
//! and composer on the right, with a status bar along the bottom.

mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
        .split(root[0]);

    draw_chat_list(f, app, columns[0]);
    draw_right(f, app, columns[1]);
    draw_status(f, app, root[1]);

    if app.show_help {
        draw_help(f, f.area());
    }
}

fn draw_chat_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .chats
        .iter()
        .map(|c| ListItem::new(c.label.clone()))
        .collect();

    let mut state = ListState::default();
    if !app.chats.is_empty() {
        state.select(Some(app.selected));
    }

    let title = format!(" Chats ({}) ", app.chats.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .highlight_symbol("› ");

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_right(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    draw_messages(f, app, rows[0]);
    draw_composer(f, app, rows[1]);
}

fn draw_messages(f: &mut Frame, app: &App, area: Rect) {
    let title = match &app.active_chat {
        Some(_) => {
            let name = app
                .chats
                .iter()
                .find(|c| Some(&c.id) == app.active_chat.as_ref())
                .map(|c| c.label.clone())
                .unwrap_or_else(|| "Chat".to_string());
            format!(" {name} ")
        }
        None => " Messages ".to_string(),
    };

    let block = Block::default().borders(Borders::ALL).title(title);

    if app.active_chat.is_none() {
        let hint = Paragraph::new("Select a chat and press Enter to open it.")
            .block(block)
            .style(Style::default().fg(theme::MUTED));
        f.render_widget(hint, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        let time = msg.created.as_deref().map(format_time).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(
                msg.sender.clone(),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {time}"), Style::default().fg(theme::MUTED)),
        ]));
        for text_line in msg.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
        }
        lines.push(Line::from(""));
    }

    if lines.is_empty() && app.loading {
        lines.push(Line::from(Span::styled(
            "Loading…",
            Style::default().fg(theme::MUTED),
        )));
    }

    // Keep the most recent messages in view by scrolling to the bottom.
    let visible = area.height.saturating_sub(2) as usize;
    let scroll = lines.len().saturating_sub(visible) as u16;

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(para, area);
}

fn draw_composer(f: &mut Frame, app: &App, area: Rect) {
    let (border_style, title) = match app.mode {
        Mode::Insert => (Style::default().fg(theme::ACCENT), " Compose (Enter ⏎) "),
        Mode::Normal => (Style::default().fg(theme::MUTED), " Press i to compose "),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);

    let content = if app.mode == Mode::Insert {
        format!("{}_", app.composer)
    } else {
        app.composer.clone()
    };
    let para = Paragraph::new(content).block(block);
    f.render_widget(para, area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let mode = match app.mode {
        Mode::Normal => " NORMAL ",
        Mode::Insert => " INSERT ",
    };
    let who = app.display_name.clone();

    let line = Line::from(vec![
        Span::styled(
            mode,
            Style::default()
                .bg(theme::ACCENT)
                .fg(theme::BG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::raw(app.status.clone()),
        Span::styled(format!("  {who} "), Style::default().fg(theme::MUTED)),
        Span::styled(" ? help · q quit ", Style::default().fg(theme::MUTED)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let popup = centered_rect(60, 60, area);
    f.render_widget(Clear, popup);
    let text = vec![
        Line::from(Span::styled(
            "TeamsCLI — keys",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  j / ↓      move down"),
        Line::from("  k / ↑      move up"),
        Line::from("  Enter      open selected chat"),
        Line::from("  i          compose a message"),
        Line::from("  Esc        cancel compose"),
        Line::from("  ?          toggle this help"),
        Line::from("  q / Ctrl-C quit"),
        Line::from(""),
        Line::from(Span::styled(
            "  press any key to close",
            Style::default().fg(theme::MUTED),
        )),
    ];
    let block = Block::default().borders(Borders::ALL).title(" Help ");
    f.render_widget(Paragraph::new(text).block(block), popup);
}

fn format_time(iso: &str) -> String {
    // Render just HH:MM from an ISO-8601 timestamp; fall back to the raw value.
    use chrono::{DateTime, Local};
    match iso.parse::<DateTime<chrono::Utc>>() {
        Ok(dt) => dt.with_timezone(&Local).format("%H:%M").to_string(),
        Err(_) => iso.to_string(),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
