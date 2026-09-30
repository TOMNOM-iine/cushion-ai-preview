//! Official Sign in with ChatGPT public-client flow. No application text enters this module.
//! Tokens never leave native memory except for the fixed OAuth/API origins and this module's
//! isolated, non-synchronizing Keychain record. No cookie/session reuse or browser automation.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const SERVICE: &str = "local.cushion.chatgpt-plan.v1";
const ACCOUNT: &str = "host-and-registration";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const JWKS: &str = "https://auth.openai.com/.well-known/jwks.json";
const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const MODELS: &str = "https://api.openai.com/v1/models";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const REQUIRED_SCOPES: [&str; 6] = [
    "openid",
    "profile",
    "email",
    "offline_access",
    "resource.invoke",
    "chatgpt.tokens.use.direct",
];
const MAX_AUTH_BODY: usize = 131_072;
const MAX_CATALOG_BODY: usize = 1_048_576;
const MAX_CALLBACK_HEADERS: usize = 16_384;
const DEADLINE: Duration = Duration::from_secs(600);
const AUTH_ERROR: &str =
    "ChatGPTの認証を検証できませんでした。接続を停止しました。もう一度サインインしてください。";
const KEYCHAIN_ERROR: &str =
    "ChatGPT接続をこのMacのキーチェーンで扱えません。平文では保存しません。";
const CANCELLED: &str = "ChatGPTへのサインインを取り消しました。";
const REFRESH_ERROR: &str =
    "ChatGPTの認証を更新できませんでした。接続を停止しました。もう一度サインインしてください。";
const CATALOG_ERROR: &str = "このChatGPT接続の利用可能モデルを確認できません。送信を停止しました。";
// The in-process mutex and a zero-byte, user-private OS lock serialize rotating refreshes.
// Neither the lock filename nor its contents hold credentials or application data.
static AUTH_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanModel {
    pub slug: String,
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStatus {
    pub connected: bool,
    pub models: Vec<PlanModel>,
}

// No Debug/Clone implementations: a diagnostic cannot accidentally format credentials.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    #[serde(with = "secret_serde")]
    access_token: Zeroizing<String>,
    #[serde(with = "secret_serde")]
    refresh_token: Zeroizing<String>,
    #[serde(with = "secret_serde")]
    id_token: Zeroizing<String>,
    scopes: Vec<String>,
    expires_at: u64,
    saved_at: u64,
    nonce: String,
    epoch: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    client_id: String,
    subject: String,
    credentials: Credentials,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    host_id: String,
    registration: Option<Registration>,
}

pub struct Session {
    record: Record,
    catalog: Vec<PlanModel>,
    usable: bool,
    persisted: bool,
    expected_binding: Option<String>,
}

impl Session {
    /// Reads only this module's isolated Keychain item; no network or other credentials.
    /// A loaded session does not become connected until fetch_models succeeds.
    pub fn load() -> Result<Option<Self>, String> {
        let _guard = AUTH_LOCK.lock().map_err(|_| KEYCHAIN_ERROR)?;
        let Some(record) = load_record()? else {
            return Ok(None);
        };
        validate_record(&record)?;
        if record.registration.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            record,
            catalog: Vec::new(),
            usable: true,
            persisted: true,
            expected_binding: None,
        }))
    }

    pub fn status(&self) -> PlanStatus {
        PlanStatus {
            connected: self.usable && !self.catalog.is_empty(),
            models: if self.usable {
                self.catalog.clone()
            } else {
                Vec::new()
            },
        }
    }

    pub fn models(&self) -> &[PlanModel] {
        &self.catalog
    }

    /// The host calls this only after checking the pending generation/cancel flag under its
    /// connection lock. A canceled or superseded OAuth result must never be persisted.
    pub fn commit(&mut self, cancel: &AtomicBool) -> Result<(), String> {
        check_cancel(cancel)?;
        if !self.status().connected || self.persisted {
            return Err(AUTH_ERROR.into());
        }
        let _guard = AUTH_LOCK.lock().map_err(|_| AUTH_ERROR)?;
        let _process = process_lock(Some(cancel))?;
        check_cancel(cancel)?;
        let previous = load_record()?.ok_or(KEYCHAIN_ERROR)?;
        validate_record(&previous)?;
        validate_commit(
            &previous,
            &self.record,
            self.expected_binding.as_deref(),
            cancel,
        )?;
        check_cancel(cancel)?;
        save_record(&self.record)?;
        self.persisted = true;
        self.expected_binding = None;
        Ok(())
    }

    /// Native transport only. UI must never receive this return value.
    pub fn access_token(&self) -> &str {
        if !self.usable {
            return "";
        }
        self.record
            .registration
            .as_ref()
            .map(|r| r.credentials.access_token.as_str())
            .unwrap_or("")
    }

    /// This opaque binding has no token bytes. Refresh keeps it; reauthorization rotates it.
    pub fn binding(&self) -> String {
        record_binding(&self.record).unwrap_or_default()
    }

    pub fn ensure_access(&mut self) -> Result<(), String> {
        if !self.usable {
            return Err(REFRESH_ERROR.into());
        }
        let result = self.refresh_if_needed();
        if result.is_err() {
            self.usable = false;
            self.catalog.clear();
        }
        result
    }

    fn refresh_if_needed(&mut self) -> Result<(), String> {
        let checked_at = now()?;
        if !self.persisted {
            let credentials = &self
                .record
                .registration
                .as_ref()
                .ok_or(REFRESH_ERROR)?
                .credentials;
            return if credentials.expires_at > checked_at.saturating_add(60) {
                Ok(())
            } else {
                Err(REFRESH_ERROR.into())
            };
        }
        let _guard = AUTH_LOCK.lock().map_err(|_| REFRESH_ERROR)?;
        let _process = process_lock(None)?;
        let current = load_record()?.ok_or(REFRESH_ERROR)?;
        validate_record(&current)?;
        let expected = self.record.registration.as_ref().ok_or(REFRESH_ERROR)?;
        let latest = current.registration.as_ref().ok_or(REFRESH_ERROR)?;
        if !same_registration(expected, latest) || current.host_id != self.record.host_id {
            return Err(REFRESH_ERROR.into());
        }
        // A previous window may already have rotated the token. Adopt that complete atomic
        // record, never combine its token with this window's client/account metadata.
        self.record = current;
        let r = self.record.registration.as_ref().ok_or(REFRESH_ERROR)?;
        if r.credentials.expires_at > checked_at.saturating_add(60) {
            return Ok(());
        }
        let client = http_client()?;
        let response = client
            .post(TOKEN)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", r.client_id.as_str()),
                ("refresh_token", r.credentials.refresh_token.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .map_err(|_| REFRESH_ERROR)?;
        let token = parse_token_response(response, REFRESH_ERROR)?;
        let received_at = now()?;
        let jwks = fetch_jwks(&client)?;
        let credentials = replacement_credentials(token, &jwks, r, received_at, now()?)?;
        let updated = Record {
            version: 1,
            host_id: self.record.host_id.clone(),
            registration: Some(Registration {
                client_id: r.client_id.clone(),
                subject: r.subject.clone(),
                credentials,
            }),
        };
        // SecItemUpdate replaces all rotating credentials and metadata together.
        save_record(&updated)?;
        self.record = updated;
        Ok(())
    }

    pub fn fetch_models(&mut self) -> Result<&[PlanModel], String> {
        self.ensure_access()?;
        let result = (|| {
            let response = http_client()?
                .get(MODELS)
                .bearer_auth(self.access_token())
                .send()
                .map_err(|_| CATALOG_ERROR)?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(CATALOG_ERROR.into());
            }
            let bytes = bounded_body(response, MAX_CATALOG_BODY, CATALOG_ERROR)?;
            parse_models(&bytes)
        })();
        match result {
            Ok(models) => {
                self.catalog = models;
                Ok(&self.catalog)
            }
            Err(error) => {
                self.usable = false;
                self.catalog.clear();
                Err(error)
            }
        }
    }
}

/// Called only after the user's explicit Continue with ChatGPT action.
/// The listener never accepts a body or any application document; it dies after this attempt.
pub fn sign_in(cancel: Arc<AtomicBool>) -> Result<Session, String> {
    check_cancel(&cancel)?;
    let mut record = {
        let _guard = AUTH_LOCK
            .try_lock()
            .map_err(|_| "別のChatGPT認証処理が進行中です。".to_string())?;
        let _process = process_lock(Some(&cancel))?;
        match load_record()? {
            Some(record) => {
                validate_record(&record)?;
                record
            }
            None => {
                let record = Record {
                    version: 1,
                    host_id: host_id()?,
                    registration: None,
                };
                check_cancel(&cancel)?;
                save_record(&record)?;
                record
            }
        }
    };
    let expected_binding = record_binding(&record);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| AUTH_ERROR)?;
    listener.set_nonblocking(true).map_err(|_| AUTH_ERROR)?;
    let port = listener.local_addr().map_err(|_| AUTH_ERROR)?.port();
    let pending = Pending::new(
        port,
        record.registration.as_ref().map(|r| r.client_id.as_str()),
    )?;
    let url = pending.authorization_url(&record.host_id)?;
    check_cancel(&cancel)?;
    open_system_browser(&url)?;
    let callback = wait_callback(&listener, &pending, &cancel)?;
    // Drop the only HTTP listener before exchanging the one-use authorization code.
    drop(listener);
    check_cancel(&cancel)?;
    let client = http_client()?;
    let response = client
        .post(TOKEN)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", callback.client_id.as_str()),
            ("code", callback.code.as_str()),
            ("code_verifier", pending.verifier.as_str()),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("resource", RESOURCE),
        ])
        .send()
        .map_err(|_| AUTH_ERROR)?;
    let token = parse_token_response(response, AUTH_ERROR)?;
    let received_at = now()?;
    check_cancel(&cancel)?;
    let jwks = fetch_jwks(&client)?;
    let saved_at = now()?;
    let claims = verify_identity(
        &token.id_token,
        &jwks,
        &callback.client_id,
        Some(&pending.nonce),
        saved_at,
    )?;
    if record
        .registration
        .as_ref()
        .is_some_and(|r| r.subject != claims.sub || r.client_id != callback.client_id)
    {
        return Err(AUTH_ERROR.into());
    }
    let credentials = credentials_from_response(token, &pending.nonce, &random_32()?, received_at)?;
    record.registration = Some(Registration {
        client_id: callback.client_id,
        subject: claims.sub,
        credentials,
    });
    check_cancel(&cancel)?;
    // Tokens remain in native memory until the host accepts the current generation and
    // model catalog. A late canceled result cannot silently replace the Keychain session.
    Ok(Session {
        record,
        catalog: Vec::new(),
        usable: true,
        persisted: false,
        expected_binding,
    })
}

struct Pending {
    state: Zeroizing<String>,
    nonce: Zeroizing<String>,
    verifier: Zeroizing<String>,
    client_id: String,
    redirect_uri: String,
    port: u16,
    started: Instant,
}

impl Pending {
    fn new(port: u16, returning_client: Option<&str>) -> Result<Self, String> {
        Ok(Self {
            state: Zeroizing::new(random_32()?),
            nonce: Zeroizing::new(random_32()?),
            verifier: Zeroizing::new(random_32()?),
            client_id: returning_client.unwrap_or(DYNAMIC_CLIENT).to_string(),
            redirect_uri: format!("http://127.0.0.1:{port}/auth/callback"),
            port,
            started: Instant::now(),
        })
    }

    fn authorization_url(&self, host: &str) -> Result<reqwest::Url, String> {
        let mut url = reqwest::Url::parse(AUTHORIZE).map_err(|_| AUTH_ERROR)?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        {
            let mut query = url.query_pairs_mut();
            query.extend_pairs([
                ("client_id", self.client_id.as_str()),
                ("ext_agent_host_id", host),
                ("response_type", "code"),
                ("redirect_uri", self.redirect_uri.as_str()),
                ("scope", SCOPES),
                ("resource", RESOURCE),
                ("state", self.state.as_str()),
                ("nonce", self.nonce.as_str()),
                ("code_challenge_method", "S256"),
                ("code_challenge", challenge.as_str()),
            ]);
            if self.client_id == DYNAMIC_CLIENT {
                query.append_pair("agent_name_hint", "クッションAI");
            }
        }
        Ok(url)
    }

    fn evaluate(&self, query: BTreeMap<String, Zeroizing<String>>) -> CallbackDecision {
        let Some(state) = query.get("state") else {
            return CallbackDecision::Reject;
        };
        if !bool::from(self.state.as_bytes().ct_eq(state.as_bytes())) {
            return CallbackDecision::Reject;
        }
        // A matching state makes this the terminal callback, including an OAuth denial.
        if query.contains_key("error") {
            return CallbackDecision::Complete(Err(
                "ChatGPTへの接続は許可されませんでした。".into()
            ));
        }
        let Some(code) = query.get("code") else {
            return CallbackDecision::Complete(Err(AUTH_ERROR.into()));
        };
        if !opaque(code, 4096) {
            return CallbackDecision::Complete(Err(AUTH_ERROR.into()));
        }
        let client_id = if self.client_id == DYNAMIC_CLIENT {
            match query.get("client_id") {
                Some(value) if issued_client(value) => value.to_string(),
                _ => return CallbackDecision::Complete(Err(AUTH_ERROR.into())),
            }
        } else {
            if query
                .get("client_id")
                .is_some_and(|value| value.as_str() != self.client_id)
            {
                return CallbackDecision::Complete(Err(AUTH_ERROR.into()));
            }
            self.client_id.clone()
        };
        CallbackDecision::Complete(Ok(Callback {
            code: Zeroizing::new(code.to_string()),
            client_id,
        }))
    }
}

struct Callback {
    code: Zeroizing<String>,
    client_id: String,
}
enum CallbackDecision {
    Reject,
    Complete(Result<Callback, String>),
}

fn wait_callback(
    listener: &TcpListener,
    pending: &Pending,
    cancel: &AtomicBool,
) -> Result<Callback, String> {
    loop {
        check_cancel(cancel)?;
        if pending.started.elapsed() >= DEADLINE {
            return Err(
                "ChatGPTへのサインインが時間切れになりました。もう一度開始してください。".into(),
            );
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if peer.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) {
                    continue;
                }
                let decision = read_callback(&mut stream, pending, cancel)
                    .map(|query| pending.evaluate(query));
                match decision {
                    Ok(CallbackDecision::Complete(result)) => {
                        callback_page(&mut stream, result.is_ok());
                        return result;
                    }
                    _ => callback_page(&mut stream, false),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(40))
            }
            Err(_) => return Err(AUTH_ERROR.into()),
        }
    }
}

fn read_callback(
    stream: &mut TcpStream,
    pending: &Pending,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Zeroizing<String>>, String> {
    stream.set_nonblocking(true).map_err(|_| AUTH_ERROR)?;
    let request_deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Zeroizing::new(Vec::new());
    let mut chunk = [0u8; 1024];
    loop {
        check_cancel(cancel)?;
        if Instant::now() >= request_deadline || pending.started.elapsed() >= DEADLINE {
            return Err(AUTH_ERROR.into());
        }
        match stream.read(&mut chunk) {
            Ok(0) => return Err(AUTH_ERROR.into()),
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                chunk.fill(0);
                if bytes.len() > MAX_CALLBACK_HEADERS {
                    return Err(AUTH_ERROR.into());
                }
                if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    return parse_callback_request(&bytes, pending.port);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(_) => return Err(AUTH_ERROR.into()),
        }
    }
}

fn parse_callback_request(
    bytes: &[u8],
    port: u16,
) -> Result<BTreeMap<String, Zeroizing<String>>, String> {
    if bytes.len() > MAX_CALLBACK_HEADERS {
        return Err(AUTH_ERROR.into());
    }
    let request = std::str::from_utf8(bytes).map_err(|_| AUTH_ERROR)?;
    let (headers, body) = request.split_once("\r\n\r\n").ok_or(AUTH_ERROR)?;
    if !body.is_empty() {
        return Err(AUTH_ERROR.into());
    }
    let mut lines = headers.split("\r\n");
    let first = lines.next().ok_or(AUTH_ERROR)?;
    let parts = first.split(' ').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
        return Err(AUTH_ERROR.into());
    }
    let (path, query) = parts[1].split_once('?').ok_or(AUTH_ERROR)?;
    if path != "/auth/callback" || query.contains('#') {
        return Err(AUTH_ERROR.into());
    }
    let expected_host = format!("127.0.0.1:{port}");
    let mut host_seen = false;
    let mut names = HashSet::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(AUTH_ERROR)?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(AUTH_ERROR.into());
        }
        let name = name.to_ascii_lowercase();
        if !names.insert(name.clone()) {
            return Err(AUTH_ERROR.into());
        }
        let value = value.trim();
        if value.bytes().any(|b| b < 32 || b == 127) {
            return Err(AUTH_ERROR.into());
        }
        match name.as_str() {
            "host" => {
                if value != expected_host {
                    return Err(AUTH_ERROR.into());
                }
                host_seen = true;
            }
            "content-length" if value == "0" => (),
            "content-length" | "transfer-encoding" | "origin" => return Err(AUTH_ERROR.into()),
            _ => (),
        }
    }
    if !host_seen {
        return Err(AUTH_ERROR.into());
    }
    let mut fields = BTreeMap::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or(AUTH_ERROR)?;
        let key = decode_query(key)?;
        // OAuth's documented optional scope is accepted but never used as granted authority.
        if !matches!(
            key.as_str(),
            "code" | "state" | "client_id" | "error" | "scope"
        ) {
            return Err(AUTH_ERROR.into());
        }
        if fields
            .insert(key, Zeroizing::new(decode_query(value)?))
            .is_some()
        {
            return Err(AUTH_ERROR.into());
        }
    }
    Ok(fields)
}

fn decode_query(value: &str) -> Result<String, String> {
    let mut decoded = Zeroizing::new(Vec::with_capacity(value.len()));
    let input = value.as_bytes();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'%' => {
                if index + 2 >= input.len() {
                    return Err(AUTH_ERROR.into());
                }
                let hex = |b: u8| {
                    (b as char)
                        .to_digit(16)
                        .filter(|_| b.is_ascii_hexdigit())
                        .map(|n| n as u8)
                };
                let byte = hex(input[index + 1])
                    .zip(hex(input[index + 2]))
                    .map(|(a, b)| (a << 4) | b)
                    .ok_or(AUTH_ERROR)?;
                decoded.push(byte);
                index += 3;
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte if byte.is_ascii_graphic() => {
                decoded.push(byte);
                index += 1;
            }
            _ => return Err(AUTH_ERROR.into()),
        }
    }
    let result = std::str::from_utf8(&decoded).map_err(|_| AUTH_ERROR)?;
    if result.bytes().any(|b| b < 32 || b == 127) {
        return Err(AUTH_ERROR.into());
    }
    Ok(result.to_string())
}

fn callback_page(stream: &mut TcpStream, accepted: bool) {
    let body = if accepted {
        "<!doctype html><meta charset=utf-8><title>クッションAI</title><p>認証情報を確認しています。クッションAIへ戻ってください。</p>"
    } else {
        "<!doctype html><meta charset=utf-8><title>クッションAI</title><p>この認証要求は受け付けませんでした。クッションAIへ戻ってください。</p>"
    };
    let status = if accepted {
        "200 OK"
    } else {
        "400 Bad Request"
    };
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nPragma: no-cache\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
    let _ = stream.write_all(response.as_bytes());
}

fn open_system_browser(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || url.path() != "/api/accounts/authorize"
        || url.port().is_some()
    {
        return Err(AUTH_ERROR.into());
    }
    // Fixed program + OAuth-only URL, no shell and no credential/login hints.
    let status = Command::new("/usr/bin/open")
        .arg(url.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| AUTH_ERROR)?;
    if !status.success() {
        return Err("既定のブラウザでChatGPTの認証画面を開けませんでした。".into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(with = "secret_serde")]
    access_token: Zeroizing<String>,
    #[serde(with = "secret_serde")]
    refresh_token: Zeroizing<String>,
    #[serde(with = "secret_serde")]
    id_token: Zeroizing<String>,
    token_type: String,
    expires_in: u64,
    scope: String,
}

fn parse_token_response(
    response: reqwest::blocking::Response,
    error: &'static str,
) -> Result<TokenResponse, String> {
    if response.status() != reqwest::StatusCode::OK {
        return Err(error.into());
    }
    let body = bounded_body(response, MAX_AUTH_BODY, error)?;
    serde_json::from_slice(&body).map_err(|_| error.into())
}

fn credentials_from_response(
    token: TokenResponse,
    nonce: &str,
    epoch: &str,
    saved_at: u64,
) -> Result<Credentials, String> {
    let scopes = granted_scopes(&token.scope)?;
    if !token.token_type.eq_ignore_ascii_case("Bearer")
        || token.expires_in == 0
        || token.expires_in > 3600
        || !opaque(&token.access_token, 32768)
        || !opaque(&token.refresh_token, 32768)
        || !opaque(&token.id_token, 32768)
    {
        return Err(AUTH_ERROR.into());
    }
    Ok(Credentials {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        id_token: token.id_token,
        scopes,
        expires_at: saved_at.checked_add(token.expires_in).ok_or(AUTH_ERROR)?,
        saved_at,
        nonce: nonce.to_string(),
        epoch: epoch.to_string(),
    })
}

fn replacement_credentials(
    token: TokenResponse,
    jwks: &JwkSet,
    registration: &Registration,
    received_at: u64,
    verification_time: u64,
) -> Result<Credentials, String> {
    let claims = verify_identity(
        &token.id_token,
        jwks,
        &registration.client_id,
        None,
        verification_time,
    )?;
    if claims.sub != registration.subject
        || claims
            .nonce
            .as_ref()
            .is_some_and(|nonce| nonce != &registration.credentials.nonce)
    {
        return Err(REFRESH_ERROR.into());
    }
    credentials_from_response(
        token,
        &registration.credentials.nonce,
        &registration.credentials.epoch,
        received_at,
    )
}

fn granted_scopes(value: &str) -> Result<Vec<String>, String> {
    if value.len() > 4096 {
        return Err(AUTH_ERROR.into());
    }
    let scopes = value
        .split_ascii_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if scopes.iter().any(|s| !opaque(s, 128))
        || REQUIRED_SCOPES
            .iter()
            .any(|required| !scopes.iter().any(|s| s == required))
    {
        return Err("ChatGPTプランを使うための権限が付いていません。接続を停止しました。".into());
    }
    Ok(scopes)
}

#[derive(Deserialize)]
struct IdentityClaims {
    sub: String,
    exp: u64,
    iat: u64,
    nonce: Option<String>,
    nbf: Option<u64>,
    azp: Option<String>,
}

fn verify_identity(
    token: &str,
    jwks: &JwkSet,
    client_id: &str,
    expected_nonce: Option<&str>,
    time: u64,
) -> Result<IdentityClaims, String> {
    if token.len() > 32768 || !issued_client(client_id) {
        return Err(AUTH_ERROR.into());
    }
    let header = decode_header(token).map_err(|_| AUTH_ERROR)?;
    if !matches!(
        header.alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    ) {
        return Err(AUTH_ERROR.into());
    }
    let kid = header
        .kid
        .as_ref()
        .filter(|id| opaque(id, 256))
        .ok_or(AUTH_ERROR)?;
    let mut keys = jwks
        .keys
        .iter()
        .filter(|jwk| jwk.common.key_id.as_ref() == Some(kid));
    let key = keys.next().ok_or(AUTH_ERROR)?;
    if keys.next().is_some() {
        return Err(AUTH_ERROR.into());
    }
    if key
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &jsonwebtoken::jwk::PublicKeyUse::Signature)
        || key
            .common
            .key_operations
            .as_ref()
            .is_some_and(|operations| {
                !operations.contains(&jsonwebtoken::jwk::KeyOperations::Verify)
            })
        || key
            .common
            .key_algorithm
            .is_some_and(|algorithm| algorithm.to_string() != format!("{:?}", header.alg))
    {
        return Err(AUTH_ERROR.into());
    }
    let decoding = DecodingKey::from_jwk(key).map_err(|_| AUTH_ERROR)?;
    let mut validation = Validation::new(header.alg);
    validation.set_audience(&[client_id]);
    validation.set_issuer(&[ISSUER]);
    validation.set_required_spec_claims(&["sub", "exp", "iat", "iss", "aud"]);
    validation.leeway = 0;
    validation.validate_nbf = true;
    let claims = decode::<IdentityClaims>(token, &decoding, &validation)
        .map_err(|_| AUTH_ERROR)?
        .claims;
    if !opaque(&claims.sub, 1024)
        || claims.exp <= time
        || claims.exp <= claims.iat
        || claims.iat > time.saturating_add(30)
        || claims.nbf.is_some_and(|nbf| nbf > time)
        || claims.azp.as_ref().is_some_and(|azp| azp != client_id)
        || expected_nonce.is_some_and(|nonce| claims.nonce.as_deref() != Some(nonce))
    {
        return Err(AUTH_ERROR.into());
    }
    Ok(claims)
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|_| AUTH_ERROR.into())
}

fn fetch_jwks(client: &reqwest::blocking::Client) -> Result<JwkSet, String> {
    let response = client.get(JWKS).send().map_err(|_| AUTH_ERROR)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(AUTH_ERROR.into());
    }
    let body = bounded_body(response, MAX_AUTH_BODY, AUTH_ERROR)?;
    let keys: JwkSet = serde_json::from_slice(&body).map_err(|_| AUTH_ERROR)?;
    if keys.keys.is_empty() || keys.keys.len() > 64 {
        return Err(AUTH_ERROR.into());
    }
    Ok(keys)
}

fn bounded_body(
    response: reqwest::blocking::Response,
    limit: usize,
    error: &'static str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(error.into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error)?;
    if bytes.len() > limit {
        return Err(error.into());
    }
    Ok(bytes)
}

#[derive(Deserialize)]
struct Catalog {
    models: Vec<CatalogModel>,
}
#[derive(Deserialize)]
struct CatalogModel {
    slug: String,
    display_name: String,
    visibility: String,
}

fn parse_models(bytes: &[u8]) -> Result<Vec<PlanModel>, String> {
    if bytes.len() > MAX_CATALOG_BODY {
        return Err(CATALOG_ERROR.into());
    }
    let catalog: Catalog = serde_json::from_slice(bytes).map_err(|_| CATALOG_ERROR)?;
    if catalog.models.len() > 1024 {
        return Err(CATALOG_ERROR.into());
    }
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for model in catalog.models {
        if model.visibility != "list" {
            continue;
        }
        if model.slug.is_empty()
            || model.slug.len() > 128
            || !model
                .slug
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || model.display_name.is_empty()
            || model.display_name.chars().count() > 128
            || model.display_name.chars().any(char::is_control)
            || !seen.insert(model.slug.clone())
        {
            return Err(CATALOG_ERROR.into());
        }
        models.push(PlanModel {
            slug: model.slug,
            display_name: model.display_name,
        });
    }
    if models.is_empty() {
        return Err(CATALOG_ERROR.into());
    }
    Ok(models)
}

fn validate_record(record: &Record) -> Result<(), String> {
    if record.version != 1 || !valid_host(&record.host_id) {
        return Err(KEYCHAIN_ERROR.into());
    }
    if let Some(r) = &record.registration {
        if !issued_client(&r.client_id)
            || !opaque(&r.subject, 1024)
            || !opaque(&r.credentials.access_token, 32768)
            || !opaque(&r.credentials.refresh_token, 32768)
            || !opaque(&r.credentials.id_token, 32768)
            || r.credentials.expires_at <= r.credentials.saved_at
            || r.credentials.expires_at - r.credentials.saved_at > 3600
            || !valid_random(&r.credentials.nonce)
            || !valid_random(&r.credentials.epoch)
        {
            return Err(KEYCHAIN_ERROR.into());
        }
        granted_scopes(&r.credentials.scopes.join(" "))?;
    }
    Ok(())
}

fn same_registration(a: &Registration, b: &Registration) -> bool {
    a.client_id == b.client_id
        && a.subject == b.subject
        && a.credentials.epoch == b.credentials.epoch
}

fn validate_commit(
    previous: &Record,
    candidate: &Record,
    expected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if previous.host_id != candidate.host_id || record_binding(previous).as_deref() != expected {
        return Err(AUTH_ERROR.into());
    }
    validate_record(candidate)
}

fn record_binding(record: &Record) -> Option<String> {
    let r = record.registration.as_ref()?;
    let mut hash = Sha256::new();
    for value in [
        "cushion-chatgpt-plan-binding-v1",
        ISSUER,
        &r.client_id,
        &r.subject,
        &r.credentials.epoch,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    Some(format!(
        "chatgpt-plan:{}",
        URL_SAFE_NO_PAD.encode(hash.finalize())
    ))
}

#[cfg(target_os = "macos")]
fn process_lock(cancel: Option<&AtomicBool>) -> Result<std::fs::File, String> {
    use std::os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawFd,
    };
    let directory = std::env::temp_dir();
    let metadata = std::fs::symlink_metadata(&directory).map_err(|_| KEYCHAIN_ERROR)?;
    let uid = unsafe { libc::getuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(KEYCHAIN_ERROR.into());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join(".cushion-chatgpt-plan-refresh-v1.lock"))
        .map_err(|_| KEYCHAIN_ERROR)?;
    let metadata = file.metadata().map_err(|_| KEYCHAIN_ERROR)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.len() != 0
        || metadata.nlink() != 1
    {
        return Err(KEYCHAIN_ERROR.into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(cancel) = cancel {
            check_cancel(cancel)?;
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) || Instant::now() >= deadline {
            return Err(
                "別のクッションAIでChatGPT認証を扱っています。しばらくしてから再実行してください。"
                    .into(),
            );
        }
        thread::sleep(Duration::from_millis(40));
    }
}

#[cfg(not(target_os = "macos"))]
fn process_lock(_: Option<&AtomicBool>) -> Result<std::fs::File, String> {
    Err(KEYCHAIN_ERROR.into())
}

#[cfg(target_os = "macos")]
fn keychain_options() -> security_framework::passwords::PasswordOptions {
    let mut options =
        security_framework::passwords::PasswordOptions::new_generic_password(SERVICE, ACCOUNT);
    options.set_access_synchronized(Some(false));
    options
}

#[cfg(target_os = "macos")]
fn load_record() -> Result<Option<Record>, String> {
    match security_framework::passwords::generic_password(keychain_options()) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            if bytes.len() > MAX_AUTH_BODY {
                return Err(KEYCHAIN_ERROR.into());
            }
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| KEYCHAIN_ERROR.into())
        }
        Err(error) if error.code() == -25300 => Ok(None), // errSecItemNotFound, never permission failure.
        Err(_) => Err(KEYCHAIN_ERROR.into()),
    }
}

#[cfg(target_os = "macos")]
fn save_record(record: &Record) -> Result<(), String> {
    let bytes = Zeroizing::new(serde_json::to_vec(record).map_err(|_| KEYCHAIN_ERROR)?);
    if bytes.len() > MAX_AUTH_BODY {
        return Err(KEYCHAIN_ERROR.into());
    }
    security_framework::passwords::set_generic_password_options(&bytes, keychain_options())
        .map_err(|_| KEYCHAIN_ERROR.into())
}

#[cfg(not(target_os = "macos"))]
fn load_record() -> Result<Option<Record>, String> {
    Err(KEYCHAIN_ERROR.into())
}
#[cfg(not(target_os = "macos"))]
fn save_record(_: &Record) -> Result<(), String> {
    Err(KEYCHAIN_ERROR.into())
}

fn random_32() -> Result<String, String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(bytes.as_mut()).map_err(|_| AUTH_ERROR)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes.as_ref()))
}

fn host_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| AUTH_ERROR)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn valid_host(host: &str) -> bool {
    let Some(id) = host.strip_prefix("urn:uuid:") else {
        return false;
    };
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && id.as_bytes()[14] == b'4'
        && matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}
fn issued_client(value: &str) -> bool {
    value != DYNAMIC_CLIENT
        && opaque(value, 256)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn valid_random(value: &str) -> bool {
    value.len() == 43
        && URL_SAFE_NO_PAD
            .decode(value)
            .is_ok_and(|bytes| bytes.len() == 32)
}
fn opaque(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|b| b.is_ascii_graphic())
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| AUTH_ERROR.into())
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

mod secret_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Zeroizing<String>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Zeroizing<String>, D::Error> {
        String::deserialize(deserializer).map(Zeroizing::new)
    }
}

#[cfg(test)]
impl Session {
    /// Synthetic in-memory fixtures; no Keychain, files, login or provider request.
    pub(crate) fn synthetic_for_test(model: &str) -> Self {
        Self::synthetic_for_test_binding(model, "synthetic-registration")
    }
    pub(crate) fn synthetic_for_test_binding(model: &str, registration: &str) -> Self {
        let saved_at = now().unwrap();
        let random = URL_SAFE_NO_PAD.encode(Sha256::digest(registration.as_bytes()));
        let record = Record {
            version: 1,
            host_id: "urn:uuid:12345678-1234-4123-8123-123456789abc".into(),
            registration: Some(Registration {
                client_id: "oaiapp_synthetic".into(),
                subject: registration.into(),
                credentials: Credentials {
                    access_token: Zeroizing::new("synthetic-access-never-use".into()),
                    refresh_token: Zeroizing::new("synthetic-refresh-never-use".into()),
                    id_token: Zeroizing::new("synthetic-id-never-use".into()),
                    scopes: REQUIRED_SCOPES.iter().map(|s| s.to_string()).collect(),
                    expires_at: saved_at + 3600,
                    saved_at,
                    nonce: random.clone(),
                    epoch: random,
                },
            }),
        };
        Self {
            record,
            catalog: vec![PlanModel {
                slug: model.into(),
                display_name: "架空テストモデル".into(),
            }],
            usable: true,
            persisted: false,
            expected_binding: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
    use serde_json::{json, Value};
    use std::sync::OnceLock;

    fn request(query: &str) -> Vec<u8> {
        format!("GET /auth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n").into_bytes()
    }
    fn callback(pending: &Pending, query: &str) -> CallbackDecision {
        pending.evaluate(parse_callback_request(&request(query), pending.port).unwrap())
    }
    fn fixture_keys() -> &'static (EncodingKey, JwkSet) {
        static KEYS: OnceLock<(EncodingKey, JwkSet)> = OnceLock::new();
        KEYS.get_or_init(|| {
            let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
            let der = private.to_pkcs1_der().unwrap();
            let public = json!({"keys":[{"kty":"RSA","kid":"synthetic-key","alg":"RS256","use":"sig",
                "n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()), "e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]});
            (EncodingKey::from_rsa_der(der.as_bytes()), serde_json::from_value(public).unwrap())
        })
    }
    fn claims() -> Value {
        let time = now().unwrap();
        json!({"iss":ISSUER,"aud":"oaiapp_synthetic","sub":"synthetic-subject","exp":time+3600,"iat":time,"nonce":"synthetic-nonce"})
    }
    fn jwt(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("synthetic-key".into());
        encode(&header, claims, &fixture_keys().0).unwrap()
    }
    fn verifies(claims: &Value) -> bool {
        verify_identity(
            &jwt(claims),
            &fixture_keys().1,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap(),
        )
        .is_ok()
    }

    #[test]
    fn authorization_uses_fixed_origin_resource_pkce_and_no_credentials() {
        let pending = Pending::new(34567, None).unwrap();
        let url = pending
            .authorization_url("urn:uuid:12345678-1234-4123-8123-123456789abc")
            .unwrap();
        assert_eq!(url.origin().ascii_serialization(), ISSUER);
        let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], DYNAMIC_CLIENT);
        assert_eq!(query["agent_name_hint"], "クッションAI");
        assert_eq!(
            query["redirect_uri"],
            "http://127.0.0.1:34567/auth/callback"
        );
        assert_eq!(query["resource"], RESOURCE);
        assert_eq!(query["scope"], SCOPES);
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.as_bytes()))
        );
        for secret in [
            "access_token",
            "refresh_token",
            "id_token_hint",
            "login_hint",
            "code_verifier",
            "client_secret",
        ] {
            assert!(!query.contains_key(secret));
        }
        let another = Pending::new(34567, None).unwrap();
        assert_ne!(pending.state.as_str(), another.state.as_str());
        assert_ne!(pending.nonce.as_str(), another.nonce.as_str());
        assert_ne!(pending.verifier.as_str(), another.verifier.as_str());
        assert!(
            valid_random(&pending.state)
                && valid_random(&pending.nonce)
                && valid_random(&pending.verifier)
        );
    }

    #[test]
    fn forged_state_and_duplicates_do_not_consume_pending_valid_callback() {
        let pending = Pending::new(34567, None).unwrap();
        assert!(matches!(
            callback(&pending, "code=fake&state=spoof&client_id=oaiapp_wrong"),
            CallbackDecision::Reject
        ));
        for suffix in [
            "state=spoof",
            "%73tate=spoof",
            "code=second",
            "client_id=oaiapp_other",
        ] {
            let query = format!(
                "code=synthetic&state={}&client_id=oaiapp_synthetic&{suffix}",
                pending.state.as_str()
            );
            assert!(parse_callback_request(&request(&query), pending.port).is_err());
        }
        let query = format!(
            "code=synthetic&state={}&client_id=oaiapp_synthetic&scope=openid",
            pending.state.as_str()
        );
        assert!(matches!(
            callback(&pending, &query),
            CallbackDecision::Complete(Ok(_))
        ));
    }

    #[test]
    fn callback_accepts_only_get_exact_loopback_path_no_body_or_original() {
        for message in [
            "POST /auth/callback?code=x&state=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n",
            "GET /callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n",
            "GET http://127.0.0.1:34567/auth/callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n",
            "GET /auth/callback?original=synthetic HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n",
            "GET /auth/callback?code=x HTTP/1.1\r\nHost: localhost:34567\r\n\r\n",
            "GET /auth/callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\nOrigin: https://invalid.test\r\n\r\n",
            "GET /auth/callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\nContent-Length: 1\r\n\r\nx",
            "GET /auth/callback?code=x HTTP/1.1\r\nHost: 127.0.0.1:34567\r\nTransfer-Encoding: chunked\r\n\r\n",
            "GET /auth/callback?code=%XX HTTP/1.1\r\nHost: 127.0.0.1:34567\r\n\r\n",
        ] { assert!(parse_callback_request(message.as_bytes(),34567).is_err()); }
        assert!(parse_callback_request(&request("code=%00"), 34567).is_err());
        assert!(parse_callback_request(&vec![b'x'; MAX_CALLBACK_HEADERS + 1], 34567).is_err());
    }

    #[test]
    fn oauth_denial_requires_correct_state_and_is_terminal() {
        let pending = Pending::new(34567, None).unwrap();
        assert!(matches!(
            callback(&pending, "error=access_denied&state=wrong"),
            CallbackDecision::Reject
        ));
        assert!(matches!(
            callback(
                &pending,
                &format!("error=access_denied&state={}", pending.state.as_str())
            ),
            CallbackDecision::Complete(Err(_))
        ));
        for query in [
            format!("code=x&state={}", pending.state.as_str()),
            format!(
                "code=x&state={}&client_id={DYNAMIC_CLIENT}",
                pending.state.as_str()
            ),
        ] {
            assert!(matches!(
                callback(&pending, &query),
                CallbackDecision::Complete(Err(_))
            ));
        }
    }

    #[test]
    fn reauthorization_keeps_issued_client_no_hints_and_rejects_changed_client() {
        let pending = Pending::new(34567, Some("oaiapp_synthetic")).unwrap();
        let url = pending.authorization_url("synthetic-host").unwrap();
        assert!(!url.query_pairs().any(|(key, _)| key == "agent_name_hint"
            || key == "login_hint"
            || key == "id_token_hint"));
        let query = format!("code=x&state={}", pending.state.as_str());
        assert!(matches!(
            callback(&pending, &query),
            CallbackDecision::Complete(Ok(_))
        ));
        let query = format!(
            "code=x&state={}&client_id=oaiapp_other",
            pending.state.as_str()
        );
        assert!(matches!(
            callback(&pending, &query),
            CallbackDecision::Complete(Err(_))
        ));
    }

    #[test]
    fn listener_finishes_once_and_honors_cancel_and_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let pending = Pending::new(port, None).unwrap();
        let state = pending.state.to_string();
        let sender = thread::spawn(move || {
            for query in [
                "code=fake&state=spoof&client_id=oaiapp_synthetic".to_string(),
                format!("code=synthetic&state={state}&client_id=oaiapp_synthetic"),
            ] {
                let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let req = format!(
                    "GET /auth/callback?{query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
                );
                stream.write_all(req.as_bytes()).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                assert!(!response.contains(&state) && !response.contains("synthetic&"));
            }
        });
        let callback = wait_callback(&listener, &pending, &AtomicBool::new(false)).unwrap();
        assert_eq!(callback.code.as_str(), "synthetic");
        sender.join().unwrap();
        assert!(wait_callback(&listener, &pending, &AtomicBool::new(true)).is_err());
        let mut expired = Pending::new(port, None).unwrap();
        expired.started = Instant::now() - DEADLINE;
        assert!(wait_callback(&listener, &expired, &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn required_grants_are_checked_from_token_response() {
        assert!(granted_scopes(SCOPES).is_ok());
        for missing in REQUIRED_SCOPES {
            let reduced = REQUIRED_SCOPES
                .iter()
                .filter(|scope| **scope != missing)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(granted_scopes(&reduced).is_err());
        }
        assert!(granted_scopes("openid profile email").is_err());
    }

    #[test]
    fn jwt_valid_signature_identity_baseline() {
        assert!(verifies(&claims()));
    }

    #[test]
    fn refresh_replacement_checks_identity_and_grants_and_keeps_epoch() {
        let mut session = Session::synthetic_for_test("model-a");
        let registration = session.record.registration.as_mut().unwrap();
        registration.subject = "synthetic-subject".into();
        registration.credentials.nonce = "synthetic-nonce".into();
        let token = |identity: &Value, scope: &str| TokenResponse {
            access_token: Zeroizing::new("replacement-access".into()),
            refresh_token: Zeroizing::new("replacement-refresh".into()),
            id_token: Zeroizing::new(jwt(identity)),
            token_type: "Bearer".into(),
            expires_in: 3600,
            scope: scope.into(),
        };
        let time = now().unwrap();
        let replaced = replacement_credentials(
            token(&claims(), SCOPES),
            &fixture_keys().1,
            registration,
            time,
            time,
        )
        .unwrap();
        assert_eq!(replaced.epoch, registration.credentials.epoch);
        assert_eq!(replaced.access_token.as_str(), "replacement-access");
        assert_eq!(replaced.refresh_token.as_str(), "replacement-refresh");
        assert_eq!(replaced.expires_at, time + 3600);
        for key in ["sub", "nonce", "aud"] {
            let mut invalid = claims();
            invalid[key] = json!("different");
            assert!(replacement_credentials(
                token(&invalid, SCOPES),
                &fixture_keys().1,
                registration,
                time,
                time
            )
            .is_err());
        }
        assert!(replacement_credentials(
            token(&claims(), "openid profile email"),
            &fixture_keys().1,
            registration,
            time,
            time
        )
        .is_err());
        let mut no_nonce = claims();
        no_nonce.as_object_mut().unwrap().remove("nonce");
        // OIDC refresh ID tokens may omit nonce; a supplied nonce must match original.
        assert!(replacement_credentials(
            token(&no_nonce, SCOPES),
            &fixture_keys().1,
            registration,
            time,
            time
        )
        .is_ok());
        let missing = json!({"access_token":"synthetic","id_token":"synthetic","token_type":"Bearer","expires_in":3600,"scope":SCOPES});
        assert!(serde_json::from_value::<TokenResponse>(missing).is_err());
    }

    #[test]
    fn jwt_rejects_nonce_audience_issuer_subject_and_times() {
        for (key, value) in [
            ("nonce", json!("wrong")),
            ("aud", json!("oaiapp_other")),
            ("iss", json!("https://invalid.test")),
            ("sub", json!("")),
            ("exp", json!(now().unwrap() - 1)),
            ("iat", json!(now().unwrap() + 300)),
            ("nbf", json!(now().unwrap() + 300)),
            ("azp", json!("oaiapp_other")),
        ] {
            let mut value_claims = claims();
            value_claims[key] = value;
            assert!(!verifies(&value_claims));
        }
        for missing in ["nonce", "aud", "iss", "sub", "exp", "iat"] {
            let mut value_claims = claims();
            value_claims.as_object_mut().unwrap().remove(missing);
            assert!(!verifies(&value_claims));
        }
    }

    #[test]
    fn jwt_rejects_tampered_signature_unknown_kid_and_symmetric_algorithm() {
        let good = jwt(&claims());
        let (signed, signature) = good.rsplit_once('.').unwrap();
        let mut signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
        signature[0] ^= 1;
        let tampered = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature));
        assert!(verify_identity(
            &tampered,
            &fixture_keys().1,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("unknown".into());
        let unknown = encode(&header, &claims(), &fixture_keys().0).unwrap();
        assert!(verify_identity(
            &unknown,
            &fixture_keys().1,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("synthetic-key".into());
        let symmetric = encode(
            &header,
            &claims(),
            &EncodingKey::from_secret(b"synthetic-only"),
        )
        .unwrap();
        assert!(verify_identity(
            &symmetric,
            &fixture_keys().1,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
    }

    #[test]
    fn jwt_rejects_duplicate_key_ids_wrong_algorithm_and_encryption_keys() {
        let token = jwt(&claims());
        let mut duplicate = fixture_keys().1.clone();
        duplicate.keys.push(duplicate.keys[0].clone());
        assert!(verify_identity(
            &token,
            &duplicate,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
        let mut encrypted = fixture_keys().1.clone();
        encrypted.keys[0].common.public_key_use = Some(jsonwebtoken::jwk::PublicKeyUse::Encryption);
        assert!(verify_identity(
            &token,
            &encrypted,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
        let mut algorithm = fixture_keys().1.clone();
        algorithm.keys[0].common.key_algorithm = Some(jsonwebtoken::jwk::KeyAlgorithm::RS384);
        assert!(verify_identity(
            &token,
            &algorithm,
            "oaiapp_synthetic",
            Some("synthetic-nonce"),
            now().unwrap()
        )
        .is_err());
    }

    #[test]
    fn catalog_preserves_only_visible_identifiers_and_server_order() {
        let value = json!({"models":[{"slug":"model-b","display_name":"B","visibility":"list","email":"synthetic@example.invalid"},
            {"slug":"hidden","display_name":"H","visibility":"hide"},{"slug":"model-a","display_name":"A","visibility":"list"}]});
        let models = parse_models(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            models.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(),
            vec!["model-b", "model-a"]
        );
        let ui = serde_json::to_string(&models).unwrap();
        assert!(!ui.contains("email") && !ui.contains("hidden"));
        for invalid in [
            json!({"models":[]}),
            json!({"data":[]}),
            json!({"models":[{"slug":"https://invalid.test","display_name":"evil","visibility":"list"}]}),
        ] {
            assert!(parse_models(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn status_exposes_no_secrets_or_identity_and_requires_catalog() {
        let mut session = Session::synthetic_for_test("model-a");
        let status = serde_json::to_string(&session.status()).unwrap();
        assert!(session.status().connected);
        for secret in [
            "synthetic-access",
            "synthetic-refresh",
            "synthetic-id",
            "oaiapp_",
            "synthetic-registration",
            "nonce",
            "subject",
            "email",
        ] {
            assert!(!status.contains(secret));
        }
        session.catalog.clear();
        assert!(!session.status().connected);
        session.usable = false;
        assert_eq!(session.access_token(), "");
    }

    #[test]
    fn binding_rotates_on_identity_signin_not_rotating_tokens() {
        let mut session = Session::synthetic_for_test("model-a");
        let original = session.binding();
        let r = session.record.registration.as_mut().unwrap();
        r.credentials.access_token = Zeroizing::new("replacement-access".into());
        r.credentials.refresh_token = Zeroizing::new("replacement-refresh".into());
        assert_eq!(session.binding(), original);
        session
            .record
            .registration
            .as_mut()
            .unwrap()
            .credentials
            .epoch = random_32().unwrap();
        assert_ne!(session.binding(), original);
        let another = Session::synthetic_for_test_binding("model-a", "different-account");
        assert_ne!(another.binding(), original);
        assert!(!same_registration(
            session.record.registration.as_ref().unwrap(),
            another.record.registration.as_ref().unwrap()
        ));
    }

    #[test]
    fn expired_memory_only_session_stops_and_clears_models_without_io() {
        let mut session = Session::synthetic_for_test("model-a");
        assert!(session.ensure_access().is_ok());
        session
            .record
            .registration
            .as_mut()
            .unwrap()
            .credentials
            .expires_at = now().unwrap() - 1;
        assert!(session.ensure_access().is_err());
        assert!(
            !session.status().connected
                && session.models().is_empty()
                && session.access_token().is_empty()
        );
        assert!(session.ensure_access().is_err());
    }

    #[test]
    fn canceled_or_superseded_login_cannot_commit_without_io() {
        let mut candidate = Session::synthetic_for_test("model-a");
        assert_eq!(
            candidate.commit(&AtomicBool::new(true)).unwrap_err(),
            CANCELLED
        );
        assert!(!candidate.persisted);
        let previous = Session::synthetic_for_test_binding("model-a", "previous-account");
        let binding = previous.binding();
        assert!(validate_commit(
            &previous.record,
            &candidate.record,
            Some(&binding),
            &AtomicBool::new(false)
        )
        .is_ok());
        assert!(validate_commit(
            &previous.record,
            &candidate.record,
            None,
            &AtomicBool::new(false)
        )
        .is_err());
        assert!(validate_commit(
            &previous.record,
            &candidate.record,
            Some(&binding),
            &AtomicBool::new(true)
        )
        .is_err());
        let superseded = Session::synthetic_for_test_binding("model-a", "newer-account");
        assert!(validate_commit(
            &superseded.record,
            &candidate.record,
            Some(&binding),
            &AtomicBool::new(false)
        )
        .is_err());
        candidate.record.host_id = host_id().unwrap();
        assert!(validate_commit(
            &previous.record,
            &candidate.record,
            Some(&binding),
            &AtomicBool::new(false)
        )
        .is_err());
    }

    #[test]
    fn keychain_record_is_distinct_validated_and_no_plaintext_fallback_exists() {
        let session = Session::synthetic_for_test("model-a");
        assert!(validate_record(&session.record).is_ok());
        assert_eq!(SERVICE, "local.cushion.chatgpt-plan.v1");
        assert_ne!(SERVICE, "local.cushion.api.v1");
        let bytes = serde_json::to_vec(&session.record).unwrap();
        let mut record: Record = serde_json::from_slice(&bytes).unwrap();
        record.registration.as_mut().unwrap().client_id = DYNAMIC_CLIENT.into();
        assert!(validate_record(&record).is_err());
        assert!(valid_host(&host_id().unwrap()));
        assert!(!valid_host("urn:uuid:../../untrusted"));
    }
}
