//! The server's library, made browsable — off the render thread, and safe to
//! draw.
//!
//! The twin of [`crate::library`] for a Navidrome / OpenSubsonic server. It has
//! the same shape (albums, each with tracks), reports progress the same way (an
//! [`crate::app::AppEvent`] per page, from a worker thread), and is navigated by
//! the same keys — [`crate::keys`] is not forked for it.
//!
//! Three things are genuinely different from the local library, and all three
//! are the reason this is its own module.
//!
//! ## 1. Paging, and the termination rule that is not obvious
//!
//! `getAlbumList2` pages. The Subsonic spec caps `size` at 500, so any library
//! larger than that **requires** a page walk — and the obvious walk is wrong.
//!
//! ```text
//! while page.len() == PAGE_SIZE { offset += PAGE_SIZE; … }   // WRONG
//! ```
//!
//! Some servers silently cap the page size below what was asked for. Against one
//! of those, the first page comes back short, the loop decides the library ended
//! there, and the user sees a fraction of their music with nothing on screen to
//! say so. That is a real bug this project has already shipped once — the rule is
//! written up at `src/subsonic/useSubsonic.ts:185` in the desktop app, and
//! `eko-net` keeps a fixture for the shape that causes it
//! (`tests/fixtures/albumlist2_capped.json`: a page of 2 in answer to `size=500`,
//! whose whole point is that the parser exposes **no** "is this the last page?"
//! signal, because there isn't one).
//!
//! So [`walk_albums`] terminates on an **empty** page and on nothing else, and
//! advances `offset` by the page's **actual** length rather than by the size it
//! asked for. It costs one extra round trip per walk. That is the price of not
//! truncating somebody's library.
//!
//! ## 2. Every string here was written by a machine you do not control
//!
//! Album names, artist names and track titles arrive from the server and are
//! rendered into a terminal. A raw `ESC` in one of them is not a cosmetic
//! problem: it can move the cursor, clear the screen, or repaint the row the
//! signal-path seal lives on — and the seal is the one thing in this application
//! that must never be able to say something the code did not derive.
//!
//! Every server-supplied string is therefore passed through
//! [`crate::server::tidy`] **at the boundary**, in [`Album::from_sub`] and
//! [`Track::from_sub`], before a [`Album`] or [`Track`] exists at all. There is
//! one gate, not one per field and not one per widget: nothing downstream has to
//! remember to sanitise, because nothing downstream ever sees the raw text.
//!
//! ## 3. It is not the filesystem, so it can fail halfway
//!
//! A local scan either finds a folder or does not. A page walk can deliver four
//! pages and then lose the network. The events below therefore carry partial
//! results (`Page`) separately from the outcome (`AlbumsReady` / `AlbumsFailed`),
//! so the Deck can show what it has *and* say that it is incomplete.

use std::sync::Arc;

use eko_net::types::{SubAlbum, SubSong};
use eko_net::Client;

use crate::server;

/// Albums per `getAlbumList2` request — the Subsonic spec's maximum, and
/// `eko-net`'s own default.
///
/// A server is free to send fewer. See the module docs: that is not a signal.
pub const PAGE_SIZE: u32 = 500;

/// Safety stop, mirroring `useSubsonic.ts`'s `MAX_ALBUMS`.
///
/// The walk only ends on an empty page, so a server that ignores `offset` and
/// answers the same page forever would spin here until the process died. 300,000
/// albums is far past any real library and ~600 requests at a full page.
pub const MAX_ALBUMS: usize = 300_000;

/// What an album with no usable `name` is called.
///
/// The same words [`crate::library`] uses for an untagged local album, because
/// the two lists sit under the same cursor and a user should not have to learn
/// which source they are looking at from the placeholder text.
pub const UNKNOWN_ALBUM: &str = "Unknown Album";
/// What an album or track with no usable `artist` is called.
pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
/// What a track with no usable `title` is called.
///
/// The local library falls back to the file's path. There is no path here — the
/// only other server-supplied handle is the id, which is an opaque token rather
/// than anything a person could read — so this is a placeholder rather than a
/// substitute.
pub const UNTITLED: &str = "Untitled";

// ── The model ────────────────────────────────────────────────────────────────

/// One streamable track on the server.
///
/// Every string field has been through [`server::tidy`]; see the module docs.
///
/// It carries **no URL**. `eko-net` decorates `SubSong` with four pre-signed URLs
/// and two of them (`stream_url`, `cover_url`) are `stream://`-wrapped for
/// Tauri's protocol handler and unresolvable here. `App::play_remote` mints the
/// direct one ([`eko_net::urls::stream_src_url`]) from this id at the moment it
/// plays and drops it again, so nothing in this module holds a signed credential
/// in memory for the life of the session — a signed URL carries `u`, `t` and `s`,
/// which is a replayable credential, and a four-thousand-track library would hold
/// four thousand of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// The server's id. Opaque, tidied, and the only thing needed to stream it.
    pub id: String,
    pub title: String,
    pub artist: String,
    /// The album the server says this track is on.
    ///
    /// Redundant inside an album's own track list — the album is right there —
    /// and load-bearing outside one. A song that arrives from
    /// [`search`](fetch_search) has no album around it, and a queue entry with no
    /// album name renders as `Artist · ` in the footer. The wire has always
    /// carried it (`SubSong::album`), so this is reading a field rather than
    /// inventing one.
    pub album: String,
    pub track_no: Option<u32>,
    /// Seconds. `0.0` when the server did not say.
    pub duration: f64,
}

impl Track {
    /// Sanitise and narrow a wire `SubSong`. **The boundary.**
    ///
    /// `None` for a track whose id is unusable — see [`safe_id`].
    fn from_sub(song: &SubSong) -> Option<Self> {
        Some(Self {
            id: safe_id(&song.id)?,
            title: or_placeholder(&song.title, UNTITLED),
            artist: or_placeholder(&song.artist, UNKNOWN_ARTIST),
            album: or_placeholder(&song.album, UNKNOWN_ALBUM),
            track_no: song.track,
            duration: f64::from(song.duration.unwrap_or(0)),
        })
    }
}

/// One album on the server, and its tracks once they have been fetched.
///
/// Every string field has been through [`server::tidy`]; see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: String,
    /// The server's own count, from the listing. Present before the tracks are.
    pub song_count: Option<u32>,
    /// `None` until `getAlbum` has answered for this album.
    ///
    /// The distinction is load-bearing: `None` is "not asked yet", `Some(vec![])`
    /// is "asked, and this album really is empty". Collapsing the two would make
    /// an album that is still loading indistinguishable from one with nothing in
    /// it, which is the same class of mistake as an empty library reading as a
    /// broken one.
    pub tracks: Option<Vec<Track>>,
}

impl Album {
    /// Sanitise and narrow a wire `SubAlbum`. **The boundary.**
    ///
    /// `None` for an album whose id is unusable — see [`safe_id`].
    fn from_sub(album: &SubAlbum) -> Option<Self> {
        Some(Self {
            id: safe_id(&album.id)?,
            name: or_placeholder(&album.name, UNKNOWN_ALBUM),
            artist: or_placeholder(&album.artist, UNKNOWN_ARTIST),
            song_count: album.song_count,
            tracks: None,
        })
    }

    /// How many tracks to claim in a list row: the real count once they are
    /// loaded, the server's advertised count until then, and nothing at all when
    /// neither is known.
    #[must_use]
    pub fn track_count(&self) -> Option<usize> {
        match &self.tracks {
            Some(tracks) => Some(tracks.len()),
            None => self.song_count.map(|n| n as usize),
        }
    }
}

/// Tidy `value`, and fall back to `placeholder` when nothing readable is left.
///
/// The fallback runs **after** `tidy`, not before: a name made entirely of
/// control characters is not an empty string on the wire, but it is one on
/// screen, and a blank row is a row a user cannot aim at.
fn or_placeholder(value: &str, placeholder: &str) -> String {
    let tidied = server::tidy(value);
    if tidied.is_empty() {
        placeholder.to_string()
    } else {
        tidied
    }
}

/// An id this client is willing to hold, **byte for byte**, or `None`.
///
/// Ids are the one server-supplied string that must survive unmodified: it goes
/// straight back to the server on the next request, so tidying one would quietly
/// break the album or track it names. Sanitising it is therefore not an option —
/// and holding a raw escape sequence in the model is not one either, because
/// nothing downstream can then assume its strings are safe.
///
/// So the third option: an id that `tidy` would have had to change is not a
/// legitimate id, and the record carrying it is **dropped**. That is already
/// `eko-net`'s policy for an id that is absent or empty (`lenient::id` rejects
/// the record rather than failing the whole page), and it means every `id` in
/// this module is simultaneously safe to render and exact to send.
fn safe_id(raw: &str) -> Option<String> {
    if raw.is_empty() || server::tidy(raw) != raw {
        return None;
    }
    Some(raw.to_string())
}

/// The server's albums, in the order the server sent them.
///
/// Deliberately **not** re-sorted. `getAlbumList2` is asked for
/// `alphabeticalByArtist` and the server's collation is the one its own web UI
/// shows; re-sorting here with [`crate::library`]'s `locale_cmp` would make the
/// same library read differently in two of EKO's own front ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub albums: Vec<Album>,
}

impl Library {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.albums.is_empty()
    }

    /// The album at `index`, if it is in range.
    #[must_use]
    pub fn album(&self, index: usize) -> Option<&Album> {
        self.albums.get(index)
    }

    /// Attach a fetched track list to the album with this id.
    ///
    /// Returns whether it landed. The id is the **requested** one, never the
    /// server's echo of it — see [`fetch_tracks`].
    pub fn set_tracks(&mut self, album_id: &str, tracks: Vec<Track>) -> bool {
        match self.albums.iter_mut().find(|a| a.id == album_id) {
            Some(album) => {
                album.tracks = Some(tracks);
                true
            }
            None => false,
        }
    }
}

// ── Progress ─────────────────────────────────────────────────────────────────

/// What a running remote fetch reports back to the fold.
///
/// The twin of [`crate::library::ScanEvent`], travelling the same channel.
#[derive(Debug, Clone, PartialEq)]
pub enum RemoteKind {
    /// One page of albums, to be appended. Pages arrive as they are read, so a
    /// large library fills in rather than appearing all at once at the end.
    Page(Vec<Album>),
    /// The walk finished. The library may legitimately be empty.
    AlbumsReady,
    /// The walk stopped early. Whatever pages already arrived stand; the Deck
    /// says both. `message` is safe to render — it comes from
    /// [`crate::server::describe`], which is **this client's** vocabulary and
    /// never the far end's own sentence. Nothing on this path holds the
    /// password, so nothing on this path could redact one out of a message the
    /// server wrote.
    AlbumsFailed(String),
    /// One album's tracks. `album_id` is the id that was **asked for**.
    Tracks {
        album_id: String,
        tracks: Vec<Track>,
    },
    /// That album's tracks could not be fetched.
    TracksFailed { album_id: String, message: String },
}

/// A [`RemoteKind`] stamped with the fetch it belongs to.
///
/// `r` retries, and a retry can land while the previous walk still has pages in
/// flight; without this they would be appended to the new library and every
/// album would appear twice. The fold bumps its generation before it starts a
/// walk and drops anything stamped with an older one — the same job
/// `useSubsonic.ts`'s `loadGen` does in the desktop app.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteEvent {
    pub generation: u64,
    pub kind: RemoteKind,
}

// ── The walk ─────────────────────────────────────────────────────────────────

/// Page through `getAlbumList2` until the server runs out, emitting each page.
///
/// **Blocking.** Never call it from the render thread; [`spawn_albums`] is the
/// way in. Separated from the spawn so a test can drive it synchronously against
/// a mock server with a recording `emit`.
///
/// `emit` returns `false` once the fold has hung up, at which point the walk
/// abandons itself rather than fetching pages nobody will read.
///
/// # Termination
///
/// **An empty page, and nothing else.** A short page is not the end of the
/// library — see the module docs, and `albumlist2_capped.json` in `eko-net` for
/// the shape that proves it. `offset` advances by the page's actual length, so a
/// server that caps `size` at 2 is walked two albums at a time instead of being
/// mistaken for a two-album library.
///
/// One residual, inherited rather than introduced: "empty" is measured on the
/// page **`eko-net` parsed**, and `eko_net::parse::album_list` drops records it
/// cannot read rather than failing the page. A page in which *every* record is
/// unreadable therefore arrives here as empty and ends the walk early. That
/// needs a malformed server rather than a merely eccentric one, and fixing it
/// means `eko-net` reporting how many records it dropped — a change to a crate
/// two clients depend on, so it is written down rather than made here.
pub fn walk_albums<F>(client: &Client, emit: &F) -> RemoteKind
where
    F: Fn(RemoteKind) -> bool,
{
    let mut offset: u32 = 0;
    let mut seen: usize = 0;
    loop {
        let page = match client.get_albums(Some(PAGE_SIZE), Some(offset)) {
            Ok(page) => page,
            Err(e) => return RemoteKind::AlbumsFailed(server::describe(&e)),
        };
        // ── THE TERMINATION RULE ──────────────────────────────────────────
        // Only an empty page ends the walk. `page.len() < PAGE_SIZE` would
        // stop here against a capping server and lose the rest of the library.
        if page.is_empty() {
            return RemoteKind::AlbumsReady;
        }
        // `got` is the page as the server counted it, and it is what `offset`
        // advances by — **not** `albums.len()`. A record dropped by `safe_id`
        // still occupies a slot on the server, so advancing by the kept count
        // would ask for it again and walk the same window forever.
        let got = page.len();
        seen += got;
        let albums: Vec<Album> = page.iter().filter_map(Album::from_sub).collect();
        if !emit(RemoteKind::Page(albums)) {
            return RemoteKind::AlbumsReady;
        }
        // The page's ACTUAL length, not PAGE_SIZE: against a capping server the
        // latter would skip everything it did not send.
        offset = offset.saturating_add(u32::try_from(got).unwrap_or(u32::MAX));
        if seen >= MAX_ALBUMS {
            return RemoteKind::AlbumsReady;
        }
    }
}

/// One album's tracks, or a message safe to render. **Blocking.**
///
/// The narrow half of [`fetch_tracks`], split out because the search pane fetches
/// tracks for a result album through the same request and the same
/// [`Track::from_sub`] gate — it only needs to stamp the answer with a different
/// event type.
fn tracks_of(client: &Client, album_id: &str) -> Result<Vec<Track>, String> {
    match client.get_album(album_id) {
        Ok(detail) => Ok(detail.songs.iter().filter_map(Track::from_sub).collect()),
        Err(e) => Err(server::describe(&e)),
    }
}

/// Fetch one album's tracks. **Blocking**; see [`spawn_tracks`].
///
/// The returned `album_id` is the one that was **asked for**, not the one the
/// server echoed back in the payload. A server that answered `getAlbum?id=A`
/// with an album claiming to be `B` would otherwise be able to drop its track
/// list into a different album's row.
#[must_use]
pub fn fetch_tracks(client: &Client, album_id: &str) -> RemoteKind {
    match tracks_of(client, album_id) {
        Ok(tracks) => RemoteKind::Tracks {
            album_id: album_id.to_string(),
            tracks,
        },
        Err(message) => RemoteKind::TracksFailed {
            album_id: album_id.to_string(),
            message,
        },
    }
}

/// Walk the album list on a worker thread, reporting through `emit`.
///
/// Mirrors [`crate::library::spawn`] exactly, down to the detached thread: it
/// owns nothing that needs dropping in order, and the process exits when the
/// fold does.
pub fn spawn_albums<F>(client: Arc<Client>, generation: u64, emit: F)
where
    F: Fn(RemoteEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let kind = walk_albums(&client, &|kind| emit(RemoteEvent { generation, kind }));
        emit(RemoteEvent { generation, kind });
    });
}

/// Fetch one album's tracks on a worker thread.
pub fn spawn_tracks<F>(client: Arc<Client>, generation: u64, album_id: String, emit: F)
where
    F: Fn(RemoteEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        emit(RemoteEvent {
            generation,
            kind: fetch_tracks(&client, &album_id),
        });
    });
}

// ── Search ───────────────────────────────────────────────────────────────────

/// What `search3` matched: some albums, and some songs.
///
/// Both halves are [`Album`] and [`Track`] — the *same* types the album walk
/// produces, built by the *same* private constructors, so every string in here
/// has been through [`crate::server::tidy`] before this struct exists. There is
/// deliberately no second path: a search-only result type with its own fields
/// would be a second boundary to remember, and the one thing this module is for
/// is that there is only one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Results {
    pub albums: Vec<Album>,
    pub songs: Vec<Track>,
}

impl Results {
    /// How many rows a flat list of these has: every album, then every song.
    #[must_use]
    pub fn len(&self) -> usize {
        self.albums.len() + self.songs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.albums.is_empty() && self.songs.is_empty()
    }
}

/// What a running search reports back to the fold.
///
/// Its own type rather than more [`RemoteKind`] variants, because it is stamped
/// against its own counter. The album walk's generation is bumped by `r` and by a
/// dropped connection; a search's is bumped by *every query*, and folding the two
/// into one number would mean either a new query cancelling the library walk or a
/// reconnect being unable to cancel a search.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchKind {
    /// The query answered. It may legitimately have matched nothing.
    Results(Box<Results>),
    /// The query did not answer. `message` comes from [`crate::server::describe`]
    /// and is safe to render.
    Failed(String),
    /// One *result* album's tracks, for the id that was asked for.
    Tracks {
        album_id: String,
        tracks: Vec<Track>,
    },
    TracksFailed {
        album_id: String,
        message: String,
    },
}

/// A [`SearchKind`] stamped with the query it belongs to.
///
/// **This is what makes a slow search safe.** Typing a second query bumps the
/// fold's counter before the request leaves; the first server's answer then
/// arrives stamped with a number the fold no longer recognises and is dropped,
/// rather than replacing the newer results with older ones. Exactly the job
/// [`RemoteEvent::generation`] does for the album walk.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchEvent {
    pub generation: u64,
    pub kind: SearchKind,
}

/// Run one `search3`. **Blocking** — `eko_net::Client` is `reqwest::blocking`,
/// so this is never called from the render thread; [`spawn_search`] is the way
/// in.
#[must_use]
pub fn fetch_search(client: &Client, query: &str) -> SearchKind {
    match client.search(query) {
        Ok(found) => SearchKind::Results(Box::new(Results {
            // The same two constructors the album walk uses, so results are
            // sanitised by construction rather than by anyone remembering to.
            albums: found.albums.iter().filter_map(Album::from_sub).collect(),
            songs: found.songs.iter().filter_map(Track::from_sub).collect(),
        })),
        Err(e) => SearchKind::Failed(server::describe(&e)),
    }
}

/// Search on a worker thread. The twin of [`spawn_albums`].
pub fn spawn_search<F>(client: Arc<Client>, generation: u64, query: String, emit: F)
where
    F: Fn(SearchEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        emit(SearchEvent {
            generation,
            kind: fetch_search(&client, &query),
        });
    });
}

/// Fetch a *result* album's tracks on a worker thread.
///
/// The same request and the same gate as [`spawn_tracks`]; only the envelope
/// differs, so a track list cannot land in the search pane under the album
/// walk's generation or the other way round.
pub fn spawn_search_tracks<F>(client: Arc<Client>, generation: u64, album_id: String, emit: F)
where
    F: Fn(SearchEvent) -> bool + Send + 'static,
{
    std::thread::spawn(move || {
        let kind = match tracks_of(&client, &album_id) {
            Ok(tracks) => SearchKind::Tracks { album_id, tracks },
            Err(message) => SearchKind::TracksFailed { album_id, message },
        };
        emit(SearchEvent { generation, kind });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eko_net::Config;
    use mockito::{Matcher, Server, ServerGuard};
    use std::sync::Mutex;

    // ── mock server plumbing ─────────────────────────────────────────────

    fn client_for(server: &ServerGuard) -> Client {
        Client::new(Config {
            base_url: server.url(),
            username: "rod".into(),
            password: "hunter2".into(),
        })
        .expect("an http client")
    }

    /// One album's JSON, as Navidrome writes it. `name` and `artist` must be
    /// plain — nothing here escapes them, on purpose: the hostile-input cases
    /// below write their own JSON by hand so the escaping is visible.
    fn album_json(id: usize, name: &str, artist: &str, songs: u32) -> String {
        format!(r#"{{"id":"al-{id:04}","name":"{name}","artist":"{artist}","songCount":{songs}}}"#)
    }

    /// A `getAlbumList2` page holding `ids`.
    fn page_json(ids: std::ops::Range<usize>) -> String {
        let albums: Vec<String> = ids
            .map(|i| album_json(i, &format!("Album {i}"), "Test Artist", 4))
            .collect();
        format!(
            r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","albumList2":{{"album":[{}]}}}}}}"#,
            albums.join(",")
        )
    }

    /// The empty page every walk ends on — `albumList2` with no `album` key,
    /// which is exactly what `albumlist2_empty.json` pins in `eko-net`.
    const EMPTY_PAGE: &str =
        r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{}}}"#;

    /// Mock `getAlbumList2` for one specific `offset`.
    ///
    /// The offset is the **last** query param `eko-net` appends, so anchoring on
    /// it is exact: `offset=0` cannot also match the request for `offset=500`.
    fn mock_page(server: &mut ServerGuard, offset: u32, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/rest/getAlbumList2")
            .match_query(Matcher::Regex(format!(r"size=500&offset={offset}$")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    fn mock_album(server: &mut ServerGuard, id: &str, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/rest/getAlbum")
            .match_query(Matcher::Regex(format!(r"id={id}$")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    /// Drive [`walk_albums`] to completion, collecting every page it emitted.
    fn walk(server: &ServerGuard) -> (Vec<Album>, RemoteKind) {
        let collected = Mutex::new(Vec::new());
        let outcome = walk_albums(&client_for(server), &|kind| {
            if let RemoteKind::Page(albums) = kind {
                collected.lock().unwrap().extend(albums);
            }
            true
        });
        (collected.into_inner().unwrap(), outcome)
    }

    // ── paging ───────────────────────────────────────────────────────────

    /// A library bigger than one page is walked to the end.
    #[test]
    fn a_multi_page_library_arrives_whole() {
        let mut server = Server::new();
        let _p0 = mock_page(&mut server, 0, &page_json(0..500));
        let _p1 = mock_page(&mut server, 500, &page_json(500..503));
        let _end = mock_page(&mut server, 503, EMPTY_PAGE);

        let (albums, outcome) = walk(&server);
        assert_eq!(outcome, RemoteKind::AlbumsReady);
        assert_eq!(albums.len(), 503, "the second page was dropped");
        assert_eq!(albums[0].name, "Album 0");
        assert_eq!(albums[502].name, "Album 502");
    }

    /// **The truncation bug, directly.**
    ///
    /// This server was asked for 500 albums a page and answers with 2, every
    /// time — the shape `eko-net`'s `albumlist2_capped.json` exists to pin. A
    /// walk that stopped on `page.len() < PAGE_SIZE` would take the first short
    /// page as the end of the library and report 2 albums out of 7.
    ///
    /// Mutation-tested, both halves of the rule:
    ///
    /// * terminating on `page.len() < PAGE_SIZE as usize` reports **0** albums —
    ///   worse than the 2 you would expect, because the check fires before the
    ///   short page is even emitted;
    /// * advancing by `PAGE_SIZE` rather than the page's actual length asks for
    ///   `offset=500` next, which no mock answers, and the walk ends in
    ///   `AlbumsFailed`.
    ///
    /// Both mutations fail here.
    #[test]
    fn a_server_that_caps_the_page_size_is_walked_to_the_end_not_truncated() {
        let mut server = Server::new();
        // Seven albums, handed over two at a time whatever we ask for.
        let _p0 = mock_page(&mut server, 0, &page_json(0..2));
        let _p1 = mock_page(&mut server, 2, &page_json(2..4));
        let _p2 = mock_page(&mut server, 4, &page_json(4..6));
        let _p3 = mock_page(&mut server, 6, &page_json(6..7));
        let _end = mock_page(&mut server, 7, EMPTY_PAGE);

        let (albums, outcome) = walk(&server);
        assert_eq!(outcome, RemoteKind::AlbumsReady);
        assert_eq!(
            albums.len(),
            7,
            "a capped page size truncated the library — the exact bug at \
             src/subsonic/useSubsonic.ts:185"
        );
        let names: Vec<&str> = albums.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Album 0", "Album 1", "Album 2", "Album 3", "Album 4", "Album 5", "Album 6"]
        );
    }

    /// A server with nothing on it answers one empty page, and that is a normal
    /// finished walk rather than a failure.
    #[test]
    fn an_empty_library_is_one_request_and_a_normal_finish() {
        let mut server = Server::new();
        let end = mock_page(&mut server, 0, EMPTY_PAGE);

        let (albums, outcome) = walk(&server);
        end.assert();
        assert!(albums.is_empty());
        assert_eq!(outcome, RemoteKind::AlbumsReady);
    }

    /// A walk that loses the server keeps the pages it already had, and says so
    /// separately.
    #[test]
    fn a_walk_that_fails_midway_reports_the_failure_and_keeps_its_pages() {
        let mut server = Server::new();
        let _p0 = mock_page(&mut server, 0, &page_json(0..2));
        let _p1 = server
            .mock("GET", "/rest/getAlbumList2")
            .match_query(Matcher::Regex(r"size=500&offset=2$".into()))
            .with_status(502)
            .with_body("bad gateway")
            .create();

        let (albums, outcome) = walk(&server);
        assert_eq!(albums.len(), 2, "the first page was thrown away");
        assert_eq!(
            outcome,
            RemoteKind::AlbumsFailed("the server answered HTTP 502".into())
        );
    }

    /// A fold that has hung up stops the walk instead of fetching pages nobody
    /// will read.
    #[test]
    fn a_hung_up_fold_abandons_the_walk() {
        let mut server = Server::new();
        let first = mock_page(&mut server, 0, &page_json(0..2)).expect_at_most(1);
        let never = mock_page(&mut server, 2, &page_json(2..4)).expect(0);

        let outcome = walk_albums(&client_for(&server), &|_| false);
        assert_eq!(outcome, RemoteKind::AlbumsReady);
        first.assert();
        never.assert();
    }

    /// Pages are emitted as they arrive, not banked until the end — a large
    /// library has to fill in.
    #[test]
    fn pages_are_emitted_as_they_arrive() {
        let mut server = Server::new();
        let _p0 = mock_page(&mut server, 0, &page_json(0..2));
        let _p1 = mock_page(&mut server, 2, &page_json(2..4));
        let _end = mock_page(&mut server, 4, EMPTY_PAGE);

        let sizes = Mutex::new(Vec::new());
        walk_albums(&client_for(&server), &|kind| {
            if let RemoteKind::Page(albums) = kind {
                sizes.lock().unwrap().push(albums.len());
            }
            true
        });
        assert_eq!(
            sizes.into_inner().unwrap(),
            vec![2, 2],
            "the walk banked its pages instead of streaming them"
        );
    }

    // ── album detail ─────────────────────────────────────────────────────

    #[test]
    fn an_albums_tracks_arrive_narrowed_and_in_order() {
        let mut server = Server::new();
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al-0001","name":"Blue","artist":"Joni Mitchell","song":[
            {"id":"tr-1","title":"All I Want","artist":"Joni Mitchell","track":1,"duration":212},
            {"id":"tr-2","title":"My Old Man","artist":"Joni Mitchell","track":2,"duration":213}
        ]}}}"#;
        let mock = mock_album(&mut server, "al-0001", body);

        let kind = fetch_tracks(&client_for(&server), "al-0001");
        mock.assert();
        match kind {
            RemoteKind::Tracks { album_id, tracks } => {
                assert_eq!(album_id, "al-0001");
                assert_eq!(tracks.len(), 2);
                assert_eq!(tracks[0].title, "All I Want");
                assert_eq!(tracks[0].track_no, Some(1));
                assert!((tracks[0].duration - 212.0).abs() < f64::EPSILON);
            }
            other => panic!("expected tracks, got {other:?}"),
        }
    }

    /// **An album with no tracks is an answer, not a failure.**
    ///
    /// `Some(vec![])` and `None` are different states — see [`Album::tracks`] —
    /// so a genuinely empty album can be told apart from one still loading.
    #[test]
    fn an_album_with_no_tracks_reports_an_empty_track_list() {
        let mut server = Server::new();
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al-0009","name":"Nothing","artist":"Nobody"}}}"#;
        let mock = mock_album(&mut server, "al-0009", body);

        let kind = fetch_tracks(&client_for(&server), "al-0009");
        mock.assert();
        assert_eq!(
            kind,
            RemoteKind::Tracks {
                album_id: "al-0009".into(),
                tracks: Vec::new(),
            }
        );

        // …and in the model the two states stay distinct.
        let mut library = Library {
            albums: vec![Album {
                id: "al-0009".into(),
                name: "Nothing".into(),
                artist: "Nobody".into(),
                song_count: Some(0),
                tracks: None,
            }],
        };
        assert_eq!(library.albums[0].tracks, None, "not asked yet");
        assert!(library.set_tracks("al-0009", Vec::new()));
        assert_eq!(
            library.albums[0].tracks,
            Some(Vec::new()),
            "asked, and empty"
        );
    }

    /// A failed detail fetch names the album it was for, so it cannot be shown
    /// against a different one.
    #[test]
    fn a_failed_detail_fetch_carries_the_album_it_was_for() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", "/rest/getAlbum")
            .match_query(Matcher::Regex(r"id=al-0001$".into()))
            .with_status(500)
            .with_body("boom")
            .create();

        let kind = fetch_tracks(&client_for(&server), "al-0001");
        mock.assert();
        assert_eq!(
            kind,
            RemoteKind::TracksFailed {
                album_id: "al-0001".into(),
                message: "the server answered HTTP 500".into(),
            }
        );
    }

    /// **The id the caller asked for wins over the one the server echoed.**
    ///
    /// Otherwise a server could answer `getAlbum?id=A` with an album calling
    /// itself `B` and drop its track list into a different row.
    #[test]
    fn the_requested_album_id_is_the_one_reported_not_the_servers_echo() {
        let mut server = Server::new();
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"SOMETHING-ELSE","name":"Blue","artist":"Joni Mitchell","song":[{"id":"tr-1","title":"All I Want"}]}}}"#;
        let _m = mock_album(&mut server, "al-0001", body);

        match fetch_tracks(&client_for(&server), "al-0001") {
            RemoteKind::Tracks { album_id, .. } => assert_eq!(album_id, "al-0001"),
            other => panic!("expected tracks, got {other:?}"),
        }
    }

    // ── the server writes this text ──────────────────────────────────────

    /// **A hostile album name cannot reach the terminal.**
    ///
    /// The album here is named with a raw `ESC [ 2 J` (clear screen) followed by
    /// `ESC [ 31 m` (paint the rest of the row red), a newline, and a `U+202E`
    /// right-to-left override; its artist carries an `ESC ] 0 ;` OSC (retitle the
    /// terminal window) terminated by `BEL`. Rendered as-is into a `ratatui`
    /// `Line` every one of those reaches the terminal verbatim — the OSC and the
    /// clear-screen would take the whole Deck, and the SGR would repaint the row
    /// the signal-path seal lives on.
    ///
    /// None of them may survive into an [`Album`], which is the only thing the
    /// widgets ever see. They are written as JSON `\u` escapes below because a
    /// raw control character is not legal JSON — which is itself worth knowing:
    /// the attack has to arrive escaped, and `serde_json` turns it into a real
    /// `ESC` before anything in this crate sees it.
    #[test]
    fn a_hostile_album_name_cannot_carry_an_escape_into_the_terminal() {
        let mut server = Server::new();
        let nasty = concat!(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":[{"#,
            r#""id":"al-0001","#,
            r#""name":"\u001b[2J\u001b[31mRED\nnext\u202e line","#,
            r#""artist":"Evil\u001b]0;pwned\u0007Corp","#,
            r#""songCount":1}]}}}"#
        );
        let _p0 = mock_page(&mut server, 0, nasty);
        // One album on that page, so the walk asks for offset=1 next.
        let _end = mock_page(&mut server, 1, EMPTY_PAGE);

        let (albums, _) = walk(&server);
        assert_eq!(albums.len(), 1);
        let album = &albums[0];
        for (what, text) in [
            ("id", &album.id),
            ("name", &album.name),
            ("artist", &album.artist),
        ] {
            assert!(
                !text.chars().any(char::is_control),
                "{what} carried a control character into the UI: {text:?}"
            );
            assert!(
                !text.contains(['\u{202e}', '\u{2028}', '\u{2029}']),
                "{what} carried a bidi override or line separator: {text:?}"
            );
        }
        // What survives is inert text: the escape *introducers* are gone, so the
        // bracket sequences left behind are just characters.
        assert_eq!(album.name, "[2J [31mRED next line");
        assert_eq!(album.artist, "Evil ]0;pwned Corp");
    }

    /// The same gate on track titles, through the detail fetch.
    #[test]
    fn a_hostile_track_title_cannot_carry_an_escape_into_the_terminal() {
        let mut server = Server::new();
        let body = concat!(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al-1","name":"X","artist":"Y","song":[{"#,
            r#""id":"tr-1","title":"\u001b[2J\u001b[Hgotcha","artist":"\u202ehctal","track":1,"duration":10}]}}}"#
        );
        let _m = mock_album(&mut server, "al-1", body);

        match fetch_tracks(&client_for(&server), "al-1") {
            RemoteKind::Tracks { tracks, .. } => {
                assert_eq!(tracks[0].title, "[2J [Hgotcha");
                assert_eq!(tracks[0].artist, "hctal");
                assert!(!tracks[0].title.chars().any(char::is_control));
                assert!(!tracks[0].artist.contains('\u{202e}'));
            }
            other => panic!("expected tracks, got {other:?}"),
        }
    }

    // ── search ───────────────────────────────────────────────────────────

    /// Mock `search3` for one specific query.
    ///
    /// `query` is the **first** parameter `eko-net` appends and the counts follow
    /// it, so anchoring on `query=…&songCount` is exact.
    fn mock_search(server: &mut ServerGuard, query: &str, body: &str) -> mockito::Mock {
        server
            .mock("GET", "/rest/search3")
            .match_query(Matcher::Regex(format!(r"query={query}&songCount")))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create()
    }

    /// A `search3` payload, with both collections written out by hand so the
    /// escaping in the hostile case below is visible rather than generated.
    fn search_json(albums: &[&str], songs: &[&str]) -> String {
        format!(
            r#"{{"subsonic-response":{{"status":"ok","version":"1.16.1","searchResult3":{{"album":[{}],"song":[{}]}}}}}}"#,
            albums.join(","),
            songs.join(",")
        )
    }

    /// **Both halves come back, narrowed, in the order the server ranked them.**
    #[test]
    fn a_search_returns_albums_and_songs_together() {
        let mut server = Server::new();
        let body = search_json(
            &[
                r#"{"id":"al-7","name":"Spirit of Eden","artist":"Talk Talk","songCount":6}"#,
                r#"{"id":"al-8","name":"Laughing Stock","artist":"Talk Talk","songCount":6}"#,
            ],
            &[
                r#"{"id":"tr-1","title":"Eden","artist":"Talk Talk","album":"Spirit of Eden","track":2,"duration":386}"#,
                r#"{"id":"tr-2","title":"Desire","artist":"Talk Talk","album":"Spirit of Eden","track":3,"duration":418}"#,
            ],
        );
        let mock = mock_search(&mut server, "eden", &body);

        let kind = fetch_search(&client_for(&server), "eden");
        mock.assert();
        let SearchKind::Results(results) = kind else {
            panic!("expected results, got {kind:?}");
        };
        assert_eq!(results.albums.len(), 2);
        assert_eq!(results.songs.len(), 2);
        assert_eq!(results.len(), 4, "the flat row count is albums, then songs");
        assert_eq!(results.albums[0].name, "Spirit of Eden");
        assert_eq!(results.albums[0].song_count, Some(6));
        // Not fetched yet — the same distinction the album walk keeps, so an
        // unopened result album cannot look like an empty one.
        assert_eq!(results.albums[0].tracks, None);
        assert_eq!(results.songs[0].title, "Eden");
        assert_eq!(results.songs[0].track_no, Some(2));
        // The album a matched song came from: the one thing a song outside an
        // album list cannot otherwise say, and the footer's second line.
        assert_eq!(results.songs[0].album, "Spirit of Eden");
        assert!((results.songs[1].duration - 418.0).abs() < f64::EPSILON);
    }

    /// **Nothing matched is an answer, not a failure.**
    ///
    /// `searchResult3` with neither key is what a server sends for a query it has
    /// nothing for, and `eko-net`'s `collection` reads that as two empty vecs. The
    /// pane then draws "nothing matched", which is a different screen from "the
    /// search broke" — see `crate::ui::main_pane`.
    #[test]
    fn a_search_that_matches_nothing_is_an_empty_result_not_an_error() {
        let mut server = Server::new();
        let body = r#"{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{}}}"#;
        let mock = mock_search(&mut server, "zzzzz", body);

        let kind = fetch_search(&client_for(&server), "zzzzz");
        mock.assert();
        assert_eq!(
            kind,
            SearchKind::Results(Box::default()),
            "an empty answer was reported as something other than empty"
        );
    }

    /// A search that cannot reach the server says so, in words that carry no
    /// credential: [`server::describe`] is the same gate the album walk uses.
    #[test]
    fn a_search_that_fails_carries_the_reason_and_nothing_else() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", "/rest/search3")
            .match_query(Matcher::Any)
            .with_status(503)
            .with_body("down")
            .create();

        let kind = fetch_search(&client_for(&server), "anything");
        mock.assert();
        assert_eq!(
            kind,
            SearchKind::Failed("the server answered HTTP 503".into())
        );
    }

    /// **A hostile search result cannot carry an escape into the terminal.**
    ///
    /// This is the point of the whole shape: a result is an [`Album`] and a
    /// [`Track`], built by the same two private constructors the album walk uses,
    /// so every string is through [`crate::server::tidy`] before a [`Results`]
    /// exists at all. There is no second path for a new gate to be forgotten on.
    ///
    /// The album clears the screen and repaints the row the signal-path seal lives
    /// on. The song retitles the terminal window with an OSC and reverses the rest
    /// of its artist with a `U+202E`. The second song's **id** carries an escape,
    /// which cannot be tidied — an id goes back to the server byte for byte — so
    /// it takes its whole record with it.
    #[test]
    fn a_hostile_search_result_cannot_carry_an_escape_into_the_terminal() {
        let mut server = Server::new();
        let body = search_json(
            &[
                r#"{"id":"al-9","name":"\u001b[2J\u001b[31mRED","artist":"Evil\u001b]0;pwned\u0007Corp","songCount":1}"#,
            ],
            &[
                r#"{"id":"tr-9","title":"\u001b[2J\u001b[Hgotcha","artist":"\u202ehctal","album":"\u001b[5mBlink","duration":10}"#,
                r#"{"id":"tr-\u001b[2J","title":"Dropped","artist":"X","album":"Y"}"#,
            ],
        );
        let _m = mock_search(&mut server, "evil", &body);

        let SearchKind::Results(results) = fetch_search(&client_for(&server), "evil") else {
            panic!("expected results");
        };
        assert_eq!(results.albums.len(), 1);
        assert_eq!(
            results.songs.len(),
            1,
            "the record whose id could not be sanitised was not dropped"
        );

        let album = &results.albums[0];
        let song = &results.songs[0];
        for (what, text) in [
            ("album id", &album.id),
            ("album name", &album.name),
            ("album artist", &album.artist),
            ("song id", &song.id),
            ("song title", &song.title),
            ("song artist", &song.artist),
            ("song album", &song.album),
        ] {
            assert!(
                !text.chars().any(char::is_control),
                "{what} carried a control character into the UI: {text:?}"
            );
            assert!(
                !text.contains(['\u{202e}', '\u{2028}', '\u{2029}']),
                "{what} carried a bidi override or line separator: {text:?}"
            );
        }
        // What survives is inert text: the escape introducers are gone, so the
        // bracket sequences left behind are just characters.
        assert_eq!(album.name, "[2J [31mRED");
        assert_eq!(album.artist, "Evil ]0;pwned Corp");
        assert_eq!(song.title, "[2J [Hgotcha");
        assert_eq!(song.artist, "hctal");
        assert_eq!(song.album, "[5mBlink");
    }

    /// A *result* album's tracks come back through the same `getAlbum` and the
    /// same gate as a browsed album's — only the envelope differs.
    #[test]
    fn a_result_albums_tracks_are_fetched_through_the_same_gate() {
        let mut server = Server::new();
        let body = concat!(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","album":{"id":"al-7","name":"Spirit of Eden","artist":"Talk Talk","song":[{"#,
            r#""id":"tr-1","title":"\u001b[2JThe Rainbow","artist":"Talk Talk","album":"Spirit of Eden","track":1,"duration":555}]}}}"#
        );
        let _m = mock_album(&mut server, "al-7", body);

        let tracks = tracks_of(&client_for(&server), "al-7").expect("tracks");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "[2JThe Rainbow");
        assert!(!tracks[0].title.chars().any(char::is_control));
    }

    /// **An id that would have to be sanitised is not an id.**
    ///
    /// It cannot be tidied — it goes straight back to the server on the next
    /// request — and it cannot be held raw either, or nothing downstream may
    /// assume its strings are safe. So the record is dropped, its siblings
    /// survive, and the walk still advances by the page's *server-side* length.
    #[test]
    fn a_record_whose_id_carries_an_escape_is_dropped_rather_than_sanitised() {
        let mut server = Server::new();
        let page = concat!(
            r#"{"subsonic-response":{"status":"ok","version":"1.16.1","albumList2":{"album":["#,
            r#"{"id":"al-\u001b[2J","name":"Hostile","artist":"X","songCount":1},"#,
            r#"{"id":"al-0002","name":"Fine","artist":"X","songCount":1}"#,
            r#"]}}}"#
        );
        let _p0 = mock_page(&mut server, 0, page);
        // TWO albums were sent, so the next offset is 2 — not 1, which is how
        // many survived the drop.
        let _end = mock_page(&mut server, 2, EMPTY_PAGE);

        let (albums, outcome) = walk(&server);
        assert_eq!(outcome, RemoteKind::AlbumsReady);
        assert_eq!(albums.len(), 1, "the hostile record was not dropped");
        assert_eq!(albums[0].id, "al-0002");
        assert_eq!(albums[0].name, "Fine");
    }

    /// A legitimate id is held **byte for byte**, because it has to go back to
    /// the server exactly as it came.
    #[test]
    fn a_legitimate_id_survives_untouched() {
        for id in ["al-0001", "1234", "AL_00-01.a", "0e7e5b8f-1f6a-4d2c-9e11"] {
            assert_eq!(safe_id(id).as_deref(), Some(id));
        }
        for bad in ["", "al\u{1b}-1", "al\n1", " al-1", "al-1 ", "a\u{202e}b"] {
            assert_eq!(safe_id(bad), None, "{bad:?} was accepted as an id");
        }
    }

    /// **A server's own prose never reaches the frame on a request path.**
    ///
    /// `SubsonicError::Subsonic` is the far end's sentence, chosen by the far
    /// end — Subsonic reports failure in the *body*, HTTP 200 with
    /// `status: "failed"`, so any request can come back this way, not just a
    /// login. `crate::server::connect_with` is the only place in the crate that
    /// holds the password and the message at the same time, and therefore the
    /// only place that can strike the credential out of one. These three paths
    /// have no secret to compare against and never will.
    ///
    /// So they do not render it at all. The sentence below is what a hostile —
    /// or merely over-helpful — server would say if it echoed the credential
    /// back on an album walk, a track fetch or a search; all three answer in
    /// this client's own words instead, and `hunter2` is on none of them.
    ///
    /// Its sibling is `server.rs`'s
    /// `a_server_that_echoes_the_password_back_cannot_put_it_in_the_footer`,
    /// which covers the login path — where the sentence *is* rendered, and the
    /// redaction is what keeps it safe.
    #[test]
    fn a_servers_own_sentence_never_reaches_an_album_walk_a_track_fetch_or_a_search() {
        // The credential, handed back by a server that "helpfully" quotes the
        // request it refused.
        const ECHOED: &str = r#"{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password: u=rod p=hunter2"}}}"#;

        let mut server = Server::new();
        let _p0 = mock_page(&mut server, 0, ECHOED);
        let _al = mock_album(&mut server, "al-1", ECHOED);
        let _se = mock_search(&mut server, "anything", ECHOED);
        let client = client_for(&server);

        let walked = walk_albums(&client, &|_| true);
        let fetched = tracks_of(&client, "al-1").expect_err("a failed fetch");
        let searched = fetch_search(&client, "anything");

        let expected = "the server refused the request";
        assert_eq!(walked, RemoteKind::AlbumsFailed(expected.into()));
        assert_eq!(fetched, expected);
        assert_eq!(searched, SearchKind::Failed(expected.into()));

        for (what, message) in [
            ("the album walk", format!("{walked:?}")),
            ("the track fetch", fetched),
            ("the search", format!("{searched:?}")),
        ] {
            for forbidden in ["hunter2", "u=rod", "Wrong username"] {
                assert!(
                    !message.contains(forbidden),
                    "{what} rendered the server's own words: {forbidden:?} in {message}"
                );
            }
        }
    }

    /// A name that is *only* control characters would render as a blank row, so
    /// it gets the same placeholder an untagged one does.
    #[test]
    fn a_name_that_tidies_away_to_nothing_falls_back_to_a_placeholder() {
        assert_eq!(
            or_placeholder("\u{1b}\u{7}\u{202e}", UNKNOWN_ALBUM),
            "Unknown Album"
        );
        assert_eq!(or_placeholder("", UNKNOWN_ARTIST), "Unknown Artist");
        assert_eq!(or_placeholder("   ", UNTITLED), "Untitled");
        assert_eq!(or_placeholder("Real Name", UNKNOWN_ALBUM), "Real Name");
    }

    // ── the model ────────────────────────────────────────────────────────

    #[test]
    fn a_track_count_prefers_the_real_list_over_the_advertised_one() {
        let mut album = Album {
            id: "a".into(),
            name: "n".into(),
            artist: "x".into(),
            song_count: Some(9),
            tracks: None,
        };
        assert_eq!(album.track_count(), Some(9), "the server's count, so far");
        album.tracks = Some(vec![Track {
            id: "t".into(),
            title: "t".into(),
            artist: "x".into(),
            album: "n".into(),
            track_no: Some(1),
            duration: 1.0,
        }]);
        assert_eq!(album.track_count(), Some(1), "what actually arrived");
        album.song_count = None;
        album.tracks = None;
        assert_eq!(
            album.track_count(),
            None,
            "nothing is known, so nothing is claimed"
        );
    }

    #[test]
    fn tracks_land_on_the_album_they_name_and_nowhere_else() {
        let mut library = Library {
            albums: vec![
                Album {
                    id: "a".into(),
                    name: "A".into(),
                    artist: "x".into(),
                    song_count: None,
                    tracks: None,
                },
                Album {
                    id: "b".into(),
                    name: "B".into(),
                    artist: "x".into(),
                    song_count: None,
                    tracks: None,
                },
            ],
        };
        let track = Track {
            id: "t".into(),
            title: "T".into(),
            artist: "x".into(),
            album: "B".into(),
            track_no: None,
            duration: 0.0,
        };
        assert!(library.set_tracks("b", vec![track]));
        assert_eq!(library.albums[0].tracks, None);
        assert_eq!(library.albums[1].tracks.as_ref().unwrap().len(), 1);
        assert!(
            !library.set_tracks("nope", Vec::new()),
            "tracks for an album we do not have must not be invented"
        );
        assert!(library.album(9).is_none());
    }
}
