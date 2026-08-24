//! The `reqwest::blocking`-inside-tokio guard for the `subsonic_*` commands.
//!
//! [`eko_net::Client`] wraps a [`reqwest::blocking::Client`], which stands up its own
//! tokio runtime. Constructing one from a thread that is already inside a tokio runtime
//! — which is exactly what a `#[tauri::command]` is — panics:
//!
//! ```text
//! Cannot drop a runtime in a context where blocking is not allowed.
//! This happens when a runtime is dropped from within an asynchronous context.
//! ```
//!
//! Nothing about that is visible to the type checker, and every wrapper in
//! `commands::subsonic` compiles just as happily with the `spawn_blocking` removed. The
//! failure would land only at runtime, on a user's machine, on the connect screen. So
//! the hazard is pinned here instead: a negative control that asserts the naive form
//! still blows up, and a positive test that walks the real command lifecycle.
//!
//! Both tests target `127.0.0.1:1`, where nothing listens: a *working* call fails fast
//! with a transport error, so no network, no server and no fixtures are involved. The
//! distinction under test is panic vs. no panic, never success vs. failure.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

fn cfg() -> eko_net::Config {
    eko_net::Config {
        base_url: "http://127.0.0.1:1".into(),
        username: "rod".into(),
        password: "hunter2".into(),
    }
}

/// Negative control 1 of 2 — the **construction** hazard, which guards the
/// `spawn_blocking` in `subsonic_set_config`.
///
/// Without the two negative controls the positive test could pass vacuously — if a
/// future `reqwest` made blocking clients runtime-safe, `spawn_blocking` would no longer
/// be load-bearing and a green positive test would prove nothing.
///
/// **The panic printed while this test runs is the expected outcome, not a failure.**
///
/// If this ever starts failing, the hazard has genuinely gone away. That is not licence
/// to strip `spawn_blocking` out of `commands::subsonic`: a blocking network call on the
/// async executor still parks a Tauri worker thread for the duration of the request.
#[test]
fn constructing_a_client_on_the_async_executor_still_panics() {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        tauri::async_runtime::block_on(async {
            // `forget`, so that a panic here can only be blamed on construction.
            std::mem::forget(eko_net::Client::new(cfg()).unwrap());
        });
    }));
    assert!(
        outcome.is_err(),
        "`eko_net::Client::new` no longer panics on the async executor — see this \
         test's doc comment before changing anything in commands::subsonic"
    );
}

/// Negative control 2 of 2 — the **call** hazard, which guards the `spawn_blocking` in
/// `commands::subsonic::run`, and with it all fifteen endpoint wrappers.
///
/// Not implied by the control above, and this is the gap it closes: here the client is
/// built correctly, off the executor, exactly as `subsonic_set_config` builds it — only
/// the *request* happens in the wrong place. Before this test existed, deleting
/// `spawn_blocking` from `run()` while leaving `subsonic_set_config` untouched kept the
/// entire suite green while the app panicked on the first `ping`.
///
/// **The panic printed while this test runs is the expected outcome, not a failure.**
#[test]
fn calling_a_client_on_the_async_executor_still_panics() {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        tauri::async_runtime::block_on(async {
            let client = tauri::async_runtime::spawn_blocking(move || eko_net::Client::new(cfg()))
                .await
                .expect("construction thread panicked")
                .expect("client construction failed");
            // Born off the executor, so construction cannot be the thing that panics —
            // only the blocking request below, issued from inside the runtime. `forget`
            // keeps the drop out of the picture too. Neither line is reached today: the
            // panic fires inside `ping`.
            let result = client.ping();
            std::mem::forget(client);
            drop(result);
        });
    }));
    assert!(
        outcome.is_err(),
        "a blocking request no longer panics on the async executor — see this test's \
         doc comment before removing `spawn_blocking` from commands::subsonic::run"
    );
}

/// The whole `subsonic_*` lifecycle, in the order the commands perform it:
///
/// 1. `subsonic_set_config` builds the client **inside** `spawn_blocking`,
/// 2. a command clones the `Arc` handle and calls through `spawn_blocking`,
/// 3. `subsonic_set_config(None)` drops the last `Arc` back in the async context.
///
/// Step 3 is the subtle one: the drop is *not* moved to a blocking thread, and it does
/// not need to be — a client that was born on a blocking thread can be dropped anywhere.
/// A client built on the executor cannot even be born.
#[test]
fn the_command_lifecycle_never_panics_on_the_async_executor() {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        tauri::async_runtime::block_on(async {
            // 1. construct, off the executor
            let client = Arc::new(
                tauri::async_runtime::spawn_blocking(move || eko_net::Client::new(cfg()))
                    .await
                    .expect("construction thread panicked")
                    .expect("client construction failed"),
            );

            // 2. call, off the executor — mirrors `commands::subsonic::run`
            let handle = client.clone();
            let result = tauri::async_runtime::spawn_blocking(move || handle.ping())
                .await
                .expect("request thread panicked");

            // 3. clear the managed state, on the executor
            let mut slot = Some(client);
            slot.take();

            result
        })
    }));

    let result = outcome.expect("the command lifecycle panicked on the async executor");
    assert!(
        matches!(result, Err(eko_net::SubsonicError::Request(_))),
        "expected a transport error from the dead port, got {result:?}"
    );
}
