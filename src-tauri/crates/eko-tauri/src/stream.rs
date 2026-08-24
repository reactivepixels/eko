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

/// The Subsonic endpoint whose upstream URL takes a cover-art `size`.
///
/// Deliberately narrow. `size` is *also* a parameter of the `stream` endpoint, where the
/// Subsonic spec defines it as a video frame dimension (`"640x480"`) — pushing a bare
/// pixel count onto an audio stream URL would be at best ignored and at worst a 400 on
/// every track. So the size only ever rides on `getCoverArt`.
const COVER_ART_PATH_SUFFIX: &str = "/getCoverArt";

/// Apply the cover-art `size` from the OUTER `stream://` URL to the upstream request.
///
/// Cover URLs are minted **without** a size (`eko_net::urls::cover_art_url`) because the
/// app renders art at seven different edge sizes and one pre-signed URL cannot serve them
/// all. The frontend appends `&size=N` to the outer proxy URL instead, and this is where
/// that lands on the upstream — which keeps the credential-bearing inner URL opaque:
/// nothing in the webview ever does string surgery on it.
///
/// Precedence is **outer, then the upstream's own, then
/// [`eko_net::urls::DEFAULT_COVER_SIZE`]**.
///
/// The middle term is now dormant by construction: `eko-net` is the only minter of cover
/// URLs, and it never bakes a size into the inner URL, so that branch always sees `None`.
/// It is retained deliberately rather than deleted. The old TypeScript `coverArtUrl` did
/// bake the size inside (see `git show 1393cf3:src/subsonic/client.ts`, lines 241-245),
/// and while both minters coexisted, falling straight to the default would have silently
/// downgraded every one of those requests — a 300px image stretched into a 600px hero
/// slot, on 100% of covers. Keeping the branch also keeps the proxy correct for any future
/// consumer that pre-sizes its own URL, which is the cheaper side of the trade.
///
/// An absent, empty, non-numeric or otherwise unusable `size` falls through to the next
/// source rather than erroring. This is a display path: a typo in a query param should
/// cost you the exact pixel size you asked for, never the image. The default is
/// imported, not restated, so it stays tied to the builder whose default it is.
///
/// Adding a query param cannot change a URL's origin, so this never affects the SSRF
/// allowlist decision — which is made against `src_url` before this runs either way.
fn apply_cover_size(src_url: &mut url::Url, outer: &url::Url) {
    if !src_url.path().ends_with(COVER_ART_PATH_SUFFIX) {
        return;
    }
    let numeric_size = |url: &url::Url| {
        url.query_pairs()
            .find(|(k, _)| k == "size")
            .and_then(|(_, v)| v.parse::<u32>().ok())
    };
    let size = numeric_size(outer)
        .or_else(|| numeric_size(src_url))
        .unwrap_or(eko_net::urls::DEFAULT_COVER_SIZE);

    // Rebuild the query so a `size` already on the upstream is replaced, not duplicated.
    let others: Vec<(String, String)> = src_url
        .query_pairs()
        .filter(|(k, _)| k != "size")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    src_url
        .query_pairs_mut()
        .clear()
        .extend_pairs(others)
        .append_pair("size", &size.to_string());
}

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
    // This runs BEFORE anything mutates `src_url`, and `apply_cover_size` only ever adds
    // a query param — which cannot change an origin — so the check below is still the
    // decision that governs the fetch.
    let mut src_url = url::Url::parse(&src)?;
    if !matches!(src_url.scheme(), "http" | "https") {
        return Err("disallowed scheme".into());
    }
    let allowed = allowed.ok_or("no configured server")?;
    if src_url.origin() != url::Url::parse(&allowed)?.origin() {
        return Err("src origin not allowed".into());
    }

    // Cover art only: fold the outer `?size=N` (or the default) into the upstream URL.
    apply_cover_size(&mut src_url, &parsed);

    // Where to start: the first number in `bytes=START-END` (or 0 for the initial load).
    let start: u64 = range
        .as_ref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let end = start + CHUNK - 1;

    let client = reqwest::Client::new();
    // `src_url`, NOT the original `src` string: `apply_cover_size` may have rewritten it,
    // and fetching the raw string would silently discard the requested size.
    let upstream = client
        .get(src_url.as_str())
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

/// `fetch` itself needs a live server (it is a network round-trip end to end), so it is
/// not unit-testable here. The one piece of decision-making it gained *is* pure and is
/// therefore tested directly: [`apply_cover_size`].
#[cfg(test)]
mod tests {
    use super::*;

    const COVER: &str = "https://music.example.com/rest/getCoverArt?u=rod&t=abc&id=al-1";
    const AUDIO: &str = "https://music.example.com/rest/stream?u=rod&t=abc&id=tr-1&format=raw";

    fn applied(upstream: &str, outer: &str) -> url::Url {
        let mut src_url = url::Url::parse(upstream).unwrap();
        apply_cover_size(&mut src_url, &url::Url::parse(outer).unwrap());
        src_url
    }

    fn size_of(url: &url::Url) -> Option<String> {
        url.query_pairs()
            .find(|(k, _)| k == "size")
            .map(|(_, v)| v.into_owned())
    }

    #[test]
    fn outer_size_is_applied_to_the_cover_upstream() {
        for requested in ["80", "120", "160", "200", "300", "512", "600"] {
            let outer = format!("stream://localhost/?src=enc&size={requested}");
            assert_eq!(size_of(&applied(COVER, &outer)).as_deref(), Some(requested));
        }
    }

    #[test]
    fn a_missing_size_on_a_sizeless_upstream_falls_back_to_the_eko_net_default() {
        let url = applied(COVER, "stream://localhost/?src=enc");
        assert_eq!(
            size_of(&url),
            Some(eko_net::urls::DEFAULT_COVER_SIZE.to_string())
        );
    }

    /// The pre-Task-5 case, and the one that matters most today: the frontend's own
    /// `coverArtUrl` bakes the size into the INNER url and puts none on the outer. If the
    /// default won here, every cover in the app would be silently downgraded to 300px —
    /// including the 512 and 600 hero slots.
    #[test]
    fn an_upstream_size_wins_over_the_default_when_the_outer_url_has_none() {
        for baked in ["80", "160", "300", "512", "600"] {
            let upstream =
                format!("https://music.example.com/rest/getCoverArt?id=al-1&size={baked}");
            let url = applied(&upstream, "stream://localhost/?src=enc");
            assert_eq!(
                size_of(&url).as_deref(),
                Some(baked),
                "a baked-in size={baked} must survive an outer url that carries none"
            );
        }
    }

    #[test]
    fn the_outer_size_wins_over_a_baked_in_upstream_size() {
        let url = applied(
            "https://music.example.com/rest/getCoverArt?id=al-1&size=300",
            "stream://localhost/?src=enc&size=600",
        );
        assert_eq!(size_of(&url).as_deref(), Some("600"));
    }

    #[test]
    fn a_malformed_size_falls_through_rather_than_failing() {
        // Cover art is a display path: a junk param costs the requested dimensions,
        // never the image.
        for junk in ["", "abc", "-1", "3.5", "600px", "99999999999999999999"] {
            // …to the default, when the upstream has no size of its own,
            let outer = format!("stream://localhost/?src=enc&size={junk}");
            assert_eq!(
                size_of(&applied(COVER, &outer)),
                Some(eko_net::urls::DEFAULT_COVER_SIZE.to_string()),
                "outer size={junk:?} should have fallen back to the default"
            );
            // …and to the upstream's own size when it has one.
            assert_eq!(
                size_of(&applied(
                    "https://music.example.com/rest/getCoverArt?id=al-1&size=512",
                    &outer
                ))
                .as_deref(),
                Some("512"),
                "outer size={junk:?} should have deferred to the baked-in size"
            );
            // A junk size on the UPSTREAM is skipped too, rather than propagated.
            let junk_upstream =
                format!("https://music.example.com/rest/getCoverArt?id=al-1&size={junk}");
            assert_eq!(
                size_of(&applied(&junk_upstream, "stream://localhost/?src=enc")),
                Some(eko_net::urls::DEFAULT_COVER_SIZE.to_string()),
                "upstream size={junk:?} should have fallen back to the default"
            );
        }
    }

    #[test]
    fn audio_streams_are_never_given_a_size() {
        // Subsonic's `stream` endpoint reads `size` as a video dimension ("640x480").
        // Neither the outer request's size nor the cover default may leak onto it.
        assert_eq!(
            size_of(&applied(AUDIO, "stream://localhost/?src=enc")),
            None
        );
        assert_eq!(
            size_of(&applied(AUDIO, "stream://localhost/?src=enc&size=600")),
            None
        );
    }

    #[test]
    fn an_existing_upstream_size_is_replaced_not_duplicated() {
        // Whichever source wins, the upstream must end up with exactly one `size`.
        for outer in [
            "stream://localhost/?src=enc&size=600", // outer wins
            "stream://localhost/?src=enc",          // baked-in wins
        ] {
            let url = applied(
                "https://music.example.com/rest/getCoverArt?id=al-1&size=300",
                outer,
            );
            assert_eq!(
                url.query_pairs().filter(|(k, _)| k == "size").count(),
                1,
                "duplicate size params for outer={outer}"
            );
        }
    }

    #[test]
    fn the_rest_of_the_upstream_query_survives() {
        // The auth params are what make the upstream URL work at all — rebuilding the
        // query must not drop or reorder them.
        let url = applied(COVER, "stream://localhost/?src=enc&size=512");
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("u".to_string(), "rod".to_string()),
                ("t".to_string(), "abc".to_string()),
                ("id".to_string(), "al-1".to_string()),
                ("size".to_string(), "512".to_string()),
            ]
        );
        assert_eq!(url.origin(), url::Url::parse(COVER).unwrap().origin());
    }
}
