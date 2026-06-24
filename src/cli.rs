//! Command-line argument parsing.
//!
//! Hand-rolled (no `clap`) to keep the dependency surface small. A handful of
//! flags configure the TUI; the `doctor` subcommand runs diagnostics instead of
//! launching it. Anything unrecognized is a hard error so typos don't silently
//! do the wrong thing.

use crate::config;

/// Program version, taken from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Log levels accepted by `--log-level` (matches `tracing`'s filter levels).
const LOG_LEVELS: [&str; 5] = ["trace", "debug", "info", "warn", "error"];

/// What the user asked us to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Launch the TUI (the default).
    Run,
    /// Print diagnostics and exit.
    Doctor,
    /// Print usage and exit.
    Help,
    /// Print version and exit.
    Version,
}

/// Parsed runtime options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub command: Command,
    /// Messages to request per conversation.
    pub message_limit: usize,
    /// Log-level override (`--debug` / `--log-level`); when `None` the
    /// `TEAMSCLI_LOG` env var (else `info`) is used.
    pub log_level: Option<String>,
    /// Whether the background poller runs at all (`--no-live` turns it off).
    pub live_refresh: bool,
    /// Poll interval for the active chat and chat list, in seconds.
    pub poll_interval_secs: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            command: Command::Run,
            message_limit: config::DEFAULT_MESSAGE_LIMIT,
            log_level: None,
            live_refresh: true,
            poll_interval_secs: config::POLL_INTERVAL_SECS,
        }
    }
}

/// Parse CLI arguments (the program name should already be stripped).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut opts = Options::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "doctor" | "--doctor" => opts.command = Command::Doctor,
            "-h" | "--help" => opts.command = Command::Help,
            "--version" | "-V" => opts.command = Command::Version,
            "--debug" => opts.log_level = Some("debug".to_string()),
            "--no-live" => opts.live_refresh = false,
            "--msg" => opts.message_limit = parse_count(&take_value(&mut args, "--msg")?)?,
            "--log-level" => {
                opts.log_level = Some(parse_level(&take_value(&mut args, "--log-level")?)?)
            }
            "--refresh" => {
                opts.poll_interval_secs = parse_secs(&take_value(&mut args, "--refresh")?)?
            }
            _ if arg.starts_with("--msg=") => {
                opts.message_limit = parse_count(&arg["--msg=".len()..])?
            }
            _ if arg.starts_with("--log-level=") => {
                opts.log_level = Some(parse_level(&arg["--log-level=".len()..])?)
            }
            _ if arg.starts_with("--refresh=") => {
                opts.poll_interval_secs = parse_secs(&arg["--refresh=".len()..])?
            }
            _ => return Err(format!("unknown argument {arg:?} (try --help)")),
        }
    }

    Ok(opts)
}

/// Consume the value that follows a space-separated flag (e.g. `--msg 20`).
fn take_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_count(raw: &str) -> Result<usize, String> {
    raw.trim()
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("expected a positive integer, got {raw:?}"))
}

fn parse_secs(raw: &str) -> Result<u64, String> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("expected a positive number of seconds, got {raw:?}"))
}

fn parse_level(raw: &str) -> Result<String, String> {
    let level = raw.trim().to_lowercase();
    if LOG_LEVELS.contains(&level.as_str()) {
        Ok(level)
    } else {
        Err(format!(
            "invalid log level {raw:?} (expected one of {})",
            LOG_LEVELS.join(", ")
        ))
    }
}

/// Help text for `--help` and for argument errors.
pub fn usage() -> String {
    format!(
        "teamscli {VERSION} — a terminal UI for Microsoft Teams

USAGE:
    teamscli [OPTIONS]
    teamscli doctor [OPTIONS]

OPTIONS:
    -h, --help               Show this help and exit
        --version            Show version and exit
        --msg <N>            Messages to load per conversation (default {msg})
        --log-level <LEVEL>  trace | debug | info | warn | error (default info)
        --debug              Shortcut for --log-level debug
        --refresh <SECS>     Background poll interval in seconds (default {refresh})
        --no-live            Disable background polling entirely

COMMANDS:
    doctor                   Run diagnostics (tokens, network, config) and exit
",
        msg = config::DEFAULT_MESSAGE_LIMIT,
        refresh = config::POLL_INTERVAL_SECS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Options, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_when_no_args() {
        let opts = parse_args(&[]).unwrap();
        assert_eq!(opts.command, Command::Run);
        assert_eq!(opts.message_limit, config::DEFAULT_MESSAGE_LIMIT);
        assert_eq!(opts.log_level, None);
        assert!(opts.live_refresh);
        assert_eq!(opts.poll_interval_secs, config::POLL_INTERVAL_SECS);
    }

    #[test]
    fn subcommands_and_info_flags() {
        assert_eq!(parse_args(&["doctor"]).unwrap().command, Command::Doctor);
        assert_eq!(parse_args(&["--help"]).unwrap().command, Command::Help);
        assert_eq!(parse_args(&["-h"]).unwrap().command, Command::Help);
        assert_eq!(
            parse_args(&["--version"]).unwrap().command,
            Command::Version
        );
    }

    #[test]
    fn msg_accepts_space_and_equals_forms() {
        assert_eq!(parse_args(&["--msg", "20"]).unwrap().message_limit, 20);
        assert_eq!(parse_args(&["--msg=200"]).unwrap().message_limit, 200);
    }

    #[test]
    fn debug_and_log_level() {
        assert_eq!(
            parse_args(&["--debug"]).unwrap().log_level.as_deref(),
            Some("debug")
        );
        assert_eq!(
            parse_args(&["--log-level", "WARN"])
                .unwrap()
                .log_level
                .as_deref(),
            Some("warn")
        );
    }

    #[test]
    fn no_live_and_refresh() {
        assert!(!parse_args(&["--no-live"]).unwrap().live_refresh);
        assert_eq!(
            parse_args(&["--refresh=10"]).unwrap().poll_interval_secs,
            10
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_args(&["--bogus"]).is_err());
        assert!(parse_args(&["--msg"]).is_err()); // missing value
        assert!(parse_args(&["--msg", "0"]).is_err()); // non-positive
        assert!(parse_args(&["--msg", "five"]).is_err()); // non-numeric
        assert!(parse_args(&["--log-level", "loud"]).is_err()); // unknown level
        assert!(parse_args(&["--refresh", "0"]).is_err()); // non-positive
    }
}
