//! The page half of CSRF: a page document carries its visitor's token in its head, where the
//! browser host reads it and sends it back as `x-csrf-token` on the page's state-changing requests
//! to its own server. One definition for every tier that composes or reads a page — the dev
//! server, a compiled server, the in-process host and the browser runtime's shared name.

/// The `<meta name>` a page document carries its visitor's CSRF token under.
pub const CSRF_META_NAME: &str = "csrf-token";

/// `document` carrying `token` in its head as `<meta name="csrf-token">`. The meta opens the head,
/// so a page's own `Head:` merges over it by identity like every other head element. A document
/// with no `<head>` is returned as it is.
pub fn with_csrf_meta(document: &str, token: &str) -> String {
    let Some(open) = document
        .to_ascii_lowercase()
        .find("<head>")
        .map(|at| at + "<head>".len())
    else {
        return document.to_string();
    };
    let escaped = token
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    let tag = format!("\n<meta name=\"{CSRF_META_NAME}\" content=\"{escaped}\">");
    format!("{}{tag}{}", &document[..open], &document[open..])
}
