//! Runtime fetch of build-staged static data.
//!
//! `scripts/stage-web-data.sh` publishes the repo's data files (benchmark results,
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
    response
        .text()
        .await
        .map_err(|e| format!("reading {path}: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub async fn fetch_static_json<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
    let text = fetch_static_text(path).await?;
    serde_json::from_str(&text).map_err(|e| format!("parsing {path}: {e}"))
}
