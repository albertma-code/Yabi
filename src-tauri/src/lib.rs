//! Bilio — Tauri shell that owns the Python sidecar process.
//!
//! Architecture: the Rust shell spawns a single long-lived Python sidecar
//! (`binaries/bilio-sidecar-<target-triple>`, built via `sidecar/build.sh`) and
//! brokers JSONL requests/responses between the Vue frontend and yt-dlp.
//!
//! - Request flow:  frontend `invoke("ping_sidecar")` → Rust assigns an id,
//!   writes `{"id":N,"cmd":"ping"}\n` to the sidecar's stdin, awaits the
//!   matching response on a `oneshot` channel resolved by the stdout reader.
//! - Unsolicited messages (`id == null`, e.g. `ready` / future `log`) are
//!   forwarded to the frontend as a Tauri event `sidecar://message`.
//!
//! Protocol details live in `sidecar/bilio_sidecar/rpc.py`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::oneshot;

/// State shared between the stdout reader task and `#[tauri::command]` handlers.
///
/// `child` is taken (`Option`) so a future `shutdown` command could move the
/// handle out and `kill()` it. `pending` correlates outbound request ids with
/// the oneshot sender that resolves the awaiting `invoke` call.
struct SidecarState {
    child: Mutex<Option<CommandChild>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_id: AtomicU64,
}

impl SidecarState {
    fn new() -> Self {
        Self {
            child: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }
}

/// Spawn the sidecar binary and start the stdout reader. Called once from
/// `setup`. Panics on failure — we cannot run without the sidecar.
fn spawn_sidecar(app: &AppHandle, state: &SidecarState) -> Result<(), String> {
    let (mut rx, child) = app
        .shell()
        .sidecar("bilio-sidecar")
        .map_err(|e| format!("sidecar not found: {e}"))?
        .spawn()
        .map_err(|e| format!("failed to spawn sidecar: {e}"))?;

    *state.child.lock().unwrap() = Some(child);

    // Reader task: each `CommandEvent::Stdout` already arrives line-by-line
    // (Tauri's plugin-shell splits on '\n'), so each line is one JSON object.
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    let line = String::from_utf8_lossy(&bytes);
                    handle_sidecar_line(&app_handle, line.trim());
                }
                CommandEvent::Stderr(bytes) => {
                    // Sidecar logs go to stderr by design — surface them in the
                    // Rust console for debugging, but never to the frontend.
                    eprintln!(
                        "[sidecar:stderr] {}",
                        String::from_utf8_lossy(&bytes).trim_end()
                    );
                }
                CommandEvent::Terminated(payload) => {
                    eprintln!("[sidecar] terminated: {:?}", payload);
                    // Drop all pending senders so awaiters get an error rather
                    // than hanging forever.
                    let state: State<SidecarState> = app_handle.state();
                    state.pending.lock().unwrap().clear();
                    break;
                }
                CommandEvent::Error(e) => {
                    eprintln!("[sidecar] error: {e}");
                }
                _ => {}
            }
        }
    });

    Ok(())
}

/// Parse one JSONL line from the sidecar and either resolve a pending request
/// or forward it to the frontend as an event.
///
/// Terminal messages (`type == "result"` or `"error"`) consume the pending
/// oneshot for their id. Non-terminal messages (`progress`, `parse_progress`,
/// future streaming types) keep the pending entry alive and fall through to
/// the `sidecar://message` event bus so the UI can react incrementally.
///
/// Note: long-running commands (`download`) emit MULTIPLE messages for the
/// same id — first a `result` with `status: "started"` (consumed by the
/// awaiting invoke), then a stream of `progress`/terminal `result`/`error`.
/// Once pending is removed, follow-ups fall through to the event channel.
fn handle_sidecar_line(app: &AppHandle, line: &str) {
    if line.is_empty() {
        return;
    }
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[sidecar] non-JSON on stdout (protocol violation): {e}: {line:?}");
            return;
        }
    };

    let msg_type = msg.get("type").and_then(Value::as_str).unwrap_or("");
    let is_terminal = matches!(msg_type, "result" | "error" | "pong");

    if is_terminal {
        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
            let state: State<SidecarState> = app.state();
            let maybe_tx = state.pending.lock().unwrap().remove(&id);
            if let Some(tx) = maybe_tx {
                let _ = tx.send(msg);
                return;
            }
            // No pending sender — streaming follow-up after the awaiting
            // invoke already resolved (e.g. download terminal status).
            // Fall through and emit to the frontend.
        }
    }

    // Forward as a frontend event. UI listeners filter by `type` and `id`.
    let _ = app.emit("sidecar://message", line.to_string());
}

/// Send a request to the sidecar and await its terminal response.
///
/// Returns the full JSON message (so callers can read either `data` or
/// `error`); shape it at the call site.
async fn request(state: &SidecarState, cmd: &str, args: Value) -> Result<Value, String> {
    let id = state.alloc_id();
    let (tx, rx) = oneshot::channel();
    state.pending.lock().unwrap().insert(id, tx);

    let payload = serde_json::json!({ "id": id, "cmd": cmd, "args": args });
    let line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    {
        let mut guard = state.child.lock().unwrap();
        let child = guard
            .as_mut()
            .ok_or_else(|| "sidecar not running".to_string())?;
        child
            .write(format!("{line}\n").as_bytes())
            .map_err(|e| format!("failed to write to sidecar stdin: {e}"))?;
    }

    let resp = rx
        .await
        .map_err(|_| "sidecar dropped response (terminated?)".to_string())?;

    // Sidecar reports errors in-band as `{"type":"error","error":"..."}`.
    if resp.get("type").and_then(Value::as_str) == Some("error") {
        let err = resp
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown sidecar error");
        return Err(err.to_string());
    }
    Ok(resp)
}

#[tauri::command]
async fn ping_sidecar(state: State<'_, SidecarState>) -> Result<Value, String> {
    // Forward the whole response object; the UI shows `ts` to confirm round-trip.
    request(&state, "ping", Value::Null).await
}

/// Start a parse. Returns the `job_id` (== request id) the frontend uses to
/// filter `parse_progress` + terminal `result`/`error` events on the
/// `sidecar://message` channel.
///
/// The parse must be two-step (id-out then stream-in) so the frontend can
/// subscribe before the first `parse_progress` event fires — the sidecar
/// emits one per resolved entry while it's still inside the call, so an
/// invoke that returned the final result would have already shipped every
/// progress event the UI wanted to see.
#[tauri::command]
async fn start_parse(
    url: String,
    cookies_from_browser: Option<String>,
    state: State<'_, SidecarState>,
) -> Result<u64, String> {
    let id = state.alloc_id();
    // No pending oneshot — the frontend listens for the terminal `result`
    // on the `sidecar://message` bus directly, same as it does for download
    // progress streams.

    let payload = serde_json::json!({
        "id": id,
        "cmd": "parse",
        "args": {
            "url": url,
            "cookies_from_browser": cookies_from_browser,
        }
    });
    let line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    {
        let mut guard = state.child.lock().unwrap();
        let child = guard
            .as_mut()
            .ok_or_else(|| "sidecar not running".to_string())?;
        child
            .write(format!("{line}\n").as_bytes())
            .map_err(|e| format!("failed to write to sidecar stdin: {e}"))?;
    }

    Ok(id)
}

/// Probe whether the selected browser's Bilibili cookies represent a real
/// logged-in session. Returns the sidecar's status dict verbatim — see
/// `ytdlp.check_cookies` for the shape (`logged_in`, `username`, `is_vip`,
/// `vip_label`, `error`).
#[tauri::command]
async fn check_cookies(
    cookies_from_browser: Option<String>,
    state: State<'_, SidecarState>,
) -> Result<Value, String> {
    let args = serde_json::json!({ "cookies_from_browser": cookies_from_browser });
    let resp = request(&state, "check_cookies", args).await?;
    Ok(resp.get("data").cloned().unwrap_or(resp))
}

/// Start a download. Returns the `job_id` (== request id) the frontend uses
/// to filter progress/completion events on the `sidecar://message` channel,
/// and to cancel the job later.
///
/// The sidecar acknowledges immediately with `{status: "started"}`; the
/// actual download runs on a worker thread and streams `progress` events
/// (same id), terminating in a second `result` with `status: "completed"|
/// "cancelled"` or an `error`. We listen for the first response here and
/// surface the id to the frontend.
#[tauri::command]
async fn download_video(
    url: String,
    format_id: String,
    output_dir: String,
    cookies_from_browser: Option<String>,
    state: State<'_, SidecarState>,
) -> Result<u64, String> {
    let id = state.alloc_id();
    let (tx, rx) = oneshot::channel();
    state.pending.lock().unwrap().insert(id, tx);

    let payload = serde_json::json!({
        "id": id,
        "cmd": "download",
        "args": {
            "url": url,
            "format_id": format_id,
            "output_dir": output_dir,
            "cookies_from_browser": cookies_from_browser,
        }
    });
    let line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    {
        let mut guard = state.child.lock().unwrap();
        let child = guard
            .as_mut()
            .ok_or_else(|| "sidecar not running".to_string())?;
        child
            .write(format!("{line}\n").as_bytes())
            .map_err(|e| format!("failed to write to sidecar stdin: {e}"))?;
    }

    let ack = rx
        .await
        .map_err(|_| "sidecar dropped response (terminated?)".to_string())?;

    if ack.get("type").and_then(Value::as_str) == Some("error") {
        let err = ack
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown sidecar error");
        return Err(err.to_string());
    }

    // Subsequent progress + terminal messages will arrive on the
    // `sidecar://message` event stream with the same id.
    Ok(id)
}

#[tauri::command]
async fn cancel_download(job_id: u64, state: State<'_, SidecarState>) -> Result<Value, String> {
    request(&state, "cancel", serde_json::json!({ "job_id": job_id })).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(SidecarState::new())
        .setup(|app| {
            let state: State<SidecarState> = app.state();
            spawn_sidecar(&app.handle(), &state).expect("failed to start sidecar");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping_sidecar,
            start_parse,
            check_cookies,
            download_video,
            cancel_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
