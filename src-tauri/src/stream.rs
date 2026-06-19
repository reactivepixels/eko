//! `stream://` URI-scheme proxy for Navidrome (OpenSubsonic).
//!
//! The webview asks for `stream://localhost/?src=<urlencoded upstream URL>` and we
//! forward the request to Navidrome, honoring HTTP **Range** so playback is progressive:
//! the audio element gets the first ~1 MiB almost immediately and pulls more as it plays
//! (and when seeking). Because the bytes are served same-origin with `Access-Control-
//! Allow-Origin: *`, the `<audio>` element is never cross-origin tainted, so the Web Audio
//! EQ + spectrum keep working — unlike pointing `<audio>` straight at Navidrome.

use tauri::http::{header, Response, StatusCode};

/// Bytes served per response. Small enough that the first chunk arrives fast, large
/// enough to avoid an excessive number of follow-up range requests.
const CHUNK: u64 = 1024 * 1024;

/// Always returns a response (errors become a 502 with CORS headers so the element fails
/// cleanly rather than hanging). `allowed` is the configured Navidrome origin (set on
/// connect); the proxy refuses to fetch anything that isn't that exact origin — without it
/// the proxy would be an open SSRF primitive that webview code could point at internal hosts
/// (cloud metadata, localhost services, the LAN).
pub async fn proxy(
    uri: String,
    range: Option<String>,
    allowed: Option<String>,
) -> Response<Vec<u8>> {
    match fetch(uri, range, allowed).await {
        Ok(r) => r,
        Err(_) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(Vec::new())
            .unwrap(),
    }
}

async fn fetch(
    uri: String,
    range: Option<String>,
    allowed: Option<String>,
) -> Result<Response<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = url::Url::parse(&uri)?;
    let src = parsed
        .query_pairs()
        .find(|(k, _)| k == "src")
        .map(|(_, v)| v.into_owned())
        .ok_or("missing src param")?;

    // SSRF guard: only proxy http(s) to the configured Navidrome server's exact origin.
    let src_url = url::Url::parse(&src)?;
    if !matches!(src_url.scheme(), "http" | "https") {
        return Err("disallowed scheme".into());
    }
    let allowed = allowed.ok_or("no configured server")?;
    if src_url.origin() != url::Url::parse(&allowed)?.origin() {
        return Err("src origin not allowed".into());
    }

    // Where to start: the first number in `bytes=START-END` (or 0 for the initial load).
    let start: u64 = range
        .as_ref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let end = start + CHUNK - 1;

    let client = reqwest::Client::new();
    let upstream = client
        .get(src.as_str())
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?;

    let status = upstream.status();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    // Total file size lives at the tail of Content-Range: "bytes start-end/TOTAL".
    let total: Option<u64> = upstream
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit('/').next().map(str::to_string))
        .and_then(|t| t.trim().parse().ok());

    let body = upstream.bytes().await?.to_vec();
    let len = body.len() as u64;

    let mut builder = Response::builder()
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, len.to_string());

    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let actual_end = start + len.saturating_sub(1);
        let total_str = total.map(|t| t.to_string()).unwrap_or_else(|| "*".into());
        builder = builder.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{actual_end}/{total_str}"),
        );
    } else {
        // Upstream ignored Range and sent the whole file — serve it as a plain 200.
        builder = builder.status(StatusCode::OK);
    }

    Ok(builder.body(body)?)
}
