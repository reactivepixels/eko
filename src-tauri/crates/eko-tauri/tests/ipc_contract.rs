//! IPC contract test — the frontend/backend command-name drift guard.
//!
//! EKO's TypeScript frontend calls into Rust over Tauri IPC **by string name**:
//!
//! ```ts
//! await invoke("engine_seek", { secs });
//! ```
//!
//! In Tauri the Rust function name *is* the command name, so renaming (or
//! dropping from `generate_handler!`) a command while the frontend still calls
//! it produces **no compile error and no test failure** — it breaks only at
//! runtime, in the user's hands. This test replaces the manual cross-check that
//! was performed by hand at every step of the `eko-core` workspace split.
//!
//! What it does:
//!   1. Parses `crates/eko-tauri/src/lib.rs` for the `tauri::generate_handler![…]`
//!      list and extracts the leaf command names
//!      (`commands::engine::engine_seek` → `engine_seek`), **ignoring** the
//!      `#[cfg(feature = "pro")]` attributes interleaved in the list.
//!   2. Walks the repo-root `src/` TypeScript tree for `invoke("name", …)` and
//!      `invoke<T>("name", …)` call sites.
//!   3. Asserts **frontend ⊆ handlers**.
//!
//! Why handlers are collected irrespective of `cfg`: the same assertion then
//! holds in the free build, the Pro build, *and* the public MIT repo (where
//! `src/pro/` is absent and the frontend set simply shrinks). The reverse
//! direction (handlers ⊆ frontend) is deliberately NOT asserted — some commands
//! are legitimately unused by the frontend today.
//!
//! NOTE ON BINARY-LOOKING FILES: `src/local/useLocal.ts` contains a deliberate
//! literal NUL byte (a composite-key separator). Files are therefore read as
//! **bytes** and lossy-converted — never through an API that skips "binary"
//! files, which would silently drop the only call site of `scan_music_folder`.
//! `frontend_scan_finds_the_nul_byte_file` locks that protection in.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Sanity floors so this test can never pass vacuously (a parser that finds
/// nothing would make the subset assertion trivially true).
const MIN_HANDLERS: usize = 40;
/// The public/free tree has no `src/pro`, so its frontend set is smaller than
/// the dev tree's — this floor is set below the free-build count.
const MIN_FRONTEND_INVOKES: usize = 25;

/// The file carrying the literal NUL byte, relative to the repo root.
const NUL_BYTE_FILE: &str = "src/local/useLocal.ts";
/// The command only ever invoked from that file.
const NUL_BYTE_FILE_COMMAND: &str = "scan_music_folder";

// ─────────────────────────────── paths ────────────────────────────────────────

/// Repo root, derived from `CARGO_MANIFEST_DIR` (`<repo>/src-tauri/crates/eko-tauri`).
/// Walks up until it finds a directory holding both `src` and `src-tauri`, so it
/// keeps working if the crate is nested differently.
fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .find(|d| d.join("src").is_dir() && d.join("src-tauri").is_dir())
        .unwrap_or_else(|| {
            panic!(
                "could not locate the repo root above {}",
                manifest.display()
            )
        })
        .to_path_buf()
}

/// Read any file as bytes and lossy-convert. Never skips "binary" content —
/// see the NUL-byte note in the module docs.
fn read_lossy(path: &Path) -> String {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

// ──────────────────────────── handler parsing ─────────────────────────────────

/// Extract the body of `tauri::generate_handler![ … ]` by bracket balancing.
/// The `#[cfg(feature = "pro")]` attributes inside are themselves balanced, so
/// plain depth counting is correct.
fn generate_handler_body(src: &str) -> String {
    let marker = "generate_handler!";
    let start = src
        .find(marker)
        .expect("no `generate_handler!` invocation found in eko-tauri/src/lib.rs");
    let bytes = src.as_bytes();
    let open = start + marker.len();
    let open = (open..bytes.len())
        .find(|&i| bytes[i] == b'[')
        .expect("`generate_handler!` is not followed by `[`");

    let mut depth = 0usize;
    for i in open..bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return src[open + 1..i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced brackets in the `generate_handler!` list");
}

/// Remove `#[ … ]` attribute groups (bracket-balanced) from a snippet.
fn strip_attributes(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'#' && bytes.get(i + 1) == Some(&b'[') {
            let mut depth = 0usize;
            let mut j = i + 1;
            while j < bytes.len() {
                match bytes[j] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// All command names registered with `generate_handler!`, regardless of `cfg`.
fn registered_handlers(repo: &Path) -> BTreeSet<String> {
    let lib = repo.join("src-tauri/crates/eko-tauri/src/lib.rs");
    let body = generate_handler_body(&read_lossy(&lib));
    let body = strip_attributes(&body);

    let mut names = BTreeSet::new();
    for raw in body.split(',') {
        // Drop `//` line comments, then take the leaf path segment.
        let cleaned: String = raw
            .lines()
            .map(|l| l.split("//").next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("");
        let leaf = cleaned.rsplit("::").next().unwrap_or("").trim();
        if is_ident(leaf) {
            names.insert(leaf.to_string());
        }
    }
    names
}

// ─────────────────────────── frontend parsing ─────────────────────────────────

fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name == "node_modules" || name.starts_with('.') {
                continue;
            }
            collect_source_files(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ts" | "tsx" | "js" | "jsx" | "mts" | "cts")
        ) {
            out.push(path);
        }
    }
}

/// Extract the command-name literals from `invoke("name", …)` /
/// `invoke<T>("name", …)` call sites in one file's text.
fn invoke_names_in(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut names = BTreeSet::new();
    let mut search = 0usize;

    while let Some(rel) = text[search..].find("invoke") {
        let at = search + rel;
        search = at + "invoke".len();

        // Must be a whole word (not `reinvoke`, `myInvoke`…).
        if at > 0 {
            let prev = bytes[at - 1] as char;
            if prev.is_ascii_alphanumeric() || prev == '_' || prev == '$' {
                continue;
            }
        }

        let mut i = search;
        let skip_ws = |i: &mut usize| {
            while *i < bytes.len() && (bytes[*i] as char).is_ascii_whitespace() {
                *i += 1;
            }
        };
        skip_ws(&mut i);

        // Optional generic argument list: `invoke<ScannedTrack[]>(…)`.
        if i < bytes.len() && bytes[i] == b'<' {
            let mut depth = 0usize;
            while i < bytes.len() {
                match bytes[i] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    b'(' | b';' | b'\n' => break, // not a generic after all
                    _ => {}
                }
                i += 1;
            }
            skip_ws(&mut i);
        }

        if i >= bytes.len() || bytes[i] != b'(' {
            continue;
        }
        i += 1;
        skip_ws(&mut i);

        let quote = match bytes.get(i) {
            Some(&q @ (b'"' | b'\'' | b'`')) => q,
            // Dynamic command name — nothing to check statically.
            _ => continue,
        };
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i >= bytes.len() {
            continue;
        }
        let literal = &text[start..i];
        if is_ident(literal) {
            names.insert(literal.to_string());
        }
    }
    names
}

/// Every command name the frontend invokes, mapped back to the files that do it.
fn frontend_invokes(repo: &Path) -> Vec<(PathBuf, BTreeSet<String>)> {
    let mut files = Vec::new();
    collect_source_files(&repo.join("src"), &mut files);
    assert!(
        !files.is_empty(),
        "found no TypeScript sources under {} — the frontend scan is broken",
        repo.join("src").display()
    );
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let names = invoke_names_in(&read_lossy(&f));
            (f, names)
        })
        .filter(|(_, names)| !names.is_empty())
        .collect()
}

// ───────────────────────────────── tests ──────────────────────────────────────

#[test]
fn handler_list_parses_to_a_realistic_number_of_commands() {
    let handlers = registered_handlers(&repo_root());
    assert!(
        handlers.len() >= MIN_HANDLERS,
        "only {} handler name(s) parsed out of `generate_handler!` (expected >= {}). \
         The parser is probably broken — a vacuous pass here would make the drift \
         check below meaningless. Parsed: {:?}",
        handlers.len(),
        MIN_HANDLERS,
        handlers
    );
}

#[test]
fn frontend_scan_finds_the_nul_byte_file() {
    // `src/local/useLocal.ts` holds a deliberate literal NUL byte and is the only
    // caller of `scan_music_folder`. If a future refactor swaps the byte-wise read
    // for a "skip binary files" helper, this fails loudly instead of silently
    // shrinking the frontend set.
    let repo = repo_root();
    let path = repo.join(NUL_BYTE_FILE);
    if !path.exists() {
        return; // file legitimately absent (e.g. a trimmed tree)
    }

    let raw = std::fs::read(&path).expect("could not read the NUL-byte file");
    assert!(
        raw.contains(&0u8),
        "{NUL_BYTE_FILE} no longer contains the literal NUL byte — if that was \
         intentional, drop this guard; otherwise the separator was clobbered."
    );

    let names = invoke_names_in(&String::from_utf8_lossy(&raw));
    assert!(
        names.contains(NUL_BYTE_FILE_COMMAND),
        "the frontend scan did not find `{NUL_BYTE_FILE_COMMAND}` in {NUL_BYTE_FILE}. \
         Found: {names:?}"
    );
}

#[test]
fn every_frontend_invoke_has_a_registered_handler() {
    let repo = repo_root();
    let handlers = registered_handlers(&repo);
    let per_file = frontend_invokes(&repo);

    let all: BTreeSet<&String> = per_file.iter().flat_map(|(_, n)| n.iter()).collect();

    assert!(
        handlers.len() >= MIN_HANDLERS,
        "only {} handler name(s) parsed (expected >= {MIN_HANDLERS}) — parser broken",
        handlers.len()
    );
    assert!(
        all.len() >= MIN_FRONTEND_INVOKES,
        "only {} frontend invoke name(s) found (expected >= {MIN_FRONTEND_INVOKES}) — \
         the frontend scan is broken, and a vacuous pass here proves nothing. Found: {:?}",
        all.len(),
        all
    );

    // frontend ⊆ handlers. NOT the reverse: some handlers are legitimately unused.
    let mut problems = Vec::new();
    for (file, names) in &per_file {
        for name in names {
            if !handlers.contains(name) {
                let rel = file.strip_prefix(&repo).unwrap_or(file);
                problems.push(format!(
                    "  {}: frontend calls \"{}\" but no handler is registered",
                    rel.display(),
                    name
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "IPC contract drift — {} frontend command name(s) have no matching Tauri \
         command in `generate_handler!` (crates/eko-tauri/src/lib.rs).\n{}\n\n\
         Registered handlers ({}): {:?}",
        problems.len(),
        problems.join("\n"),
        handlers.len(),
        handlers
    );

    eprintln!(
        "ipc_contract: {} registered handlers, {} distinct frontend invoke names across {} file(s) — all matched",
        handlers.len(),
        all.len(),
        per_file.len()
    );
}
