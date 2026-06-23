//! Ratatui rendering. A two-pane layout: chat list on the left, message history
//! and composer on the right, with a status bar along the bottom.

mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Mode};

pub fn draw(f: &mut Frame, app: &mut App) {
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
    let visible = app.visible_chats();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|c| ListItem::new(c.label.clone()))
        .collect();

    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(app.selected));
    }

    let title = if app.mode == Mode::Search || !app.filter.is_empty() {
        format!(" Chats  /{}", app.filter)
    } else {
        format!(" Chats ({}) ", visible.len())
    };
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

fn draw_right(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    draw_messages(f, app, rows[0]);
    draw_composer(f, app, rows[1]);
}

fn draw_messages(f: &mut Frame, app: &mut App, area: Rect) {
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

    if app.active_chat.is_none() {
        let block = Block::default().borders(Borders::ALL).title(title);
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

    // `area.height - 2` accounts for the top/bottom borders.
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(visible) as u16;

    // Clamp the user's scroll offset (lines up from the bottom) to the real
    // maximum, writing it back so over-scroll doesn't accumulate.
    if app.msg_scroll > max_scroll {
        app.msg_scroll = max_scroll;
    }
    // Default view is pinned to the bottom (newest); msg_scroll moves it up.
    let top = max_scroll.saturating_sub(app.msg_scroll);

    let scrolled = app.msg_scroll > 0;
    let title = if scrolled {
        format!("{title}↑ ")
    } else {
        title
    };
    let block = Block::default().borders(Borders::ALL).title(title);

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((top, 0));
    f.render_widget(para, area);
}

fn draw_composer(f: &mut Frame, app: &App, area: Rect) {
    let (border_style, title) = match app.mode {
        Mode::Insert => (Style::default().fg(theme::ACCENT), " Compose (Enter ⏎) "),
        _ => (Style::default().fg(theme::MUTED), " Press i to compose "),
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
        Mode::Search => " SEARCH ",
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
        Line::from("  j / ↓        move down (opens chat automatically)"),
        Line::from("  k / ↑        move up"),
        Line::from("  PgUp / PgDn  scroll messages (or mouse wheel)"),
        Line::from("  Ctrl-U/D     scroll messages up / down"),
        Line::from("  Home / End   jump to oldest / newest"),
        Line::from("  /            search / filter chats"),
        Line::from("  i            compose a message"),
        Line::from("  Esc          cancel compose / clear filter"),
        Line::from("  ?            toggle this help"),
        Line::from("  q / Ctrl-C   quit"),
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
