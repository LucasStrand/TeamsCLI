//! TeamsCLI — a terminal UI for Microsoft Teams over its internal API.

mod app;
mod auth;
mod config;
mod event;
mod notify;
mod teams;
mod ui;
mod util;

use std::io::{self, Stdout, Write};

use anyhow::{Context, Result};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CtEvent, EventStream, KeyEventKind,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::{self, UnboundedSender};

use app::{Action, App};
use config::Config;
use event::Event;
use teams::TeamsClient;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::load()?;
    init_logging(&cfg);

    let http = reqwest::Client::builder()
        .user_agent("TeamsCLI/0.1")
        .build()?;

    // --- Authentication happens before the TUI takes over the terminal. ---
    let tokens = auth::authenticate(&cfg, &http, |dc| {
        println!("\n  To sign in, open: {}", dc.verification_url);
        println!("  And enter the code: {}\n", dc.user_code);
        println!("  Waiting for authorization…");
        let _ = io::stdout().flush();
    })
    .await
    .context("authentication failed")?;

    let display_name = util::display_name_from_token(&tokens.access_token);
    tracing::info!("signed in as {display_name}");

    let teams = TeamsClient::new(http, cfg.clone(), tokens);
    let mut application = App::new(display_name);

    let result = run_tui(&mut application, teams, &cfg).await;

    // Always restore the terminal, even on error.
    result
}

async fn run_tui(application: &mut App, teams: TeamsClient, cfg: &Config) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

    spawn_input_reader(tx.clone());
    spawn_ticker(tx.clone(), cfg);

    // Kick off the initial chat list load.
    app::poller::load_chats(teams.clone(), tx.clone());

    // Refresh the whole chat list every Nth tick to drive recency, unread, and
    // notifications for chats other than the open one.
    const REFRESH_EVERY_TICKS: u32 = 3;
    let mut tick_count: u32 = 0;

    loop {
        terminal.draw(|f| ui::draw(f, application))?;

        let Some(ev) = rx.recv().await else { break };
        match ev {
            Event::Input(key) => {
                let action = application.on_key(key);
                handle_action(action, application, &teams, &tx);
            }
            Event::Scroll(up) => application.wheel(up),
            Event::Resize => {}
            Event::Tick => {
                maybe_poll(application, &teams, &tx);
                tick_count += 1;
                if tick_count.is_multiple_of(REFRESH_EVERY_TICKS) {
                    app::poller::refresh_chats(teams.clone(), tx.clone());
                }
            }
            Event::Chats(Ok(chats)) => application.set_chats(chats),
            Event::Chats(Err(e)) => {
                application.loading = false;
                application.status = format!("Could not load chats: {e}");
            }
            Event::ChatsRefresh(Ok(chats)) => {
                for n in application.merge_chats(chats) {
                    notify::notify_message(&n.title, &n.body);
                }
            }
            Event::ChatsRefresh(Err(e)) => {
                tracing::debug!("chat refresh failed: {e}");
            }
            Event::Messages {
                chat_id,
                result,
                initial,
            } => match result {
                Ok(msgs) => application.apply_messages(&chat_id, msgs, initial),
                Err(e) => {
                    application.loading = false;
                    application.poll_in_flight = false;
                    application.status = format!("Message fetch failed: {e}");
                }
            },
            Event::Sent(Ok(())) => {
                application.status = "Sent".to_string();
                // Poll right away so the sent message shows up promptly.
                maybe_poll(application, &teams, &tx);
            }
            Event::Sent(Err(e)) => {
                application.status = format!("Send failed: {e}");
            }
            Event::People(people) => application.set_people(people),
            Event::PersonResolved(Ok(person)) => application.add_resolved_person(person),
            Event::PersonResolved(Err(e)) => {
                application.status = format!("Lookup failed: {e}");
            }
            Event::ChatCreated(Ok(chat_id)) => {
                application.finish_new_chat();
                application.open_chat(chat_id.clone());
                app::poller::open_chat(teams.clone(), tx.clone(), chat_id);
                // Refresh the list so the new conversation appears in it.
                app::poller::refresh_chats(teams.clone(), tx.clone());
            }
            Event::ChatCreated(Err(e)) => {
                application.status = format!("Could not create chat: {e}");
            }
        }

        // Auto-open whatever chat the selection now points at (no Enter needed).
        if let Some(chat_id) = application.take_pending_open() {
            application.open_chat(chat_id.clone());
            app::poller::open_chat(teams.clone(), tx.clone(), chat_id);
        }

        if application.should_quit {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

fn handle_action(
    action: Action,
    application: &mut App,
    teams: &TeamsClient,
    tx: &UnboundedSender<Event>,
) {
    match action {
        Action::None => {}
        Action::Quit => application.should_quit = true,
        Action::SendMessage { chat_id, text } => {
            app::poller::send_message(teams.clone(), tx.clone(), chat_id, text);
        }
        Action::LoadPeople => app::poller::load_people(teams.clone(), tx.clone()),
        Action::ResolvePerson(email) => {
            app::poller::resolve_person(teams.clone(), tx.clone(), email);
        }
        Action::CreateChat { members, topic } => {
            app::poller::create_chat(teams.clone(), tx.clone(), members, topic);
        }
    }
}

/// On each tick, re-fetch the active chat's messages if no poll is already
/// outstanding.
fn maybe_poll(application: &mut App, teams: &TeamsClient, tx: &UnboundedSender<Event>) {
    if application.poll_in_flight {
        return;
    }
    if let Some(chat_id) = application.active_chat.clone() {
        application.poll_in_flight = true;
        app::poller::poll_messages(teams.clone(), tx.clone(), chat_id);
    }
}

fn spawn_input_reader(tx: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let mut reader = EventStream::new();
        while let Some(Ok(ev)) = reader.next().await {
            let out = match ev {
                CtEvent::Key(key) if key.kind != KeyEventKind::Release => Some(Event::Input(key)),
                CtEvent::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollUp => Some(Event::Scroll(true)),
                    MouseEventKind::ScrollDown => Some(Event::Scroll(false)),
                    _ => None,
                },
                CtEvent::Resize(_, _) => Some(Event::Resize),
                _ => None,
            };
            if let Some(out) = out {
                if tx.send(out).is_err() {
                    break;
                }
            }
        }
    });
}

fn spawn_ticker(tx: UnboundedSender<Event>, cfg: &Config) {
    let interval = std::time::Duration::from_secs(config::POLL_INTERVAL_SECS);
    let _ = cfg; // interval currently fixed; cfg reserved for future tuning
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if tx.send(Event::Tick).is_err() {
                break;
            }
        }
    });
}

type Tui = Terminal<CrosstermBackend<Stdout>>;

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn init_logging(cfg: &Config) {
    use tracing_subscriber::EnvFilter;

    if std::fs::create_dir_all(&cfg.log_dir).is_err() {
        return;
    }
    let file_appender = tracing_appender::rolling::daily(&cfg.log_dir, "teamscli.log");
    let filter = EnvFilter::try_from_env("TEAMSCLI_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(file_appender)
        .with_ansi(false)
        .try_init();
}
