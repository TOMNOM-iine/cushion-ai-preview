//! Native connection state. Core -> connection is the only mutation lock order.
//! Cached authentication and model discovery run outside both mutation locks.
use super::*;
use cushion_core::NativeDeliveryFailure;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use subscription_auth::{PlanModel, Session};

const DISCONNECTED: &str = "ChatGPTの契約での接続が未完了です。接続設定でログインとモデルを確認してください。APIキー方式への切替や原文送信は行いません。";
const NO_MODEL: &str =
    "この契約で利用できるモデルを確認できませんでした。接続設定を確認してください。";

pub struct Connection {
    mode: String,
    state: String,
    reason: Option<String>,
    session: Option<Session>,
    selected_model: Option<String>,
    loaded: bool,
    // Stopping this screen's connection retains its Keychain record, but a poll
    // cannot reconnect it. Only explicit selection/sign-in enables loading again.
    auto_connect: bool,
    generation: u64,
    cancel: Arc<AtomicBool>,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            mode: "api_key".into(),
            state: "disconnected".into(),
            reason: None,
            session: None,
            selected_model: None,
            loaded: false,
            auto_connect: true,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    state: String,
    reason: Option<String>,
    model: Option<String>,
    models: Vec<PlanModel>,
    delivery_mode: String,
}

struct Pending {
    generation: u64,
    cancel: Arc<AtomicBool>,
}

impl Connection {
    pub fn new(plan: bool) -> Self {
        let mut result = Self::default();
        if plan {
            result.mode = "chatgpt_plan".into();
        }
        result
    }

    pub fn binding(&self) -> String {
        if self.mode == "api_key" {
            return String::new();
        }
        let session = self
            .session
            .as_ref()
            .map(Session::binding)
            .unwrap_or_else(|| "not-connected".into());
        let model = self.selected_model.as_deref().unwrap_or("not-selected");
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}|{}|{}",
                subscription_transport::CONTRACT,
                session,
                model
            ))
        )
    }

    pub(crate) fn cancel_signal(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    fn status(&self) -> Status {
        Status {
            state: self.state.clone(),
            reason: self.reason.clone(),
            model: self.selected_model.clone(),
            models: self
                .session
                .as_ref()
                .map(|s| s.models().to_vec())
                .unwrap_or_default(),
            delivery_mode: self.mode.clone(),
        }
    }

    fn fail(&mut self, reason: String) {
        self.state = "error".into();
        self.reason = Some(reason);
    }

    fn next_pending(&mut self, shared: &Shared) -> Result<Pending, String> {
        // Core -> Connection -> signal is the mutation order. A notifier only
        // captures this signal and drops its guard before touching other locks.
        let mut signal = shared
            .subscription_cancel
            .lock()
            .map_err(|_| unavailable())?;
        self.publish_pending(&mut signal)
    }

    fn next_implicit_pending(&mut self, shared: &Shared) -> Result<Option<Pending>, String> {
        let mut signal = shared
            .subscription_cancel
            .lock()
            .map_err(|_| unavailable())?;
        // A poll may have passed its first check before cancellation arrived.
        // Recheck and publish under the notifier's mutex as one operation.
        if self.cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        self.publish_pending(&mut signal).map(Some)
    }

    fn publish_pending(&mut self, signal: &mut Arc<AtomicBool>) -> Result<Pending, String> {
        let generation = self.generation.checked_add(1).ok_or_else(unavailable)?;
        self.cancel.store(true, Ordering::SeqCst);
        self.generation = generation;
        self.cancel = Arc::new(AtomicBool::new(false));
        *signal = self.cancel.clone();
        Ok(Pending {
            generation: self.generation,
            cancel: self.cancel.clone(),
        })
    }

    fn current(&self, pending: &Pending) -> bool {
        self.mode == "chatgpt_plan"
            && self.auto_connect
            && self.state == "connecting"
            && self.generation == pending.generation
            && !pending.cancel.load(Ordering::SeqCst)
    }

    fn select_available_model(&mut self) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        if self.selected_model.is_none() {
            self.selected_model = session.models().first().map(|m| m.slug.clone());
        }
        session.status().connected
            && self
                .selected_model
                .as_ref()
                .is_some_and(|slug| session.models().iter().any(|m| &m.slug == slug))
    }
}

fn prepare_session(
    mut session: Option<Session>,
    cancel: &AtomicBool,
) -> Result<Option<Session>, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("接続の確認を中止しました。".into());
    }
    if session.is_none() {
        session = Session::load()?;
    }
    if let Some(session) = session.as_mut() {
        session.ensure_access()?;
        if cancel.load(Ordering::SeqCst) {
            return Err("接続の確認を中止しました。".into());
        }
        session.fetch_models()?;
    }
    Ok(session)
}

fn status_inner_with<F>(shared: &Shared, prepare: F) -> Result<Status, String>
where
    F: FnOnce(Option<Session>, &AtomicBool) -> Result<Option<Session>, String>,
{
    let (pending, session) = {
        let mut core = match shared.core.try_lock() {
            Ok(core) => core,
            Err(std::sync::TryLockError::WouldBlock) => {
                let c = shared.subscription.lock().map_err(|_| unavailable())?;
                let mut status = c.status();
                if c.mode == "chatgpt_plan"
                    && c.auto_connect
                    && !c.loaded
                    && c.state == "disconnected"
                    && !c.cancel.load(Ordering::SeqCst)
                {
                    // Keep the UI polling while local startup owns Core. This
                    // response starts no generation, connection or credential I/O.
                    status.state = "connecting".into();
                    status.reason = Some(
                        "このPCの準備が終わってから、保存済みのChatGPT接続を確認します。".into(),
                    );
                }
                return Ok(status);
            }
            Err(_) => return Err(unavailable()),
        };
        let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
        if c.mode != "chatgpt_plan"
            || !c.auto_connect
            || c.state == "connecting"
            || c.state == "error"
            || c.cancel.load(Ordering::SeqCst)
        {
            return Ok(c.status());
        }
        if c.session.as_ref().is_some_and(|s| !s.models().is_empty()) {
            if c.select_available_model() {
                c.state = "connected".into();
                c.reason = None;
            } else {
                c.fail(NO_MODEL.into());
            }
            core.bind_delivery_config(&c.binding())?;
            return Ok(c.status());
        }
        if c.loaded && c.session.is_none() {
            return Ok(c.status());
        }
        let Some(pending) = c.next_implicit_pending(shared)? else {
            return Ok(c.status());
        };
        c.loaded = true;
        c.state = "connecting".into();
        c.reason = Some("このMacのChatGPT接続を確認しています。".into());
        let session = c.session.take();
        core.bind_delivery_config(&c.binding())?;
        (pending, session)
    };
    // No Core/connection guard survives across Keychain, refresh or catalog I/O.
    let outcome = prepare(session, &pending.cancel);
    finish_cached(shared, pending, outcome)
}

fn finish_cached(
    shared: &Shared,
    pending: Pending,
    outcome: Result<Option<Session>, String>,
) -> Result<Status, String> {
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
    if !c.current(&pending) {
        return Ok(c.status());
    }
    match outcome {
        Ok(Some(session)) => {
            c.session = Some(session);
            if c.select_available_model() {
                c.state = "connected".into();
                c.reason = None;
            } else {
                c.fail(NO_MODEL.into());
            }
        }
        Ok(None) => {
            c.state = "disconnected".into();
            c.reason = None;
        }
        Err(reason) => c.fail(reason),
    }
    core.bind_delivery_config(&c.binding())?;
    Ok(c.status())
}

#[tauri::command]
pub async fn subscription_status(state: tauri::State<'_, PrivacyState>) -> Result<Status, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || status_inner_with(&shared, prepare_session))
        .await
        .map_err(|_| unavailable())?
}

#[tauri::command]
pub async fn subscription_select(
    mode: String,
    model: Option<String>,
    state: tauri::State<'_, PrivacyState>,
) -> Result<Status, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || select_inner(&shared, &mode, model.as_deref()))
        .await
        .map_err(|_| unavailable())?
}

fn select_inner(shared: &Shared, mode: &str, model: Option<&str>) -> Result<Status, String> {
    select_inner_with(shared, mode, model, prepare_session)
}

fn select_inner_with<F>(
    shared: &Shared,
    mode: &str,
    model: Option<&str>,
    prepare: F,
) -> Result<Status, String>
where
    F: FnOnce(Option<Session>, &AtomicBool) -> Result<Option<Session>, String>,
{
    cancel_active(shared);
    {
        let mut core = shared.core.lock().map_err(|_| unavailable())?;
        let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
        core.dispatch(Request::Cancel {})?;
        c.next_pending(shared)?;
        // Rejected selections also stop a pending login/preparation and approval.
        c.auto_connect = false;
        if c.state == "connecting" {
            c.state = "disconnected".into();
        }
        if mode != "api_key" && mode != "chatgpt_plan" {
            return Err("この接続方式には対応していません。".into());
        }
        if let Some(model) = model {
            if mode != "chatgpt_plan"
                || !c
                    .session
                    .as_ref()
                    .is_some_and(|s| s.models().iter().any(|m| m.slug == model))
            {
                return Err("利用できるモデルの一覧から選び直してください。".into());
            }
            c.selected_model = Some(model.into());
        }
        c.mode = mode.into();
        c.auto_connect = mode == "chatgpt_plan";
        c.state = "disconnected".into();
        c.reason = None;
        if c.session.is_none() {
            c.loaded = false;
        }
        core.bind_delivery_config(&c.binding())?;
        if mode == "api_key" {
            return Ok(c.status());
        }
    }
    status_inner_with(shared, prepare)
}

fn signal_cancel(shared: &Shared) -> Result<Arc<AtomicBool>, String> {
    let signal = shared
        .subscription_cancel
        .lock()
        .map_err(|_| unavailable())?;
    // Connection can be held across renewal or Keychain commit. Notification
    // must reach the captured generation before waiting for that transaction.
    signal.store(true, Ordering::SeqCst);
    Ok(signal.clone())
}

fn finish_cancel(shared: &Shared, cancel: &Arc<AtomicBool>) -> Result<Status, String> {
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
    if !Arc::ptr_eq(cancel, &c.cancel) {
        // A delayed cancellation belongs to its captured generation. It must
        // not revoke a newer login, selection, approval or local scan.
        return Ok(c.status());
    }
    cancel_active(shared);
    core.dispatch(Request::Cancel {})?;
    c.next_pending(shared)?;
    c.auto_connect = false;
    c.state = "disconnected".into();
    c.reason = Some(
        "この画面の接続を止めました。保存済みのログイン情報はMacのキーチェーンに残っています。"
            .into(),
    );
    c.session = None;
    c.selected_model = None;
    c.loaded = false;
    core.bind_delivery_config(&c.binding())?;
    Ok(c.status())
}

#[cfg(test)]
fn cancel_inner(shared: &Shared) -> Result<Status, String> {
    let cancel = signal_cancel(shared)?;
    finish_cancel(shared, &cancel)
}

#[tauri::command]
pub async fn subscription_cancel(state: tauri::State<'_, PrivacyState>) -> Result<Status, String> {
    let shared = state.0.clone();
    let cancel = signal_cancel(&shared)?;
    tauri::async_runtime::spawn_blocking(move || finish_cancel(&shared, &cancel))
        .await
        .map_err(|_| unavailable())?
}

fn begin_login(shared: &Shared) -> Result<Pending, String> {
    cancel_active(shared);
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
    core.dispatch(Request::Cancel {})?;
    if c.state == "connecting" {
        return Err("ログインを待っています。中止してからやり直してください。".into());
    }
    let pending = c.next_pending(shared)?;
    c.mode = "chatgpt_plan".into();
    c.auto_connect = true;
    c.state = "connecting".into();
    c.reason = None;
    core.bind_delivery_config(&c.binding())?;
    Ok(pending)
}

fn finish_login_with<C>(
    shared: &Shared,
    pending: Pending,
    outcome: Result<Session, String>,
    commit: C,
) -> Result<Status, String>
where
    C: FnOnce(&mut Session, &AtomicBool) -> Result<(), String>,
{
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
    if !c.current(&pending) {
        return Ok(c.status());
    }
    match outcome {
        Ok(mut session) => {
            if !session.status().connected {
                c.fail(NO_MODEL.into());
            } else if let Err(reason) = commit(&mut session, &pending.cancel) {
                c.fail(reason);
            } else if !c.current(&pending) {
                return Ok(c.status());
            } else {
                c.selected_model = session.models().first().map(|m| m.slug.clone());
                c.session = Some(session);
                c.loaded = true;
                c.state = "connected".into();
                c.reason = None;
            }
        }
        Err(reason) => c.fail(reason),
    }
    cancel_active(shared);
    core.bind_delivery_config(&c.binding())?;
    Ok(c.status())
}

#[tauri::command]
pub async fn subscription_sign_in(state: tauri::State<'_, PrivacyState>) -> Result<Status, String> {
    let shared = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let pending = begin_login(&shared)?;
        // OAuth and catalog I/O finish in memory. Only the current result commits.
        let outcome = subscription_auth::sign_in(pending.cancel.clone()).and_then(|mut session| {
            session.ensure_access()?;
            if pending.cancel.load(Ordering::SeqCst) {
                return Err("ログインを中止しました。".into());
            }
            session.fetch_models()?;
            Ok(session)
        });
        finish_login_with(&shared, pending, outcome, Session::commit)
    })
    .await
    .map_err(|_| unavailable())?
}

pub fn send_selected(shared: &Shared, approval_id: &str) -> Result<Snapshot, String> {
    send_selected_with_probe(
        shared,
        approval_id,
        live_worker_fingerprint,
        |provider, text| {
            let key = provider::key(provider)?;
            provider::exchange(provider, text, &key)
        },
        |model, text, access| {
            subscription_transport::exchange_classified(model, text, access)
                .map_err(|stop| stop.code().to_owned())
        },
    )
}

fn typed_plan_failure(code: &str) -> Option<NativeDeliveryFailure> {
    use subscription_transport::Stop;
    Some(match Stop::from_code(code)? {
        Stop::Network => NativeDeliveryFailure::Network,
        Stop::HttpDenied => NativeDeliveryFailure::HttpDenied,
        Stop::HttpLimit => NativeDeliveryFailure::HttpLimit,
        Stop::UsageUnavailable => NativeDeliveryFailure::UsageUnavailable,
        Stop::HttpOther => NativeDeliveryFailure::HttpOther,
        Stop::HttpNoContent => NativeDeliveryFailure::HttpNoContent,
        Stop::EmptyBody => NativeDeliveryFailure::EmptyBody,
        Stop::ContentType => NativeDeliveryFailure::ContentType,
        Stop::ContentTypeJson => NativeDeliveryFailure::ContentTypeJson,
        Stop::ContentTypeHtml => NativeDeliveryFailure::ContentTypeHtml,
        Stop::ContentTypeMissing => NativeDeliveryFailure::ContentTypeMissing,
        Stop::ContentTypeOther => NativeDeliveryFailure::ContentTypeOther,
        Stop::Truncated => NativeDeliveryFailure::Truncated,
        Stop::Json => NativeDeliveryFailure::Json,
        Stop::EventOrder => NativeDeliveryFailure::EventOrder,
        Stop::UnknownEvent => NativeDeliveryFailure::UnknownEvent,
        Stop::UnexpectedOutput => NativeDeliveryFailure::UnexpectedOutput,
        Stop::Refusal => NativeDeliveryFailure::Refusal,
        Stop::UnsupportedItem => NativeDeliveryFailure::UnsupportedItem,
        Stop::UnsupportedPart => NativeDeliveryFailure::UnsupportedPart,
        Stop::UnexpectedRole => NativeDeliveryFailure::UnexpectedRole,
        Stop::IncompleteMessage => NativeDeliveryFailure::IncompleteMessage,
        Stop::EmptyAnswer => NativeDeliveryFailure::EmptyAnswer,
        Stop::MalformedOutput => NativeDeliveryFailure::MalformedOutput,
        Stop::ResponseIdentity => NativeDeliveryFailure::ResponseIdentity,
        Stop::ByteLimit => NativeDeliveryFailure::ByteLimit,
        Stop::TextLimit => NativeDeliveryFailure::TextLimit,
        Stop::Partial => NativeDeliveryFailure::Partial,
    })
}

fn halt_delivery(core: &mut Core, failure: NativeDeliveryFailure) -> String {
    core.stop_native_delivery(failure)
        .blocked_reason
        .unwrap_or_else(unavailable)
}

fn send_selected_with_probe<P, A, S>(
    shared: &Shared,
    approval_id: &str,
    probe: P,
    api: A,
    plan: S,
) -> Result<Snapshot, String>
where
    P: FnOnce(&Shared) -> Result<String, String>,
    A: FnOnce(&str, &str) -> Result<String, String>,
    S: FnOnce(&str, &str, &str) -> Result<String, String>,
{
    send_selected_with_refresh(
        shared,
        approval_id,
        probe,
        Session::ensure_access,
        api,
        plan,
    )
}

fn send_selected_with_refresh<P, R, A, S>(
    shared: &Shared,
    approval_id: &str,
    probe: P,
    refresh: R,
    api: A,
    plan: S,
) -> Result<Snapshot, String>
where
    P: FnOnce(&Shared) -> Result<String, String>,
    R: FnOnce(&mut Session) -> Result<(), String>,
    A: FnOnce(&str, &str) -> Result<String, String>,
    S: FnOnce(&str, &str, &str) -> Result<String, String>,
{
    let mut core = shared.core.lock().map_err(|_| unavailable())?;
    let snapshot = core.snapshot();
    let mut c = shared.subscription.lock().map_err(|_| unavailable())?;
    core.bind_delivery_config(&c.binding())?;
    let answer = if c.mode == "chatgpt_plan" {
        if snapshot.destination != "ChatGPT" || c.state != "connected" || !c.auto_connect {
            return Err(halt_delivery(
                &mut core,
                NativeDeliveryFailure::Disconnected,
            ));
        }
        let delivery_cancel = c.cancel.clone();
        let Some(model) = c.selected_model.clone() else {
            return Err(halt_delivery(&mut core, NativeDeliveryFailure::Unavailable));
        };
        let Some(session) = c.session.as_mut() else {
            return Err(halt_delivery(
                &mut core,
                NativeDeliveryFailure::Disconnected,
            ));
        };
        if delivery_cancel.load(Ordering::SeqCst) {
            return Err(halt_delivery(&mut core, NativeDeliveryFailure::Cancelled));
        }
        if !session.models().iter().any(|m| m.slug == model) {
            return Err(halt_delivery(&mut core, NativeDeliveryFailure::Unavailable));
        }
        if refresh(session).is_err() {
            let reason = halt_delivery(&mut core, NativeDeliveryFailure::Authentication);
            c.fail(reason.clone());
            return Err(reason);
        }
        if !session.status().connected || !session.models().iter().any(|m| m.slug == model) {
            return Err(halt_delivery(
                &mut core,
                NativeDeliveryFailure::Authentication,
            ));
        }
        let access = zeroize::Zeroizing::new(session.access_token().to_owned());
        core.bind_delivery_config(&c.binding())?;
        drop(c);
        // Renewal may block and the child can die independently of Core's lock.
        // Check runtime liveness after authentication, immediately before export.
        if verify_current_model_with(shared, &mut core, probe).is_err() {
            return Err(halt_delivery(
                &mut core,
                NativeDeliveryFailure::ModelUnavailable,
            ));
        }
        if delivery_cancel.load(Ordering::SeqCst) {
            return Err(halt_delivery(&mut core, NativeDeliveryFailure::Cancelled));
        }
        let failure = Cell::new(None);
        let exported = core.export_approved_with(approval_id, |text| {
            if delivery_cancel.load(Ordering::SeqCst) {
                failure.set(Some(NativeDeliveryFailure::Cancelled));
                return Err(DISCONNECTED.into());
            }
            plan(&model, text, &access).map_err(|code| {
                failure.set(typed_plan_failure(&code));
                code
            })
        });
        match exported {
            Ok(answer) => answer,
            Err(reason) => {
                if let Some(failure) = failure.get() {
                    return Err(halt_delivery(&mut core, failure));
                }
                return Err(reason);
            }
        }
    } else {
        let provider = match snapshot.destination.as_str() {
            "ChatGPT" => "OpenAI",
            "Claude" => "Claude",
            _ => {
                return Err(halt_delivery(&mut core, NativeDeliveryFailure::Unavailable));
            }
        };
        drop(c);
        if verify_current_model_with(shared, &mut core, probe).is_err() {
            return Err(halt_delivery(
                &mut core,
                NativeDeliveryFailure::ModelUnavailable,
            ));
        }
        core.export_approved_with(approval_id, |text| api(provider, text))?
    };
    core.dispatch(Request::RestoreIfCurrent {
        answer,
        case_id: snapshot.case_id,
        revision: snapshot.revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cushion_core::{Patch, Phase};
    use std::cell::Cell;
    use std::sync::{atomic::AtomicUsize, mpsc, Barrier};
    use std::time::Duration;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn shared_connection(connection: Connection) -> Arc<Shared> {
        let mut core = Core::new().unwrap();
        core.bind_delivery_config(&connection.binding()).unwrap();
        let fixture = cushion_core::synthetic_fixtures()
            .into_iter()
            .find(|f| f.id == "delivery")
            .unwrap();
        core.dispatch(Request::Update {
            patch: Patch {
                mode: Some("local".into()),
                original: Some(fixture.original),
                dictionary_text: Some(fixture.secrets.join("\n")),
                ..Patch::default()
            },
        })
        .unwrap();
        let job = core.begin_local_scan(HASH, true).unwrap();
        core.finish_local_scan(&job.job_id, &serde_json::json!({"schema_version":"1.0", "document_id":job.document_id, "complete":true, "covered_chars":job.original.chars().count(), "findings":[], "context_flags":[], "unresolved":[]}).to_string()).unwrap();
        core.dispatch(Request::Approve {
            revision: core.snapshot().revision,
        })
        .unwrap();
        Arc::new(Shared {
            generation: AtomicU64::new(1),
            core: Mutex::new(core),
            worker: Mutex::new(None),
            model: Mutex::new(ModelStatus::ready(HASH)),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            vault: Mutex::new(Vault::new(std::path::PathBuf::from(
                "/tmp/cushion-synthetic-no-io-tests",
            ))),
            subscription_cancel: Mutex::new(connection.cancel_signal()),
            subscription: Mutex::new(connection),
        })
    }

    fn shared(plan: bool) -> Arc<Shared> {
        shared_connection(Connection::new(plan))
    }
    fn connected() -> Arc<Shared> {
        let mut c = Connection::new(true);
        c.session = Some(Session::synthetic_for_test("fixture-model"));
        c.selected_model = Some("fixture-model".into());
        c.loaded = true;
        c.state = "connected".into();
        shared_connection(c)
    }
    fn approval(s: &Shared) -> String {
        s.core
            .lock()
            .unwrap()
            .snapshot()
            .approval_id
            .clone()
            .unwrap()
    }
    fn snapshot(s: &Shared) -> serde_json::Value {
        serde_json::to_value(s.core.lock().unwrap().snapshot()).unwrap()
    }

    #[test]
    fn disconnected_plan_never_falls_back_to_api_or_sends() {
        let s = shared(true);
        assert!(send_selected_with_probe(
            &s,
            &approval(&s),
            |_| Ok(HASH.into()),
            |_, _| panic!("API fallback"),
            |_, _, _| panic!("unconnected plan send")
        )
        .is_err());
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn selecting_connection_cancels_review_and_keeps_original() {
        let s = shared(false);
        let before = s.core.lock().unwrap().snapshot();
        assert!(select_inner(&s, "untrusted", None).is_err());
        let after = s.core.lock().unwrap().snapshot();
        assert_eq!(before.original, after.original);
        assert!(after.phase == Phase::Draft && after.approval_id.is_none());
    }

    #[test]
    fn api_selected_uses_same_core_approved_only_transaction() {
        let s = shared(false);
        let before = s.core.lock().unwrap().snapshot();
        let id = before.approval_id.unwrap();
        let token = before.findings[0].replacement.clone();
        let result = send_selected_with_probe(
            &s,
            &id,
            |_| Ok(HASH.into()),
            |provider, text| {
                assert_eq!(provider, "OpenAI");
                assert!(!text.contains("こもれび商店"));
                Ok(token)
            },
            |_, _, _| panic!("plan fallback"),
        )
        .unwrap();
        assert!(result.phase == Phase::RestoredLocal && result.restored.contains("こもれび商店"));
    }

    #[test]
    fn selected_plan_exposes_only_approved_text_and_restores_locally() {
        let s = connected();
        let before = s.core.lock().unwrap().snapshot();
        let token = before.findings[0].replacement.clone();
        let result = send_selected_with_refresh(
            &s,
            &approval(&s),
            |_| Ok(HASH.into()),
            |_| Ok(()),
            |_, _| panic!("API fallback"),
            |model, text, access| {
                assert_eq!(model, "fixture-model");
                assert!(!access.is_empty());
                assert!(!text.contains("こもれび商店"));
                assert_eq!(text, before.transformed);
                Ok(token)
            },
        )
        .unwrap();
        assert!(result.phase == Phase::RestoredLocal && result.restored.contains("こもれび商店"));
    }

    #[test]
    fn actual_send_route_has_one_winner_under_concurrent_calls() {
        let s = connected();
        let id = approval(&s);
        let barrier = Arc::new(Barrier::new(3));
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                let s = s.clone();
                let id = id.clone();
                let barrier = barrier.clone();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    send_selected_with_refresh(
                        &s,
                        &id,
                        |_| Ok(HASH.into()),
                        |_| Ok(()),
                        |_, _| panic!("API fallback"),
                        |_, _, _| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok("架空の回答".into())
                        },
                    )
                    .is_ok()
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            tasks
                .into_iter()
                .map(|t| usize::from(t.join().unwrap()))
                .sum::<usize>(),
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn final_connection_binding_drift_blocks_both_transports() {
        let s = connected();
        let id = approval(&s);
        // Simulate internal connection mutation bypassing the selection handler.
        s.subscription.lock().unwrap().session = Some(Session::synthetic_for_test_binding(
            "fixture-model",
            "different-registration",
        ));
        assert!(send_selected_with_refresh(
            &s,
            &id,
            |_| Ok(HASH.into()),
            |_| Ok(()),
            |_, _| panic!("API leak"),
            |_, _, _| panic!("plan leak")
        )
        .is_err());
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn worker_probe_occurs_after_refresh_and_dead_child_blocks_send() {
        let s = connected();
        let refreshed = Cell::new(false);
        assert!(send_selected_with_refresh(
            &s,
            &approval(&s),
            |_| {
                assert!(refreshed.get());
                Err("synthetic dead child".into())
            },
            |_| {
                refreshed.set(true);
                Ok(())
            },
            |_, _| panic!("API fallback"),
            |_, _, _| panic!("dead worker send")
        )
        .is_err());
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn refresh_registration_drift_cannot_reuse_approval() {
        let s = connected();
        let id = approval(&s);
        assert!(send_selected_with_refresh(
            &s,
            &id,
            |_| Ok(HASH.into()),
            |session| {
                *session = Session::synthetic_for_test_binding("fixture-model", "new-registration");
                Ok(())
            },
            |_, _| panic!("API fallback"),
            |_, _, _| panic!("wrong registration send")
        )
        .is_err());
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn real_cancel_during_refresh_notifies_without_connection_lock_and_blocks_send() {
        let s = connected();
        let id = approval(&s);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let send = {
            let s = s.clone();
            std::thread::spawn(move || {
                send_selected_with_refresh(
                    &s,
                    &id,
                    |_| Ok(HASH.into()),
                    |_| {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                        Ok(())
                    },
                    |_, _| panic!("API fallback"),
                    |_, _, _| panic!("canceled plan send"),
                )
            })
        };
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(s.core.try_lock().is_err() && s.subscription.try_lock().is_err());
        let (signalled_tx, signalled_rx) = mpsc::channel();
        let cancel = {
            let s = s.clone();
            std::thread::spawn(move || {
                // The same capture/finish sequence used by subscription_cancel.
                let signal = signal_cancel(&s).unwrap();
                signalled_tx.send(signal.load(Ordering::SeqCst)).unwrap();
                finish_cancel(&s, &signal).unwrap()
            })
        };
        // Renewal still owns Connection: notification cannot depend on that lock.
        assert!(signalled_rx.recv_timeout(Duration::from_secs(2)).unwrap());
        release_tx.send(()).unwrap();
        assert!(send
            .join()
            .unwrap()
            .err()
            .unwrap()
            .contains("接続が停止された"));
        assert_eq!(cancel.join().unwrap().state, "disconnected");
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
        assert!(!s.core.lock().unwrap().snapshot().original.is_empty());
    }

    #[test]
    fn stale_cancel_cannot_revoke_new_generation_approval_or_send() {
        let s = connected();
        let stale = signal_cancel(&s).unwrap();
        let selected = select_inner_with(&s, "chatgpt_plan", Some("fixture-model"), |_, _| {
            panic!("already prepared session")
        })
        .unwrap();
        assert_eq!(selected.state, "connected");
        let current = s.subscription_cancel.lock().unwrap().clone();
        assert!(!Arc::ptr_eq(&stale, &current));
        assert!(!current.load(Ordering::SeqCst));
        let (id, token) = {
            let mut core = s.core.lock().unwrap();
            let job = core.begin_local_scan(HASH, true).unwrap();
            core.finish_local_scan(&job.job_id, &serde_json::json!({"schema_version":"1.0", "document_id":job.document_id, "complete":true, "covered_chars":job.original.chars().count(), "findings":[], "context_flags":[], "unresolved":[]}).to_string()).unwrap();
            let token = core.snapshot().findings[0].replacement.clone();
            let revision = core.snapshot().revision;
            let id = core
                .dispatch(Request::Approve { revision })
                .unwrap()
                .approval_id
                .unwrap();
            (id, token)
        };
        let before = snapshot(&s);
        assert_eq!(finish_cancel(&s, &stale).unwrap().state, "connected");
        assert_eq!(snapshot(&s), before);
        assert!(!current.load(Ordering::SeqCst));
        let calls = Cell::new(0);
        let result = send_selected_with_refresh(
            &s,
            &id,
            |_| Ok(HASH.into()),
            |_| Ok(()),
            |_, _| panic!("API fallback"),
            |_, text, _| {
                assert!(text.contains(&token));
                calls.set(calls.get() + 1);
                Ok(token)
            },
        )
        .unwrap();
        assert!(result.phase == Phase::RestoredLocal);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn real_cancel_reaches_login_commit_while_connection_is_locked() {
        let s = shared(false);
        let pending = begin_login(&s).unwrap();
        let committed = Cell::new(false);
        let result = finish_login_with(
            &s,
            pending,
            Ok(Session::synthetic_for_test("late-model")),
            |_, cancel| {
                assert!(s.core.try_lock().is_err() && s.subscription.try_lock().is_err());
                let signal = signal_cancel(&s).unwrap();
                assert!(Arc::ptr_eq(&signal, &s.subscription_cancel.lock().unwrap()));
                if cancel.load(Ordering::SeqCst) {
                    return Err("ログインを中止しました。".into());
                }
                committed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result.state, "error");
        assert!(!committed.get());
        assert!(s.subscription.lock().unwrap().session.is_none());
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn status_poll_cannot_supersede_a_signalled_cancel() {
        let s = shared(true);
        let cancel = {
            let _core = s.core.lock().unwrap();
            let mut c = s.subscription.lock().unwrap();
            // Freeze the actual poll between its initial check and publication.
            assert!(!c.cancel.load(Ordering::SeqCst));
            let cancel = signal_cancel(&s).unwrap();
            assert!(c.next_implicit_pending(&s).unwrap().is_none());
            assert!(Arc::ptr_eq(&cancel, &c.cancel));
            cancel
        };
        let status = status_inner_with(&s, |_, _| panic!("poll superseded user cancel")).unwrap();
        assert_eq!(status.state, "disconnected");
        assert!(Arc::ptr_eq(&cancel, &s.subscription_cancel.lock().unwrap()));
        assert_eq!(finish_cancel(&s, &cancel).unwrap().state, "disconnected");
        assert!(!s.subscription.lock().unwrap().auto_connect);
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn startup_core_lock_returns_waiting_status_then_prepares_once_after_release() {
        let s = shared(true);
        let core = s.core.lock().unwrap();
        let waiting =
            status_inner_with(&s, |_, _| panic!("prepare while local startup owns Core")).unwrap();
        assert_eq!(waiting.state, "connecting");
        assert!(waiting.reason.unwrap().contains("このPCの準備"));
        {
            let c = s.subscription.lock().unwrap();
            assert_eq!(c.state, "disconnected");
            assert!(!c.loaded && c.generation == 0 && c.session.is_none());
            assert!(!c.cancel.load(Ordering::SeqCst));
        }
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("duplicate startup preparation"))
                .unwrap()
                .state,
            "connecting"
        );
        drop(core);
        let calls = Cell::new(0);
        let ready = status_inner_with(&s, |session, cancel| {
            assert!(session.is_none() && !cancel.load(Ordering::SeqCst));
            calls.set(calls.get() + 1);
            Ok(Some(Session::synthetic_for_test("startup-model")))
        })
        .unwrap();
        assert_eq!(ready.state, "connected");
        assert_eq!(calls.get(), 1);
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("prepared session loaded twice"))
                .unwrap()
                .state,
            "connected"
        );
    }

    #[test]
    fn startup_waiting_status_stops_polling_after_explicit_cancel_without_preparation() {
        let s = shared(true);
        let core = s.core.lock().unwrap();
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("prepare while Core is held"))
                .unwrap()
                .state,
            "connecting"
        );
        let cancel = signal_cancel(&s).unwrap();
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("signalled cancel started preparation"))
                .unwrap()
                .state,
            "disconnected"
        );
        assert!(!s.subscription.lock().unwrap().loaded);
        drop(core);
        assert_eq!(finish_cancel(&s, &cancel).unwrap().state, "disconnected");
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("explicit cancel auto-reconnected"))
                .unwrap()
                .state,
            "disconnected"
        );
        assert!(s.subscription.lock().unwrap().session.is_none());
    }

    #[test]
    fn startup_core_lock_preserves_api_and_existing_error_statuses() {
        let s = shared(false);
        let core = s.core.lock().unwrap();
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("API status prepared plan"))
                .unwrap()
                .state,
            "disconnected"
        );
        drop(core);
        let s = shared(true);
        s.subscription.lock().unwrap().fail(NO_MODEL.into());
        let _core = s.core.lock().unwrap();
        let status = status_inner_with(&s, |_, _| panic!("error state auto-prepared")).unwrap();
        assert_eq!(status.state, "error");
        assert_eq!(status.reason.as_deref(), Some(NO_MODEL));
    }

    #[test]
    fn cached_preparation_releases_locks_and_cancel_rejects_late_success() {
        let s = shared(true);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = {
            let s = s.clone();
            std::thread::spawn(move || {
                status_inner_with(&s, |_, _| {
                    assert!(s.core.try_lock().is_ok() && s.subscription.try_lock().is_ok());
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(Some(Session::synthetic_for_test("late-model")))
                })
                .unwrap()
            })
        };
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(cancel_inner(&s).unwrap().state, "disconnected");
        s.core
            .lock()
            .unwrap()
            .dispatch(Request::Update {
                patch: Patch {
                    purpose: Some("summary".into()),
                    ..Patch::default()
                },
            })
            .unwrap();
        let before = snapshot(&s);
        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap().state, "disconnected");
        assert_eq!(snapshot(&s), before);
        assert!(s.subscription.lock().unwrap().session.is_none());
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("cancel must not reconnect"))
                .unwrap()
                .state,
            "disconnected"
        );
    }

    #[test]
    fn canceled_login_drops_memory_without_poll_reconnecting_or_committing() {
        let s = connected();
        let pending = begin_login(&s).unwrap();
        cancel_inner(&s).unwrap();
        let result = finish_login_with(
            &s,
            pending,
            Ok(Session::synthetic_for_test("late-model")),
            |_, _| panic!("stale login persisted"),
        )
        .unwrap();
        assert_eq!(result.state, "disconnected");
        assert!(result.model.is_none() && result.models.is_empty());
        assert!(s.subscription.lock().unwrap().session.is_none());
        assert_eq!(
            status_inner_with(&s, |_, _| panic!("canceled session auto-reconnected"))
                .unwrap()
                .state,
            "disconnected"
        );
        assert_eq!(
            select_inner_with(&s, "chatgpt_plan", None, |session, _| {
                assert!(session.is_none());
                Ok(Some(Session::synthetic_for_test(
                    "explicit-reconnected-model",
                )))
            })
            .unwrap()
            .state,
            "connected"
        );
    }

    #[test]
    fn superseded_login_result_never_commits_or_changes_selected_mode() {
        let s = shared(false);
        let pending = begin_login(&s).unwrap();
        select_inner(&s, "api_key", None).unwrap();
        let before = snapshot(&s);
        let result = finish_login_with(
            &s,
            pending,
            Ok(Session::synthetic_for_test("late-model")),
            |_, _| panic!("superseded login persisted"),
        )
        .unwrap();
        assert_eq!(result.delivery_mode, "api_key");
        assert_eq!(snapshot(&s), before);
    }

    #[test]
    fn current_login_commits_after_catalog_validation_and_binds_new_registration() {
        let s = shared(false);
        let pending = begin_login(&s).unwrap();
        let committed = Cell::new(false);
        let result = finish_login_with(
            &s,
            pending,
            Ok(Session::synthetic_for_test("new-model")),
            |session, cancel| {
                assert!(!cancel.load(Ordering::SeqCst));
                assert!(session.status().connected);
                committed.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(committed.get());
        assert_eq!(result.state, "connected");
        assert_eq!(result.model.as_deref(), Some("new-model"));
        assert!(s.core.lock().unwrap().snapshot().approval_id.is_none());
    }

    #[test]
    fn classified_plan_failure_records_fixed_same_context_core_stop() {
        use subscription_transport::Stop;
        for (stop, expected) in [
            (Stop::HttpDenied, "認証または利用権限"),
            (Stop::HttpLimit, "利用上限"),
            (Stop::UsageUnavailable, "利用可能枠を確認できません"),
            (Stop::HttpNoContent, "HTTP 204/205"),
            (Stop::EmptyBody, "応答本文が空"),
            (Stop::ContentTypeJson, "受信形式（JSON）"),
            (Stop::ContentTypeHtml, "ページ形式（HTML）"),
            (Stop::ContentTypeMissing, "受信形式の指定がなく"),
            (Stop::ContentTypeOther, "対応していない受信形式"),
            (Stop::UnknownEvent, "対応していない応答イベント"),
            (Stop::Refusal, "拒否を示す通知"),
            (Stop::UnsupportedItem, "対応していない出力項目"),
            (Stop::UnsupportedPart, "対応していない本文要素"),
            (Stop::UnexpectedRole, "回答の発言者"),
            (Stop::IncompleteMessage, "本文の完了状態"),
            (Stop::EmptyAnswer, "回答の本文が見つかりません"),
            (Stop::MalformedOutput, "必要な配列や文字列の形式"),
            (Stop::Truncated, "回答の途中で受信が終わり"),
            (Stop::Partial, "未完了・拒否・失敗"),
        ] {
            let s = connected();
            let before = s.core.lock().unwrap().snapshot();
            let id = approval(&s);
            let calls = Cell::new(0);
            let error = send_selected_with_refresh(
                &s,
                &id,
                |_| Ok(HASH.into()),
                |_| Ok(()),
                |_, _| panic!("API fallback"),
                |_, _, _| {
                    calls.set(calls.get() + 1);
                    Err(stop.code().to_owned())
                },
            )
            .err()
            .unwrap();
            let after = s.core.lock().unwrap().snapshot();
            assert_eq!(calls.get(), 1);
            assert_eq!(after.case_id, before.case_id);
            assert_eq!(after.revision, before.revision);
            assert_eq!(after.original, before.original);
            assert!(after.phase == Phase::Blocked && after.approval_id.is_none());
            assert_eq!(after.blocked_reason.as_deref(), Some(error.as_str()));
            assert!(error.contains(expected) && !error.contains(&before.original));
            assert!(error.contains("自動再送はしません"));
            assert!(send_selected_with_refresh(
                &s,
                &id,
                |_| Ok(HASH.into()),
                |_| Ok(()),
                |_, _| panic!("API fallback"),
                |_, _, _| panic!("old approval replay")
            )
            .is_err());
            assert_eq!(calls.get(), 1);
        }
    }

    #[test]
    fn authentication_failure_sets_fixed_core_reason_before_any_transport() {
        let s = connected();
        let before = s.core.lock().unwrap().snapshot();
        let id = approval(&s);
        let error = send_selected_with_refresh(
            &s,
            &id,
            |_| panic!("probe follows refresh"),
            |_| Err("synthetic private auth exception".into()),
            |_, _| panic!("API fallback"),
            |_, _, _| panic!("authentication failed before send"),
        )
        .err()
        .unwrap();
        let after = s.core.lock().unwrap().snapshot();
        assert!(after.phase == Phase::Blocked && after.approval_id.is_none());
        assert_eq!(after.case_id, before.case_id);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.original, before.original);
        assert_eq!(after.blocked_reason.as_deref(), Some(error.as_str()));
        assert!(error.contains("認証を更新または確認できませんでした"));
        assert!(!error.contains("synthetic private auth exception"));
        let status = serde_json::to_string(&s.subscription.lock().unwrap().status()).unwrap();
        assert!(!status.contains("synthetic private auth exception"));
    }
}
