//! Ratatui rendering. A two-pane layout: chat list on the left, message history
//! and composer on the right, with a status bar along the bottom.

mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Focus, Mode, Tab};

/// Border style for a pane, brighter when it has keyboard focus.
fn pane_border(focused: bool) -> Style {
    if focused {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default().fg(theme::MUTED)
    }
}

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
    // Tab bar on top, list below.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    draw_tabs(f, app, rows[0]);

    let visible = app.visible_chats();
    let items: Vec<ListItem> = visible.iter().map(|c| chat_item(c)).collect();

    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(app.selected));
    }

    let title = if app.mode == Mode::Search || !app.filter.is_empty() {
        format!(" /{} ", app.filter)
    } else if visible.is_empty() {
        " — ".to_string()
    } else {
        // Show position so it's clear the list scrolls past the visible window.
        format!(" {}/{} ", app.selected + 1, visible.len())
    };
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(pane_border(app.focus == Focus::Chats))
                .title(title),
        )
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .highlight_symbol("› ");

    f.render_stateful_widget(list, rows[1], &mut state);
}

/// Render one chat row: an unread dot, a `#` for channels, then the label.
fn chat_item(c: &crate::teams::models::ChatSummary) -> ListItem<'static> {
    let mut spans: Vec<Span> = Vec::new();
    if c.unread {
        spans.push(Span::styled(
            "● ",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("  "));
    }
    if c.kind.is_channel() {
        spans.push(Span::styled("# ", Style::default().fg(theme::MUTED)));
    }
    let label_style = if c.unread {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    spans.push(Span::styled(c.label.clone(), label_style));
    ListItem::new(Line::from(spans))
}

/// The All / DMs / Channels tab strip, with per-tab unread counts.
fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let (all, dms, chans) = app.unread_counts();
    let counts = [all, dms, chans];
    let mut spans: Vec<Span> = Vec::new();
    for (i, tab) in Tab::ORDER.iter().enumerate() {
        let active = *tab == app.tab;
        let unread = counts[i];
        let mut text = format!(" {} ", tab.label());
        if unread > 0 {
            text = format!(" {} {} ", tab.label(), unread);
        }
        let style = if active {
            Style::default()
                .bg(theme::ACCENT)
                .fg(theme::BG)
                .add_modifier(Modifier::BOLD)
        } else if unread > 0 {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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
            let summary = app
                .chats
                .iter()
                .find(|c| Some(&c.id) == app.active_chat.as_ref());
            match summary {
                Some(c) => match &c.team {
                    // Channels show their parent team for context.
                    Some(team) => format!(" {team} / {} ", c.label),
                    None => format!(" {} ", c.label),
                },
                None => " Chat ".to_string(),
            }
        }
        None => " Messages ".to_string(),
    };

    let border = pane_border(app.focus == Focus::Messages);

    if app.active_chat.is_none() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(title);
        let hint = Paragraph::new("Select a chat (j/k), then Tab or l to read it.")
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

    // Border-inset content dimensions.
    let inner_w = area.width.saturating_sub(2);
    let inner_h = area.height.saturating_sub(2);

    // Build the paragraph first so we can ask it for the real wrapped line
    // count (long messages word-wrap into several physical rows — counting the
    // logical lines would undercount and hide the newest messages).
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = para.line_count(inner_w) as u16;
    let max_scroll = total.saturating_sub(inner_h);

    // Clamp the user's scroll offset (rows up from the bottom) to the real
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(title);

    let para = para.block(block).scroll((top, 0));
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
        Line::from(Span::styled(
            "  acts on the focused pane (chats ↔ messages)",
            Style::default().fg(theme::MUTED),
        )),
        Line::from(""),
        Line::from("  Tab / h / l   switch focus (chats ↔ messages)"),
        Line::from("  1 / 2 / 3     tabs: All / DMs / Channels"),
        Line::from("  [ / ]         cycle tabs"),
        Line::from("  j / k         down / up"),
        Line::from("  Ctrl-D / U    half-page down / up"),
        Line::from("  gg / G        jump to top / bottom"),
        Line::from("  mouse wheel   scroll the focused pane"),
        Line::from("  Enter         open & focus the message view"),
        Line::from("  /             search / filter chats"),
        Line::from("  i             compose a message"),
        Line::from("  Esc           leave compose / filter / messages"),
        Line::from("  ?             toggle this help"),
        Line::from("  q / Ctrl-C    quit"),
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
