//! The Genzo shell. Maps the engine API to Tauri IPC and nothing more:
//! commands and queries go through `invoke`, the engine's events through
//! `emit` as its sink receives them, and frames and thumbnails through the
//! `genzo-frame` and `genzo-thumb` URI schemes as raw bytes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use genzo_engine::Engine;
use genzo_engine::api::{
    CONTRACT_VERSION, Command, CommandResult, ContractVersion, Handshake, HandshakeResult, Query,
    QueryResult,
};
use tauri::http::{Response, StatusCode, header};
use tauri::{Emitter, Manager, State};

/// Event name and URI scheme; must match frontend/src/lib/transport/tauri.ts.
const EVENT: &str = "genzo://event";
const FRAME_SCHEME: &str = "genzo-frame";
const THUMB_SCHEME: &str = "genzo-thumb";

/// The contract version of the bundled frontend, from contract/contract.lock.
const APP_CONTRACT_VERSION: &str = env!("GENZO_APP_CONTRACT_VERSION");

#[tauri::command]
fn handshake(engine: State<'_, Arc<Engine>>, hello: Handshake) -> HandshakeResult {
    engine.handshake(&hello)
}

#[tauri::command]
fn query(engine: State<'_, Arc<Engine>>, query: Query) -> QueryResult {
    engine.query(query)
}

#[tauri::command]
async fn execute(
    engine: State<'_, Arc<Engine>>,
    command: Command,
) -> Result<CommandResult, String> {
    let engine = Arc::clone(&engine);
    tauri::async_runtime::spawn_blocking(move || engine.execute(command))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The tile messages under `path`'s number: a frame's by generation, a
/// thumbnail's by photo id.
fn bytes_response(path: &str, get: impl FnOnce(u64) -> Option<Vec<u8>>) -> Response<Vec<u8>> {
    let bytes = path.trim_start_matches('/').parse::<u64>().ok().and_then(get);
    let (status, body) = match bytes {
        Some(bytes) => (StatusCode::OK, bytes),
        None => (StatusCode::NOT_FOUND, Vec::new()),
    };
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .expect("static response parts are valid")
}

fn main() {
    let app = ContractVersion::parse(APP_CONTRACT_VERSION);
    let engine = ContractVersion::current();
    if !app.is_some_and(|v| v.is_compatible_with(engine)) {
        eprintln!(
            "Genzo cannot start: the app was built for contract {APP_CONTRACT_VERSION}, \
             the engine speaks {CONTRACT_VERSION}."
        );
        std::process::exit(1);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let mut engine = Engine::new();
            // Without a cache directory, thumbnails are kept in memory only.
            if let Ok(dir) = app.path().app_cache_dir() {
                engine = engine.with_cache_dir(dir);
            }
            let engine = engine.with_event_sink(Arc::new(move |event| {
                // Only fails once the app is closing.
                let _ = handle.emit(EVENT, event);
            }));
            app.manage(Arc::new(engine));
            Ok(())
        })
        .register_uri_scheme_protocol(FRAME_SCHEME, |ctx, request| {
            let engine = ctx.app_handle().state::<Arc<Engine>>();
            bytes_response(request.uri().path(), |generation| engine.frame(generation))
        })
        .register_uri_scheme_protocol(THUMB_SCHEME, |ctx, request| {
            let engine = ctx.app_handle().state::<Arc<Engine>>();
            bytes_response(request.uri().path(), |photo| engine.thumbnail(photo))
        })
        .invoke_handler(tauri::generate_handler![handshake, query, execute])
        .run(tauri::generate_context!())
        .expect("error while running Genzo");
}
