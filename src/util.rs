//! Small shared helpers.

use base64::Engine;

/// Decode the (unverified) claims of a JWT. We only use this to read display
/// fields like `name`/`upn` from our own access token — never for trust
/// decisions.
pub fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The signed-in user's Teams MRI (`8:orgid:<object-id>`), derived from the
/// `oid` claim of their access token. Used to identify "me" in chat rosters.
pub fn my_mri_from_token(token: &str) -> Option<String> {
    let oid = jwt_claims(token)?
        .get("oid")?
        .as_str()
        .map(|s| s.to_string())?;
    Some(format!("8:orgid:{oid}"))
}

/// Best-effort display name for the signed-in user from their access token.
pub fn display_name_from_token(token: &str) -> String {
    let claims = jwt_claims(token);
    let pick = |key: &str| {
        claims
            .as_ref()
            .and_then(|c| c.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    pick("name")
        .or_else(|| pick("upn"))
        .or_else(|| pick("unique_name"))
        .or_else(|| pick("preferred_username"))
        .unwrap_or_else(|| "signed in".to_string())
}

/// Strip HTML tags and decode a few common entities — enough to render Teams
/// message bodies (which are often `RichText/Html`) in a terminal. Used for the
/// short last-message preview; full message bodies go through
/// [`render_message_html`] instead.
pub fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_entities(&out).trim().to_string()
}

/// A hosted-image reference pulled out of a message body (Azure Media Service).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlImage {
    /// The object URL to fetch (carries the skypetoken when requested).
    pub url: String,
    /// A best-effort display name for the chip.
    pub name: String,
}

/// Render a Teams `RichText/Html` body into plain text plus any hosted-image
/// references. Unlike [`strip_html`], this keeps emoji (encoded as
/// `<img … alt="🙂">`) by inlining their alt text, and pulls AMS image
/// attachments out so the caller can render them as chips / inline images.
pub fn render_message_html(input: &str) -> (String, Vec<HtmlImage>) {
    let mut text = String::with_capacity(input.len());
    let mut images = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '<' {
            text.push(c);
            continue;
        }
        // Capture the tag body up to the closing `>`.
        let mut tag = String::new();
        for t in chars.by_ref() {
            if t == '>' {
                break;
            }
            tag.push(t);
        }
        handle_tag(&tag, &mut text, &mut images);
    }

    (tidy(&decode_entities(&text)), images)
}

/// Apply one HTML tag's effect to the running text / image list.
fn handle_tag(tag: &str, text: &mut String, images: &mut Vec<HtmlImage>) {
    let trimmed = tag.trim();
    let is_close = trimmed.starts_with('/');
    let name = trimmed
        .trim_start_matches('/')
        .split([' ', '\t', '\n', '/'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    match name.as_str() {
        "img" if !is_close => {
            let alt = attr(tag, "alt");
            let src = attr(tag, "src").unwrap_or_default();
            let itemtype = attr(tag, "itemtype")
                .unwrap_or_default()
                .to_ascii_lowercase();
            let src_l = src.to_ascii_lowercase();

            let is_emoji = itemtype.contains("emoji")
                || src_l.contains("/emoji")
                || src_l.contains("evergreen-assets")
                || src_l.contains("statics.teams");
            let is_ams = itemtype.contains("amsimage")
                || src_l.contains("asm.skype.com")
                || src_l.contains("/v1/objects/");

            if is_emoji {
                // Inline the unicode the emoji image stands in for.
                if let Some(a) = alt {
                    text.push_str(&a);
                }
            } else if is_ams && !src.is_empty() {
                let label = alt
                    .or_else(|| attr(tag, "itemid"))
                    .unwrap_or_else(|| "image".into());
                images.push(HtmlImage {
                    url: src,
                    name: label,
                });
            } else if let Some(a) = alt {
                text.push_str(&a);
            }
        }
        // Block boundaries become line breaks so multi-line messages survive.
        "br" if !is_close => text.push('\n'),
        "div" | "p" | "li" if is_close => text.push('\n'),
        _ => {}
    }
}

/// Read an attribute value (`name="…"`, `name='…'`, or unquoted) from a tag,
/// matching the name case-insensitively but preserving the value's case.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let pos = lower.find(&key)?;
    let after = tag[pos + key.len()..].trim_start();
    let mut ch = after.chars();
    match ch.next() {
        Some(q @ ('"' | '\'')) => {
            let rest = &after[1..];
            let end = rest.find(q)?;
            Some(rest[..end].to_string())
        }
        Some(_) => {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            Some(after[..end].to_string())
        }
        None => None,
    }
}

/// Decode the handful of HTML entities Teams actually emits.
fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Trim trailing spaces, drop leading/trailing blank lines, and collapse runs
/// of blank lines down to a single one.
fn tidy(s: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() && out.last() == Some(&"") {
            continue; // collapse consecutive blanks
        }
        out.push(line);
    }
    while out.first() == Some(&"") {
        out.remove(0);
    }
    while out.last() == Some(&"") {
        out.pop();
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags_and_decodes_entities() {
        let html = "<div>Hello&nbsp;<b>world</b> &amp; <i>all</i>!</div>";
        assert_eq!(strip_html(html), "Hello world & all!");
    }

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_html("just text"), "just text");
    }

    #[test]
    fn keeps_emoji_alt_text() {
        let html = "<div>Hello everyone <span class=\"x\"><img itemid=\"smile\" \
            itemtype=\"http://schema.skype.com/Emoji\" \
            src=\"https://statics.teams.cdn.office.net/.../20.png\" alt=\"🙂\"></span></div>";
        let (text, images) = render_message_html(html);
        assert_eq!(text, "Hello everyone 🙂");
        assert!(images.is_empty());
    }

    #[test]
    fn extracts_ams_image_attachment() {
        let html = "<div>see this <img itemtype=\"http://schema.skype.com/AMSImage\" \
            src=\"https://eu-api.asm.skype.com/v1/objects/0-eu-d1-abc/views/imgo\" \
            alt=\"shot.png\"></div>";
        let (text, images) = render_message_html(html);
        assert_eq!(text, "see this");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "shot.png");
        assert!(images[0].url.contains("/v1/objects/0-eu-d1-abc/"));
    }

    #[test]
    fn block_tags_become_newlines() {
        let (text, _) = render_message_html("<div>line one</div><div>line two</div>");
        assert_eq!(text, "line one\nline two");
    }

    #[test]
    fn plain_body_passes_through() {
        let (text, images) = render_message_html("just text &amp; more");
        assert_eq!(text, "just text & more");
        assert!(images.is_empty());
    }
}
