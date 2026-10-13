//! Shared browser pool setup for the wasm benches.
//!
//! Included via `#[path]` by each bench target (they're separate crates), so
//! it's compiled once per target. Kept dependency-light: only `mpz-common`,
//! `rayon`, `web-spawn` and `wasm-bindgen-futures`.
#![allow(dead_code)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use js_sys::Promise;
use mpz_common::{ThreadPool, ThreadPoolBuildError};
use wasm_bindgen_futures::JsFuture;

/// `web_spawn::start_spawner` panics if invoked more than once, so guard it.
static SPAWNER_STARTED: AtomicBool = AtomicBool::new(false);

/// Sizes pools off the browser's hardware concurrency.
fn num_threads() -> usize {
    web_spawn::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
}

/// Starts the `web_spawn` spawner. Must run before any pool spawns workers.
/// Idempotent: `web_spawn::start_spawner` panics if called twice.
async fn start_spawner() {
    if SPAWNER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    JsFuture::from(web_spawn::start_spawner()).await.unwrap();
}

/// Starts the spawner and installs the process-wide mpz `ThreadPool` with a
/// browser (Web Worker) spawn callback, so every replay session reuses the same
/// workers. Idempotent.
pub async fn init_mpz() {
    start_spawner().await;

    let builder = ThreadPool::builder().num_threads(num_threads()).spawn(|f| {
        let _ = web_spawn::spawn(f);
        Ok(())
    });

    match builder.build_global() {
        Ok(()) | Err(ThreadPoolBuildError::AlreadyInitialized) => {}
        Err(e) => panic!("failed to build global mpz thread pool: {e}"),
    }
}

/// Starts the spawner and installs the process-wide rayon pool with a browser
/// (Web Worker) spawn handler. Idempotent.
///
/// Note: this only makes rayon's *workers* run in Web Workers. Rayon's `par_*`
/// combinators park their caller while work is stolen, and parking is illegal
/// on the browser main thread, so pools built this way must only be used from
/// inside a worker too. Prefer [`run_in_rayon_worker`] for blocking
/// rayon-parallel work.
pub async fn init_rayon() {
    start_spawner().await;

    match rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads())
        .spawn_handler(|thread| {
            let _ = web_spawn::spawn(move || thread.run());
            Ok(())
        })
        .build_global()
    {
        Ok(()) => {}
        // rayon's error type is opaque (no public variant/predicate), so
        // distinguish the expected "already initialized" case by message.
        Err(e) if e.to_string().contains("already been initialized") => {}
        Err(e) => panic!("failed to build global rayon pool: {e}"),
    }
}

/// Runs `f` on a fresh Web Worker and returns its output.
///
/// Blocking on WebAssembly atomics (`Atomics.wait`, thread parking) is not
/// allowed on the browser main thread, but it is in a Worker. Rayon's `par_*`
/// combinators park their caller while stealing work, so rayon-parallel
/// sections (e.g. `mpz-zk-core`'s `check`) must run here rather than directly
/// on the event loop.
///
/// `f` runs inside a local rayon pool whose worker threads are themselves
/// spawned as Web Workers, so `rayon::par_*` calls inside `f` execute in
/// parallel and any blocking happens on the worker thread. The main thread
/// only polls for the result, yielding to the event loop.
pub async fn run_in_rayon_worker<R, F>(f: F) -> R
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    start_spawner().await;

    let result: Arc<Mutex<Option<R>>> = Arc::new(Mutex::new(None));
    let result_clone = result.clone();
    let _ = web_spawn::spawn(move || {
        // Build the pool on the worker thread (rayon's constructor can block).
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads())
            .spawn_handler(|thread| {
                let _ = web_spawn::spawn(move || thread.run());
                Ok(())
            })
            .build()
            .expect("failed to build local rayon pool");
        // `install` runs `f` as a member of this pool, so `par_*` inside `f`
        // uses it; worker threads are Web Workers where parking is legal.
        let value = pool.install(f);
        *result_clone.lock().unwrap() = Some(value);
    });

    // Poll for completion, yielding to the event loop between checks so the
    // worker can make progress. Never block the main thread.
    loop {
        if let Some(value) = result.lock().unwrap().take() {
            return value;
        }
        let promise = Promise::new(&mut |resolve, _| {
            web_sys::window()
                .expect("window exists on the main thread")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 1)
                .expect("failed to schedule timeout");
        });
        JsFuture::from(promise).await.expect("setTimeout promise");
    }
}
