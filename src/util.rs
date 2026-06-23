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
/// message bodies (which are often `RichText/Html`) in a terminal.
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
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
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
}
