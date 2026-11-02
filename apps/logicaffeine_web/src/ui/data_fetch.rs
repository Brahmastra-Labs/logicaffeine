//! Runtime fetch of build-staged static data.
//!
//! `scripts/web/stage-web-data.sh` publishes the repo's data files (benchmark results,
//! program sources, legal HTML, lexicon) under `/data/` next to the app bundle.
//! Native builds compile the same bytes in via `include_str!`; these helpers are the
//! wasm side of that split, so the data never rides inside the shipped binary.

#[cfg(target_arch = "wasm32")]
pub async fn fetch_static_text(path: &str) -> Result<String, String> {
    let response = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| format!("fetching {path}: {e}"))?;
    if !response.ok() {
        return Err(format!("fetching {path}: HTTP {}", response.status()));
    }
    let content_type = response.headers().get("content-type");
    let text = response
        .text()
        .await
        .map_err(|e| format!("reading {path}: {e}"))?;
    check_served(path, content_type.as_deref(), &text)?;
    Ok(text)
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_static_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let text = fetch_static_text(path).await?;
    serde_json::from_str(&text).map_err(|e| format!("parsing {path}: {e}"))
}

/// Whether what came back for a staged `/data` file is that file. A server with an SPA
/// fallback answers a missing file with the app's `index.html` and a 200, so a missing
/// `lexicon.json` would otherwise surface as a JSON parse error ("expected value at line 1
/// column 1") instead of saying the file is not deployed.
pub fn check_served(path: &str, content_type: Option<&str>, body: &str) -> Result<(), String> {
    if path.ends_with(".html") || path.ends_with(".htm") {
        return Ok(());
    }
    let head: String = body.trim_start().chars().take(15).collect::<String>().to_ascii_lowercase();
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        let served_as = content_type.map(|t| format!(" as {t}")).unwrap_or_default();
        return Err(format!(
            "{path} is missing from this deployment — the server answered with its HTML page{served_as} (the /data bundle was not staged)"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/data_fetch/tests.rs"]
mod tests;
