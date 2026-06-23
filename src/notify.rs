//! Desktop / terminal notifications for new incoming messages.

/// Fire a desktop notification (best-effort) for a new message in a chat the
/// user isn't currently viewing.
pub fn notify_message(sender: &str, preview: &str) {
    let body = if preview.chars().count() > 140 {
        let truncated: String = preview.chars().take(140).collect();
        format!("{truncated}…")
    } else {
        preview.to_string()
    };

    if let Err(e) = notify_rust::Notification::new()
        .summary(&format!("Teams · {sender}"))
        .body(&body)
        .appname("TeamsCLI")
        .show()
    {
        tracing::debug!("desktop notification failed: {e}");
    }
}
