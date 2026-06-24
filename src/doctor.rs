//! `teamscli doctor` — preflight diagnostics that run without taking over the
//! terminal. Each check prints a `[PASS|WARN|FAIL]` line; the process exits
//! non-zero if any check fails, so it's usable in scripts.

use std::time::{Duration, Instant};

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::auth::{device_code, token_store};
use crate::cli::{Options, VERSION};
use crate::config::{self, Config};
use crate::util;

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        }
    }
}

/// Accumulates check results so we can print a summary and derive an exit code.
struct Report {
    checks: Vec<(Status, String, String)>,
}

impl Report {
    fn new() -> Self {
        Self { checks: Vec::new() }
    }

    fn add(&mut self, status: Status, name: impl Into<String>, details: impl Into<String>) {
        self.checks.push((status, name.into(), details.into()));
    }

    fn print(&self) {
        for (status, name, details) in &self.checks {
            println!("[{}] {name}: {details}", status.label());
        }
        let (mut pass, mut warn, mut fail) = (0, 0, 0);
        for (status, _, _) in &self.checks {
            match status {
                Status::Pass => pass += 1,
                Status::Warn => warn += 1,
                Status::Fail => fail += 1,
            }
        }
        println!("\nSummary: {pass} pass, {warn} warn, {fail} fail");
    }

    fn healthy(&self) -> bool {
        !self.checks.iter().any(|(s, _, _)| *s == Status::Fail)
    }
}

/// Run all diagnostics. Returns `true` when nothing failed.
pub async fn run(cfg: &Config, opts: &Options, http: &reqwest::Client) -> bool {
    let mut report = Report::new();

    report.add(
        Status::Pass,
        "Build",
        format!(
            "teamscli {VERSION} on {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    terminal_check(&mut report);
    options_check(&mut report, opts);
    log_dir_check(&mut report, cfg);
    let refresh = token_cache_check(&mut report, cfg);
    network_checks(&mut report).await;
    auth_check(&mut report, cfg, http, refresh).await;

    report.print();
    report.healthy()
}

fn terminal_check(report: &mut Report) {
    match std::env::var("TERM").ok().filter(|t| !t.is_empty()) {
        None => report.add(
            Status::Warn,
            "Terminal",
            "TERM is not set; the TUI may render poorly",
        ),
        Some(term) if term.eq_ignore_ascii_case("dumb") => report.add(
            Status::Warn,
            "Terminal",
            "TERM=dumb; run teamscli in a cursor-addressable terminal",
        ),
        Some(term) => report.add(Status::Pass, "Terminal", format!("TERM={term}")),
    }
}

fn options_check(report: &mut Report, opts: &Options) {
    let live = if opts.live_refresh {
        format!("live refresh every {}s", opts.poll_interval_secs)
    } else {
        "live refresh disabled (--no-live)".to_string()
    };
    let level = opts.log_level.as_deref().unwrap_or("info (default)");
    report.add(
        Status::Pass,
        "Options",
        format!("msg={}, log-level={level}, {live}", opts.message_limit),
    );
}

fn log_dir_check(report: &mut Report, cfg: &Config) {
    match std::fs::create_dir_all(&cfg.log_dir) {
        Ok(()) => report.add(
            Status::Pass,
            "Logs",
            format!("log directory writable at {}", cfg.log_dir.display()),
        ),
        Err(e) => report.add(
            Status::Fail,
            "Logs",
            format!("cannot create {}: {e}", cfg.log_dir.display()),
        ),
    }
}

/// Inspect the token cache and return the cached refresh token (if any) so the
/// auth check can try a live silent refresh with it.
fn token_cache_check(report: &mut Report, cfg: &Config) -> Option<String> {
    let path = &cfg.token_cache_path;
    if !path.exists() {
        report.add(
            Status::Warn,
            "Token Cache",
            format!(
                "no cached token at {}; run `teamscli` to sign in",
                path.display()
            ),
        );
        return None;
    }

    match token_store::load_refresh(path) {
        Some(refresh) => {
            let mut details = format!("refresh token cached at {}", path.display());
            if let Some(warn) = insecure_permissions(path) {
                report.add(Status::Warn, "Token Cache", format!("{details}; {warn}"));
            } else {
                details.push_str(" (0600)");
                report.add(Status::Pass, "Token Cache", details);
            }
            Some(refresh)
        }
        None => {
            report.add(
                Status::Fail,
                "Token Cache",
                format!("{} exists but has no usable refresh token", path.display()),
            );
            None
        }
    }
}

/// On Unix, warn if the token file is readable by group/other.
#[cfg(unix)]
fn insecure_permissions(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    if mode & 0o077 != 0 {
        Some(format!("permissions are {:o}, expected 0600", mode & 0o777))
    } else {
        None
    }
}

#[cfg(not(unix))]
fn insecure_permissions(_path: &std::path::Path) -> Option<String> {
    None
}

async fn network_checks(report: &mut Report) {
    let mut hosts: Vec<String> = Vec::new();
    for url in [
        config::DEVICECODE_URL,
        config::AUTHZ_URL,
        config::CHATSVCAGG_RESOURCE,
        config::DEFAULT_MESSAGING_HOST,
    ] {
        if let Some(host) = host_of(url) {
            if !hosts.iter().any(|h| h == host) {
                hosts.push(host.to_string());
            }
        }
    }

    for host in hosts {
        match dial(&host).await {
            Ok(elapsed) => report.add(
                Status::Pass,
                format!("Network {host}"),
                format!("reachable in {}ms", elapsed.as_millis()),
            ),
            Err(e) => report.add(Status::Fail, format!("Network {host}"), e),
        }
    }
}

/// Open a TCP connection to `host:443`, timing how long it takes.
async fn dial(host: &str) -> Result<Duration, String> {
    let started = Instant::now();
    match timeout(Duration::from_secs(3), TcpStream::connect((host, 443))).await {
        Ok(Ok(_stream)) => Ok(started.elapsed()),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("timed out after 3s".to_string()),
    }
}

/// Validate the cached credentials end-to-end by performing a silent refresh —
/// the same path the app uses on launch.
async fn auth_check(
    report: &mut Report,
    cfg: &Config,
    http: &reqwest::Client,
    refresh: Option<String>,
) {
    let Some(refresh) = refresh else {
        report.add(Status::Warn, "Auth", "skipped (not signed in)");
        return;
    };

    match device_code::refresh(cfg, http, &refresh, config::SKYPE_RESOURCE).await {
        Ok(tokens) => {
            let who = util::display_name_from_token(&tokens.access_token);
            let exp = tokens.expires_at.format("%Y-%m-%d %H:%M UTC");
            report.add(
                Status::Pass,
                "Auth",
                format!("silent refresh OK; signed in as {who}; access token valid until {exp}"),
            );
        }
        Err(e) => report.add(
            Status::Fail,
            "Auth",
            format!("silent refresh failed: {e}; delete the token cache and sign in again"),
        ),
    }
}

/// Extract the host portion of a URL (or a `https://host`-style resource) using
/// simple string slicing — avoids pulling in a URL-parsing dependency.
fn host_of(url: &str) -> Option<&str> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after_scheme
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("");
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_extracts_host() {
        assert_eq!(
            host_of("https://teams.microsoft.com/api/x"),
            Some("teams.microsoft.com")
        );
        assert_eq!(
            host_of("https://chatsvcagg.teams.microsoft.com"),
            Some("chatsvcagg.teams.microsoft.com")
        );
        assert_eq!(host_of("https://host:8080/path"), Some("host"));
        assert_eq!(host_of(""), None);
    }
}
