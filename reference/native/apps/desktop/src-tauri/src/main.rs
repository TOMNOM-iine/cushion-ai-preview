#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod provider;
mod subscription_auth;
mod subscription_host;
mod subscription_transport;
use cushion_core::LocalFailure;
use cushion_core::{Core, Request, Snapshot, MAX_REQUEST_BYTES};
use cushion_inference::runtime::{development_root, ModelStatus, Runtime};
use cushion_vault::Vault;
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::Manager;
struct Shared {
    generation: AtomicU64,
    core: Mutex<Core>,
    worker: Mutex<Option<Runtime>>,
    model: Mutex<ModelStatus>,
    cancel: Mutex<Arc<AtomicBool>>,
    vault: Mutex<Vault>,
    subscription: Mutex<subscription_host::Connection>,
    // Briefly held only to publish/capture a generation's cancellation signal.
    // Never wait for Core or Connection while holding this mutex.
    subscription_cancel: Mutex<Arc<AtomicBool>>,
}
struct PrivacyState(Arc<Shared>);
fn unavailable() -> String {
    "処理を安全に続けられません。原文を保持したままアプリを開き直してください。".into()
}
// Except startup, callers hold Core: reload invalidation and ready/busy transitions
// cannot race approval/export. Generation also rejects an old loader/worker result.
fn reload(shared: Arc<Shared>) {
    let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut status) = shared.model.lock() {
        *status = ModelStatus::initial();
    }
    std::thread::spawn(move || {
        let Ok(mut slot) = shared.worker.lock() else {
            return;
        };
        if shared.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        *slot = None;
        let result = Runtime::start(&development_root());
        if shared.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        if let Ok(mut status) = shared.model.lock() {
            if shared.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match result {
                Ok(worker) => {
                    *status = ModelStatus::ready(&worker.fingerprint);
                    *slot = Some(worker)
                }
                Err(e) => *status = ModelStatus::failed(&e),
            }
        }
    });
}
fn cancel_active(s: &Shared) {
    if let Ok(c) = s.cancel.lock() {
        c.store(true, Ordering::SeqCst);
    }
}
fn copy_approved(text: &str) -> Result<(), String> {
    let mut child = Command::new("/usr/bin/pbcopy")
        .env_clear()
        .env("LANG", "ja_JP.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "クリップボードを開けませんでした。再承認してからやり直してください。")?;
    let mut stdin = child.stdin.take().ok_or_else(unavailable)?;
    stdin
        .write_all(text.as_bytes())
        .map_err(|_| unavailable())?;
    drop(stdin);
    if !child.wait().map_err(|_| unavailable())?.success() {
        return Err("クリップボードへ書き込めませんでした。送信はしていません。".into());
    }
    Ok(())
}
fn approved_copy<F>(core: &mut Core, approval_id: &str, write: F) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let mut attempted = false;
    core.export_approved_with(approval_id, |text| {
        attempted = true;
        write(text)
    })
    .map_err(|reason| {
        if attempted {
            "クリップボードへの書込み完了を確認できませんでした。承認を取り直してください。AIへの送信は行っていません。".into()
        } else {
            reason
        }
    })
}
#[tauri::command]
async fn dispatch(
    request: serde_json::Value,
    state: tauri::State<'_, PrivacyState>,
) -> Result<Snapshot, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || dispatch_inner(request, &shared))
        .await
        .map_err(|_| unavailable())?
}
fn dispatch_inner(request: serde_json::Value, shared: &Arc<Shared>) -> Result<Snapshot, String> {
    let reject_edit = || -> Result<(), String> {
        if matches!(
            request.get("op").and_then(|value| value.as_str()),
            Some("update" | "cancel" | "new_case")
        ) {
            cancel_active(shared);
            let keys = request
                .get("patch")
                .and_then(|value| value.as_object())
                .map(|patch| patch.keys().map(String::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            shared
                .core
                .lock()
                .map_err(|_| unavailable())?
                .mark_rejected_edit(&keys);
        }
        Ok(())
    };
    let json = match serde_json::to_string(&request) {
        Ok(value) => value,
        Err(_) => {
            reject_edit()?;
            return Err(unavailable());
        }
    };
    if json.len() > MAX_REQUEST_BYTES {
        reject_edit()?;
        return Err("操作データが上限を超えています。".into());
    }
    let req: Request = match serde_json::from_str(&json) {
        Ok(value) => value,
        Err(_) => {
            reject_edit()?;
            return Err("操作の形式を確認できませんでした。".into());
        }
    };
    if matches!(
        &req,
        Request::Update { .. } | Request::Cancel {} | Request::NewCase {}
    ) {
        cancel_active(shared)
    }
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    if matches!(
        &req,
        Request::Update { .. } | Request::Cancel {} | Request::NewCase {}
    ) {
        cancel_active(shared)
    }
    match req {
        Request::BeginScan {} if core.snapshot().mode == "local" => {
            let status = shared.model.lock().map_err(|_| unavailable())?.clone();
            let job = match core.begin_local_scan(
                status.model_config.as_deref().unwrap_or(""),
                status.state == "ready",
            ) {
                Ok(j) => j,
                Err(_) => return Ok(core.snapshot()),
            };
            let snapshot = core.snapshot();
            let cancel = Arc::new(AtomicBool::new(false));
            *shared.cancel.lock().map_err(|_| unavailable())? = cancel.clone();
            if let Ok(mut status) = shared.model.lock() {
                status.state = "busy".into();
                status.reason = Some("このPC内で機密候補を確認しています。".into());
            }
            let generation = shared.generation.load(Ordering::SeqCst);
            let shared = Arc::clone(shared);
            std::thread::spawn(move || {
                let result = shared
                    .worker
                    .lock()
                    .map_err(|_| unavailable())
                    .and_then(|mut w| {
                        let worker = w.as_mut().ok_or_else(unavailable)?;
                        if shared.generation.load(Ordering::SeqCst) != generation
                            || worker.fingerprint != job.model_fingerprint
                            || cancel.load(Ordering::SeqCst)
                        {
                            return Err(unavailable());
                        }
                        worker.infer(&job.original, &job.document_id, &cancel)
                    });
                apply_worker_result(&shared, &job, generation, result, &cancel);
            });
            Ok(snapshot)
        }
        Request::ExportPrepared { approval_id } => {
            verify_current_model(shared, &mut core)?;
            approved_copy(&mut core, &approval_id, copy_approved)?;
            Ok(core.snapshot())
        }
        other => core.dispatch(other),
    }
}
fn apply_worker_result(
    shared: &Arc<Shared>,
    job: &cushion_core::LocalScanJob,
    generation: u64,
    result: Result<cushion_inference::runtime::InferenceResult, String>,
    cancel: &AtomicBool,
) {
    let failed = result.is_err();
    if let Ok(mut core) = shared.core.lock() {
        if shared.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        match result {
            Ok(v) => {
                let _ = core.finish_local_scan(&job.job_id, &v.result.to_string());
            }
            Err(_) => {
                core.fail_local_scan(
                    &job.job_id,
                    if cancel.load(Ordering::SeqCst) {
                        LocalFailure::Stopped
                    } else {
                        LocalFailure::InvalidOutput
                    },
                );
            }
        }
        if failed {
            reload(shared.clone())
        } else if let Ok(mut status) = shared.model.lock() {
            *status = ModelStatus::ready(&job.model_fingerprint);
        }
    };
}
fn model_status_inner(shared: &Shared) -> Result<ModelStatus, String> {
    let mut core = match shared.core.try_lock() {
        Ok(core) => core,
        // The next poll checks idle liveness. Never accumulate blocking status
        // tasks behind an in-progress API permission transaction.
        Err(std::sync::TryLockError::WouldBlock) => {
            return Ok(shared.model.lock().map_err(|_| unavailable())?.clone())
        }
        Err(std::sync::TryLockError::Poisoned(_)) => return Err(unavailable()),
    };
    let status = shared.model.lock().map_err(|_| unavailable())?.clone();
    // Drop model BEFORE touching worker; reload owns worker first. Busy/verifying
    // workers remain with their owner. An idle death invalidates approval here.
    if status.state == "ready" {
        match live_worker_fingerprint(shared) {
            Ok(fingerprint) if status.model_config.as_deref() == Some(&fingerprint) => {}
            _ => mark_model_stopped(shared, &mut core)?,
        }
    }
    Ok(shared.model.lock().map_err(|_| unavailable())?.clone())
}
#[tauri::command]
async fn model_status(state: tauri::State<'_, PrivacyState>) -> Result<ModelStatus, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || model_status_inner(&shared))
        .await
        .map_err(|_| unavailable())?
}
fn reload_inner(shared: &Arc<Shared>) -> Result<(), String> {
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let status = shared
        .model
        .lock()
        .map_err(|_| unavailable())?
        .state
        .clone();
    if status == "busy" || status == "verifying" {
        return Err("現在の処理が終わるまでお待ちください。".into());
    }
    cancel_active(shared);
    core.dispatch(Request::Cancel {})?;
    reload(shared.clone());
    Ok(())
}
#[tauri::command]
async fn model_reload(state: tauri::State<'_, PrivacyState>) -> Result<(), String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || reload_inner(&shared))
        .await
        .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_status(state: tauri::State<'_, PrivacyState>) -> Result<serde_json::Value, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let v = shared.vault.lock().map_err(|_| unavailable())?;
        Ok(serde_json::json!({"available":true,"unlocked":v.status().unlocked,"reason":null}))
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_unlock(state: tauri::State<'_, PrivacyState>) -> Result<serde_json::Value, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut v = shared.vault.lock().map_err(|_| unavailable())?;
        v.unlock()
            .map_err(|_| "保存庫を開けません。キーチェーンの状態を確認してください。")?;
        Ok(serde_json::json!({"available":true,"unlocked":v.status().unlocked,"reason":null}))
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_lock(state: tauri::State<'_, PrivacyState>) -> Result<serde_json::Value, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut v = shared.vault.lock().map_err(|_| unavailable())?;
        v.lock().map_err(|_| unavailable())?;
        Ok(serde_json::json!({"available":true,"unlocked":v.status().unlocked,"reason":null}))
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_list(state: tauri::State<'_, PrivacyState>) -> Result<serde_json::Value, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let v = shared.vault.lock().map_err(|_| unavailable())?;
        serde_json::to_value(
            v.list()
                .map_err(|_| "保存一覧を読み込めません。保存庫を開いてください。")?,
        )
        .map_err(|_| unavailable())
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_save(state: tauri::State<'_, PrivacyState>) -> Result<serde_json::Value, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let core = shared.core.lock().map_err(|_| unavailable())?;
        let bytes = zeroize::Zeroizing::new(core.export_session()?);
        let mut v = shared.vault.lock().map_err(|_| unavailable())?;
        let entry = v
            .save(&bytes)
            .map_err(|_| "相談を暗号化保存できませんでした。原文は画面に残しています。")?;
        drop(bytes);
        serde_json::to_value(entry).map_err(|_| unavailable())
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_load(id: String, state: tauri::State<'_, PrivacyState>) -> Result<Snapshot, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        cancel_active(&shared);
        let mut core = shared.core.lock().map_err(|_| unavailable())?;
        let v = shared.vault.lock().map_err(|_| unavailable())?;
        let bytes = v
            .load(&id)
            .map_err(|_| "保存した相談を開けませんでした。")?;
        core.import_session(&bytes)
    })
    .await
    .map_err(|_| unavailable())?
}
#[tauri::command]
async fn vault_delete(id: String, state: tauri::State<'_, PrivacyState>) -> Result<bool, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .vault
            .lock()
            .map_err(|_| unavailable())?
            .delete(&id)
            .map_err(|_| "保存を削除できませんでした。".into())
    })
    .await
    .map_err(|_| unavailable())?
}
const MODEL_STOPPED: &str = "ローカルモデルの動作を確認できません。コピー・API送信を停止しました。モデルを再読込して再検査してください。";
fn live_worker_fingerprint(shared: &Shared) -> Result<String, String> {
    let mut slot = shared
        .worker
        .lock()
        .map_err(|_| MODEL_STOPPED.to_string())?;
    if let Some(worker) = slot.as_mut() {
        if worker.is_alive() {
            return Ok(worker.fingerprint.clone());
        }
    }
    *slot = None;
    Err(MODEL_STOPPED.into())
}
fn mark_model_stopped(shared: &Shared, core: &mut Core) -> Result<(), String> {
    cancel_active(shared);
    core.dispatch(Request::Cancel {})?;
    shared.generation.fetch_add(1, Ordering::SeqCst);
    *shared.model.lock().map_err(|_| unavailable())? = ModelStatus::failed(MODEL_STOPPED);
    Ok(())
}
fn verify_current_model(shared: &Shared, core: &mut Core) -> Result<(), String> {
    verify_current_model_with(shared, core, live_worker_fingerprint)
}
// Synthetic probes are selected only by Rust tests, never IPC or UI settings.
// Production entrypoints always pass the actual-child probe above.
fn verify_current_model_with<P>(shared: &Shared, core: &mut Core, probe: P) -> Result<(), String>
where
    P: FnOnce(&Shared) -> Result<String, String>,
{
    let model = shared.model.lock().map_err(|_| unavailable())?.clone();
    if model.state != "ready"
        || model.model_config.as_deref() != Some(&core.snapshot().model_config)
    {
        core.dispatch(Request::Cancel {})?;
        return Err("モデルの準備状態が変わりました。再検査してから承認してください。".into());
    }
    // Never hold model while waiting for worker: reload uses worker -> model.
    match probe(shared) {
        Ok(fingerprint) if Some(fingerprint.as_str()) == model.model_config.as_deref() => Ok(()),
        _ => {
            mark_model_stopped(shared, core)?;
            Err(MODEL_STOPPED.into())
        }
    }
}
#[tauri::command]
async fn api_status() -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(||serde_json::json!({"configuredOpenAI":provider::configured("OpenAI"),"configuredClaude":provider::configured("Claude")})).await.map_err(|_|unavailable())
}
#[tauri::command]
async fn api_set_key(
    provider: String,
    key: String,
    state: tauri::State<'_, PrivacyState>,
) -> Result<(), String> {
    let key = zeroize::Zeroizing::new(key);
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        cancel_active(&shared);
        let mut core = shared.core.lock().map_err(|_| unavailable())?;
        cancel_active(&shared);
        core.dispatch(Request::Cancel {})?;
        provider::set_key(&provider, &key)
    })
    .await
    .map_err(|_| unavailable())?
}
// One permission transaction: final model/approval checks, transport, token
// registration and restoration all hold Core. It runs off the UI thread.
#[cfg(test)]
fn approved_exchange<F>(shared: &Shared, approval_id: &str, effect: F) -> Result<Snapshot, String>
where
    F: FnOnce(&str, &str) -> Result<String, String>,
{
    approved_exchange_with_probe(shared, approval_id, live_worker_fingerprint, effect)
}
#[cfg(test)]
fn approved_exchange_with_probe<P, F>(
    shared: &Shared,
    approval_id: &str,
    probe: P,
    effect: F,
) -> Result<Snapshot, String>
where
    P: FnOnce(&Shared) -> Result<String, String>,
    F: FnOnce(&str, &str) -> Result<String, String>,
{
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    verify_current_model_with(shared, &mut core, probe)?;
    let snapshot = core.snapshot();
    let provider = match snapshot.destination.as_str() {
        "ChatGPT" => "OpenAI",
        "Claude" => "Claude",
        _ => return Err("このAPI接続先には対応していません。".into()),
    };
    let answer = core.export_approved_with(approval_id, |text| effect(provider, text))?;
    core.dispatch(Request::RestoreIfCurrent {
        answer,
        case_id: snapshot.case_id,
        revision: snapshot.revision,
    })
}
#[tauri::command]
async fn api_send(
    approval_id: String,
    state: tauri::State<'_, PrivacyState>,
) -> Result<Snapshot, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        subscription_host::send_selected(&shared, &approval_id)
    })
    .await
    .map_err(|_| unavailable())?
}
fn main() {
    let _ = tauri::Builder::default()
        .menu(|app| {
            let menu = tauri::menu::Menu::default(app)?;
            let zoom = tauri::menu::SubmenuBuilder::new(app, "表示サイズ")
                .text("zoom_100", "100%（標準）")
                .text("zoom_200", "200%（拡大）")
                .build()?;
            menu.append(&zoom)?;
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            let scale = match event.id().as_ref() {
                "zoom_100" => 1.0,
                "zoom_200" => 2.0,
                _ => return,
            };
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_zoom(scale);
            }
        })
        .setup(|app| {
            let core = Core::new().map_err(std::io::Error::other)?;
            let plan_entry = std::env::args().any(|arg| arg == "--subscription");
            let mut core = core;
            let subscription = subscription_host::Connection::new(plan_entry);
            core.bind_delivery_config(&subscription.binding())
                .map_err(std::io::Error::other)?;
            let shared = Arc::new(Shared {
                generation: AtomicU64::new(0),
                core: Mutex::new(core),
                worker: Mutex::new(None),
                model: Mutex::new(ModelStatus::initial()),
                cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
                vault: Mutex::new(Vault::new(app.path().app_data_dir()?)),
                subscription_cancel: Mutex::new(subscription.cancel_signal()),
                subscription: Mutex::new(subscription),
            });
            app.manage(PrivacyState(shared.clone()));
            // Only a fixed local page is selectable; argv never carries a document.
            let support_record = std::env::args().any(|arg| arg == "--support-record");
            if support_record || std::env::args().any(|arg| arg == "--embedded") {
                if let Some(window) = app.get_webview_window("main") {
                    let mut url = window.url()?;
                    url.set_path(if support_record {
                        "/support.html"
                    } else {
                        "/embedded.html"
                    });
                    url.set_query(None);
                    url.set_fragment(None);
                    window.navigate(url)?;
                }
            }
            reload(shared);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            dispatch,
            model_status,
            model_reload,
            vault_status,
            vault_unlock,
            vault_lock,
            vault_list,
            vault_save,
            vault_load,
            vault_delete,
            api_status,
            api_set_key,
            api_send,
            subscription_host::subscription_status,
            subscription_host::subscription_sign_in,
            subscription_host::subscription_cancel,
            subscription_host::subscription_select
        ])
        .run(tauri::generate_context!());
}

#[cfg(test)]
mod safety_tests;
