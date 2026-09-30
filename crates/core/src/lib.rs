//! Local privacy state machine. All development fixtures and tests are synthetic.
//! Core performs no filesystem/network/clipboard IO. Native-host entrypoints accept
//! verified local worker results and gate an explicitly supplied export effect.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(not(target_arch = "wasm32"))]
mod local;
#[cfg(not(target_arch = "wasm32"))]
pub use local::{LocalFailure, LocalScanJob, NativeDeliveryFailure, SavedCaseData, LOCAL_POLICY};

pub const SAMPLE: &str = "こもれび商店の小森さんへ、発送日が9月20日から9月23日に変わるお知らせを書いてください。注文番号はKMR-4821です。";
pub const BLOCKED_SAMPLE: &str = "架空職員の小森さんの診断名と服薬内容を整理してください。";
pub const DEFAULT_DICTIONARY: &str = "こもれび商店\n小森\nKMR-4821";
pub const POLICY: &str = "synthetic-v1";
pub const MODEL: &str = "demo-deterministic-v1";
pub const MAX_REQUEST_BYTES: usize = 131_072;
const MAX_DICTIONARY_BYTES: usize = 8_192;
const MAX_ANSWER_CHARS: usize = 24_000;
const APPROVAL_TTL_MS: u64 = 5 * 60 * 1_000;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Phase {
    Draft,
    Scanning,
    Review,
    Approved,
    Exported,
    Blocked,
    RestoredLocal,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub start: usize,
    pub end: usize,
    pub original: String,
    pub replacement: String,
    pub action: String,
    pub reason: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub case_id: String,
    pub revision: u64,
    pub phase: Phase,
    pub original: String,
    pub purpose: String,
    pub destination: String,
    pub mode: String,
    pub transformed: String,
    pub findings: Vec<Finding>,
    pub residual: Vec<String>,
    pub lost: Vec<String>,
    pub blocked_reason: Option<String>,
    pub approval_id: Option<String>,
    pub job_id: Option<String>,
    pub restored: String,
    pub unknown_tokens: Vec<String>,
    pub test_copy: Option<String>,
    pub dictionary_text: String,
    pub policy_version: String,
    pub model_config: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    pub original: Option<String>,
    pub purpose: Option<String>,
    pub destination: Option<String>,
    pub mode: Option<String>,
    pub transformed: Option<String>,
    pub dictionary_text: Option<String>,
    pub policy_version: Option<String>,
    pub model_config: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Snapshot {},
    Update {
        patch: Patch,
    },
    BeginScan {},
    FinishScan {
        #[serde(rename = "jobId")]
        job_id: String,
        fault: Fault,
    },
    Cancel {},
    Approve {
        revision: u64,
    },
    DemoExport {
        #[serde(rename = "approvalId")]
        approval_id: String,
    },
    ExportPrepared {
        #[serde(rename = "approvalId")]
        approval_id: String,
    },
    Restore {
        answer: String,
    },
    RestoreIfCurrent {
        answer: String,
        #[serde(rename = "caseId")]
        case_id: String,
        revision: u64,
    },
    NewCase {},
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    None,
    InvalidJson,
    Stopped,
    Incomplete,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub id: String,
    pub original: String,
    pub secrets: Vec<String>,
    pub expected_state: String,
    pub prohibited: Vec<String>,
    pub preserve: Vec<String>,
    pub expected_ranges: Vec<ExpectedRange>,
    pub blocked_reason: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct ExpectedRange {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

pub fn synthetic_fixtures() -> Vec<Fixture> {
    // Compile-time, checked-in synthetic data only. Never reads user directories.
    serde_json::from_str(include_str!("../../../tests/fixtures/synthetic/cases.json"))
        .expect("bundled synthetic fixtures invalid")
}

/// Separate, frozen development fixtures for the first local business workflow.
/// Never expands the legacy benchmark corpus or enables arbitrary business input.
pub fn workflow_fixtures() -> Vec<Fixture> {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/workflows/support-record.json"
    ))
    .expect("bundled workflow fixtures invalid")
}

fn purpose_instruction(purpose: &str) -> &'static str {
    match purpose {
        "summary" => "次の文章の要点を整理してください。",
        "perspectives" => "次の文章について、一般的な観点を示してください。個別の事情を推測しないでください。",
        "support_record" => "次の支援記録を、活動・観察・関わりの見出しで読みやすく整えてください。原文にある事実だけを使い、主語・否定・条件・出来事の順序を変えないでください。意図・能力・診断・支援効果を推測せず、足りない事実を補わないでください。",
        _ => "次の文章を、意味を変えずに読みやすく整えてください。",
    }
}

struct Approval {
    id: String,
    case_id: String,
    revision: u64,
    fingerprint: [u8; 32],
    created_ms: u64,
    expires_ms: u64,
}
struct Job {
    id: String,
    revision: u64,
    fingerprint: [u8; 32],
    kind: JobKind,
}

enum JobKind {
    Demo,
    #[cfg(not(target_arch = "wasm32"))]
    Local {
        model_fingerprint: String,
    },
}

const REJECTED_INPUT_REASON: &str = "受け付けられなかった入力が残っています。文章・辞書・設定を上限内に修正してから、もう一度確認してください。以前の文章では処理を続けません。";

// Inspect only edit metadata of an already received oversized request. Document
// values are skipped by IgnoredAny, never allocated as an original or accepted.
// We must read the complete map: a later invalid dictionary field cannot vanish
// merely because the original was the first oversized field in that same edit.
fn rejected_edit_fields(raw: &str) -> Vec<&'static str> {
    #[derive(Default)]
    struct FieldNames(Vec<&'static str>);
    impl<'de> Deserialize<'de> for FieldNames {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct NamesVisitor;
            impl<'de> serde::de::Visitor<'de> for NamesVisitor {
                type Value = FieldNames;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("an edit field map")
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<FieldNames, A::Error> {
                    let mut fields = Vec::new();
                    while let Some(name) = map.next_key::<String>()? {
                        let known = match name.as_str() {
                            "original" => Some("original"),
                            "purpose" => Some("purpose"),
                            "destination" => Some("destination"),
                            "mode" => Some("mode"),
                            "dictionaryText" => Some("dictionaryText"),
                            "policyVersion" => Some("policyVersion"),
                            "modelConfig" => Some("modelConfig"),
                            "transformed" => Some("transformed"),
                            _ => None,
                        };
                        if let Some(name) = known {
                            if !fields.contains(&name) {
                                fields.push(name);
                            }
                        } else {
                            return Err(serde::de::Error::custom("unsupported edit metadata"));
                        }
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                    Ok(FieldNames(fields))
                }
            }
            deserializer.deserialize_map(NamesVisitor)
        }
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Metadata {
        op: String,
        #[serde(default)]
        patch: FieldNames,
    }
    match serde_json::from_str::<Metadata>(raw) {
        Ok(meta) if matches!(meta.op.as_str(), "update" | "cancel" | "new_case") => meta.patch.0,
        _ => Vec::new(),
    }
}

pub struct Core {
    snapshot: Snapshot,
    // Native host's nonsecret delivery identity. No JSON patch, UI snapshot or
    // saved consultation can supply or recover a delivery permission.
    delivery_config: String,
    approval: Option<Approval>,
    job: Option<Job>,
    // Per-case mapping survives revisions, but is never exposed via an IPC command.
    tokens: BTreeMap<String, String>,
    exported_tokens: BTreeSet<String>,
    has_exported: bool,
    // Set ONLY by a successful native-host worker transaction, never by serde/IPC.
    local_validation: Option<[u8; 32]>,
    // Not cleared by cancel. Only an explicitly accepted value repairs its field.
    rejected_fields: BTreeSet<&'static str>,
}

fn random_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| "端末の乱数を取得できないため、処理を停止しました。".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn now_ms() -> u64 {
    #[cfg(all(target_arch = "wasm32", feature = "wasm"))]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "wasm")))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

impl Core {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            snapshot: Snapshot {
                case_id: random_id()?,
                revision: 0,
                phase: Phase::Draft,
                original: String::new(),
                purpose: "polish".into(),
                destination: "ChatGPT".into(),
                mode: "demo".into(),
                transformed: String::new(),
                findings: vec![],
                residual: vec![],
                lost: vec![],
                blocked_reason: None,
                approval_id: None,
                job_id: None,
                restored: String::new(),
                unknown_tokens: vec![],
                test_copy: None,
                dictionary_text: DEFAULT_DICTIONARY.into(),
                policy_version: POLICY.into(),
                model_config: MODEL.into(),
            },
            delivery_config: String::new(),
            approval: None,
            job: None,
            tokens: BTreeMap::new(),
            exported_tokens: BTreeSet::new(),
            has_exported: false,
            local_validation: None,
            rejected_fields: BTreeSet::new(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    pub fn dispatch_json(&mut self, request: &str) -> Result<String, String> {
        if request.len() > MAX_REQUEST_BYTES {
            self.mark_rejected_edit(&rejected_edit_fields(request));
            return Err("操作データが上限を超えています。文章を短くしてください。".into());
        }
        let request: Request = match serde_json::from_str(request) {
            Ok(value) => value,
            Err(_) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(request) {
                    if matches!(
                        value.get("op").and_then(|v| v.as_str()),
                        Some("update" | "cancel" | "new_case")
                    ) {
                        let keys = value
                            .get("patch")
                            .and_then(|v| v.as_object())
                            .map(|p| p.keys().map(String::as_str).collect::<Vec<_>>())
                            .unwrap_or_default();
                        self.mark_rejected_edit(&keys);
                    }
                }
                return Err("操作の形式を確認できませんでした。コピーは行っていません。".into());
            }
        };
        let snapshot = self.dispatch(request)?;
        serde_json::to_string(&snapshot).map_err(|_| "画面情報を準備できませんでした。".to_string())
    }

    pub fn dispatch(&mut self, request: Request) -> Result<Snapshot, String> {
        match request {
            Request::Snapshot {} => {}
            Request::Update { patch } => self.update(patch)?,
            Request::BeginScan {} => self.begin_scan()?,
            Request::FinishScan { job_id, fault } => self.finish_scan(&job_id, fault)?,
            Request::Cancel {} => {
                self.invalidate();
                if self.rejected_fields.is_empty() {
                    self.snapshot.phase = Phase::Draft;
                } else {
                    self.block(REJECTED_INPUT_REASON);
                }
            }
            Request::Approve { revision } => self.approve(revision, now_ms())?,
            Request::DemoExport { approval_id } => self.demo_export(&approval_id, now_ms())?,
            Request::ExportPrepared { approval_id } => {
                self.export_prepared(&approval_id)?;
            }
            Request::Restore { answer } => self.restore(&answer)?,
            Request::RestoreIfCurrent {
                answer,
                case_id,
                revision,
            } => {
                // Check and mutation are one Core transaction under the native Mutex.
                // SDK-side preflight alone cannot prevent a change between awaits.
                if self.snapshot.case_id != case_id
                    || self.snapshot.revision != revision
                    || self.snapshot.phase != Phase::Exported
                    || self.snapshot.approval_id.is_none()
                {
                    return Err(
                        "回答を待つ間に相談または文章が変わりました。古い回答は復元していません。"
                            .into(),
                    );
                }
                self.restore(&answer)?;
            }
            Request::NewCase {} => {
                let mut next = Self::new()?;
                // Delivery/account selection belongs to the trusted host session,
                // not to an individual consultation or its old approval.
                next.delivery_config = self.delivery_config.clone();
                *self = next;
            }
        }
        Ok(self.snapshot())
    }

    fn invalidate(&mut self) {
        self.local_validation = None;
        self.approval = None;
        self.job = None;
        self.snapshot.approval_id = None;
        self.snapshot.job_id = None;
        self.snapshot.test_copy = None;
        self.snapshot.blocked_reason = None;
        self.snapshot.restored.clear();
        self.snapshot.unknown_tokens.clear();
    }

    fn block(&mut self, reason: &str) {
        self.invalidate();
        self.snapshot.phase = Phase::Blocked;
        self.snapshot.transformed.clear();
        self.snapshot.findings.clear();
        self.snapshot.residual.clear();
        self.snapshot.lost.clear();
        self.snapshot.blocked_reason = Some(reason.into());
    }

    /// A native host calls this when an edit fails before typed Request parsing.
    /// Names only; the rejected payload itself is never retained or logged.
    pub fn mark_rejected_edit(&mut self, field_names: &[&str]) -> Snapshot {
        let fields: Vec<_> = field_names
            .iter()
            .filter_map(|name| match *name {
                "original" => Some("original"),
                "purpose" => Some("purpose"),
                "destination" => Some("destination"),
                "mode" => Some("mode"),
                "dictionaryText" => Some("dictionary_text"),
                "policyVersion" => Some("policy_version"),
                "modelConfig" => Some("model_config"),
                "transformed" => Some("transformed"),
                _ => None,
            })
            .collect();
        if fields.is_empty() {
            self.rejected_fields.extend([
                "original",
                "purpose",
                "destination",
                "mode",
                "dictionary_text",
                "policy_version",
            ]);
        } else {
            self.rejected_fields.extend(fields);
        }
        self.block(REJECTED_INPUT_REASON);
        self.snapshot()
    }

    fn update(&mut self, patch: Patch) -> Result<(), String> {
        let mut invalid = BTreeSet::new();
        if patch.purpose.as_ref().is_some_and(|v| {
            !["polish", "summary", "perspectives", "support_record"].contains(&v.as_str())
        }) {
            invalid.insert("purpose");
        }
        if patch
            .destination
            .as_ref()
            .is_some_and(|v| !["ChatGPT", "Claude", "Gemini"].contains(&v.as_str()))
        {
            invalid.insert("destination");
        }
        if patch
            .mode
            .as_ref()
            .is_some_and(|v| !["demo", "local"].contains(&v.as_str()))
        {
            invalid.insert("mode");
        }
        if patch
            .dictionary_text
            .as_ref()
            .is_some_and(|s| s.len() > MAX_DICTIONARY_BYTES)
        {
            invalid.insert("dictionary_text");
        }
        if patch.policy_version.as_ref().is_some_and(|s| s.len() > 128) {
            invalid.insert("policy_version");
        }
        if patch.model_config.as_ref().is_some_and(|s| s.len() > 256) {
            invalid.insert("model_config");
        }
        if patch
            .original
            .as_ref()
            .is_some_and(|s| s.len() > MAX_REQUEST_BYTES / 2)
        {
            invalid.insert("original");
        }
        if patch
            .transformed
            .as_ref()
            .is_some_and(|s| s.len() > MAX_REQUEST_BYTES / 2)
        {
            invalid.insert("transformed");
        }
        if !invalid.is_empty() {
            let enum_error = invalid.contains("purpose")
                || invalid.contains("destination")
                || invalid.contains("mode");
            let reason = if enum_error {
                "目的・送信予定先・実行モードを選び直してください。"
            } else if invalid.contains("original") || invalid.contains("transformed") {
                "文章が入力上限を超えています。短く分けてください。"
            } else if invalid.contains("dictionary_text") {
                "辞書が試作の上限を超えています。コピーはできません。"
            } else {
                "設定データが上限を超えています。"
            };
            self.rejected_fields.extend(invalid);
            self.block(reason);
            return if enum_error {
                Err(reason.into())
            } else {
                Ok(())
            };
        }
        let had_rejected = !self.rejected_fields.is_empty();
        let has_patch = patch.original.is_some()
            || patch.purpose.is_some()
            || patch.destination.is_some()
            || patch.mode.is_some()
            || patch.dictionary_text.is_some()
            || patch.policy_version.is_some()
            || patch.model_config.is_some()
            || patch.transformed.is_some();
        let was_blocked = self.snapshot.phase == Phase::Blocked;
        // An accepted source repair also discards any rejected derived output.
        if patch.original.is_some() {
            self.rejected_fields.remove("transformed");
        }
        let before = self.fingerprint();
        macro_rules! assign {
            ($field:ident) => {
                if let Some(value) = patch.$field {
                    self.snapshot.$field = value;
                    self.rejected_fields.remove(stringify!($field));
                }
            };
        }
        assign!(original);
        assign!(purpose);
        assign!(destination);
        assign!(mode);
        assign!(dictionary_text);
        assign!(policy_version);
        assign!(model_config);
        assign!(transformed);
        if self.fingerprint() != before
            || (self.rejected_fields.is_empty() && (had_rejected || (was_blocked && has_patch)))
        {
            self.snapshot.revision = self
                .snapshot
                .revision
                .checked_add(1)
                .ok_or("更新回数の上限に達しました。新しい相談を開いてください。")?;
            self.invalidate();
            self.snapshot.phase = Phase::Draft;
            self.snapshot.findings.clear();
            self.snapshot.residual.clear();
            self.snapshot.lost.clear();
        }
        if !self.rejected_fields.is_empty() {
            self.block(REJECTED_INPUT_REASON);
        }
        Ok(())
    }

    fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        // Length prefixes prevent ambiguous concatenation of distinct fields.
        for value in [
            &self.snapshot.case_id,
            &self.snapshot.original,
            &self.snapshot.transformed,
            &self.snapshot.purpose,
            &self.snapshot.destination,
            &self.snapshot.mode,
            &self.snapshot.dictionary_text,
            &self.snapshot.policy_version,
            &self.snapshot.model_config,
            &self.delivery_config,
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        hash.finalize().into()
    }

    fn fixture(&self) -> Option<Fixture> {
        synthetic_fixtures()
            .into_iter()
            .chain(workflow_fixtures())
            .find(|f| f.original == self.snapshot.original)
    }

    fn begin_scan(&mut self) -> Result<(), String> {
        if !self.rejected_fields.is_empty() {
            self.block(REJECTED_INPUT_REASON);
            return Ok(());
        }
        if self.snapshot.phase == Phase::Scanning {
            return Err(
                "確認処理はすでに進行中です。完了を待つか、キャンセルしてください。".into(),
            );
        }
        self.invalidate();
        self.snapshot.transformed.clear();
        self.snapshot.findings.clear();
        self.snapshot.residual.clear();
        self.snapshot.lost.clear();
        if self.snapshot.original.trim().is_empty() {
            self.block("文章がまだありません。架空サンプルを置いてから確認してください。");
            return Ok(());
        }
        if self.snapshot.original.chars().count() > cushion_inference::MAX_INPUT_CHARS {
            self.block("試作の入力上限は4,000文字です。文章を短く分けてください。未処理のままコピーはしません。");
            return Ok(());
        }
        if self.snapshot.mode == "local" {
            self.block(cushion_inference::model_readiness().unwrap_err());
            return Ok(());
        }
        if self.snapshot.policy_version != POLICY || self.snapshot.model_config != MODEL {
            self.block("このポリシーまたはモデル構成は検証されていません。既定のデモ設定に戻してください。");
            return Ok(());
        }
        if self.fixture().is_none() {
            self.block("デモでは登録済みの架空サンプルだけを確認できます。実データは入力しないでください。架空サンプルへ戻すか、モデルの準備をお待ちください。");
            return Ok(());
        }
        let id = random_id()?;
        self.job = Some(Job {
            id: id.clone(),
            revision: self.snapshot.revision,
            fingerprint: self.fingerprint(),
            kind: JobKind::Demo,
        });
        self.snapshot.job_id = Some(id);
        self.snapshot.phase = Phase::Scanning;
        Ok(())
    }

    fn finish_scan(&mut self, job_id: &str, fault: Fault) -> Result<(), String> {
        let Some(job) = &self.job else {
            return Ok(());
        }; // A late callback is ignored, including after cancel/new case.
        if job.id != job_id
            || job.revision != self.snapshot.revision
            || job.fingerprint != self.fingerprint()
            || self.snapshot.phase != Phase::Scanning
        {
            return Ok(());
        }
        if !matches!(job.kind, JobKind::Demo) {
            return Err("ローカル推論の結果は専用のホスト経路でのみ受け付けます。".into());
        }
        match fault {
            Fault::Stopped => {
                self.block("確認処理が停止しました。変換文はコピーしていません。原文を残したまま、もう一度確認できます。");
                return Ok(());
            }
            Fault::InvalidJson => {
                self.block("確認結果の形式が壊れているため、コピーできません。原文は残しています。もう一度確認してください。");
                return Ok(());
            }
            Fault::Incomplete => {
                self.block("文章の最後まで確認できませんでした。短く分けて、もう一度確認してください。変換文はコピーしていません。");
                return Ok(());
            }
            Fault::None => {}
        }
        let fixture = self
            .fixture()
            .ok_or("架空サンプルを確認できませんでした。")?;
        if let Some(reason) = fixture.blocked_reason {
            self.block(&reason);
            return Ok(());
        }
        // This synthetic mock supplies NO claim of LLM detection. Even its envelope passes
        // the real structural validator before deterministic dictionary transformation.
        let mock = serde_json::json!({ "schema_version":"1.0", "document_id":self.snapshot.case_id,
            "complete":true, "covered_chars":self.snapshot.original.chars().count(),
            "findings":[], "context_flags":[], "unresolved":[] });
        if cushion_inference::validate(
            &mock.to_string(),
            &self.snapshot.original,
            &self.snapshot.case_id,
        )
        .is_err()
        {
            self.block("確認結果を検証できませんでした。コピーはできません。");
            return Ok(());
        }
        let mut secrets = fixture.secrets;
        for line in self.snapshot.dictionary_text.lines() {
            let value = line.trim();
            if !value.is_empty() && !secrets.iter().any(|s| s == value) {
                secrets.push(value.to_string());
            }
        }
        let ranges = dictionary_ranges(&self.snapshot.original, &secrets);
        let original_chars: Vec<char> = self.snapshot.original.chars().collect();
        let mut output = String::new();
        let mut cursor = 0;
        let mut findings = vec![];
        for (start, end) in ranges {
            output.extend(original_chars[cursor..start].iter());
            let surface: String = original_chars[start..end].iter().collect();
            // Detection normalization never overwrites a value in the restoration table.
            // Explicit alias/entity support is deferred: distinct spellings stay distinct.
            let key = surface.clone();
            let token = if let Some(token) = self.tokens.get(&key) {
                token.clone()
            } else {
                let token = format!("[[CUSHION_{}]]", random_id()?);
                self.tokens.insert(key, token.clone());
                token
            };
            output.push_str(&token);
            findings.push(Finding {
                start,
                end,
                original: surface,
                replacement: token,
                action: "置換".into(),
                reason: "架空のテスト辞書に登録された情報を、この相談専用の仮名に置き換えました。"
                    .into(),
            });
            cursor = end;
        }
        output.extend(original_chars[cursor..].iter());
        // A generated token is not residual source text. Exclude only the tokens
        // actually generated for this revision, never arbitrary token-shaped input.
        let mut residual_body = output.clone();
        for finding in &findings {
            residual_body = residual_body.replace(&finding.replacement, "");
        }
        if !dictionary_ranges(&residual_body, &secrets).is_empty() {
            self.block("登録された情報が変換後の文章に残るため、コピーを止めています。");
            return Ok(());
        }
        let instruction = purpose_instruction(&self.snapshot.purpose);
        self.snapshot.transformed =
            format!("{instruction}\n角括弧の仮名トークンは変更しないでください。\n\n{output}");
        // Framing is also exported information: a user dictionary may mark a word
        // occurring in the purpose instruction as secret. Validate the FULL payload.
        let mut residual_framed = self.snapshot.transformed.clone();
        for finding in &findings {
            residual_framed = residual_framed.replace(&finding.replacement, "");
        }
        if !dictionary_ranges(&residual_framed, &secrets).is_empty() {
            self.block("登録された情報が依頼文を含む変換文に残るため、コピーを止めています。辞書と目的を見直してください。");
            return Ok(());
        }
        self.snapshot.findings = findings;
        self.snapshot.residual = vec!["文章の目的、出来事、否定や条件、日付・数量など、仮名以外の内容は外へ残ります。".into(), "このデモでは文脈上の未知の秘密をAIで検査していません。登録された架空文だけを処理しています。".into()];
        if self.snapshot.original == SAMPLE {
            self.snapshot.residual.insert(
                0,
                "発送日が9月20日から9月23日に変わることと、お知らせを作りたいことは残ります。"
                    .into(),
            );
        }
        self.snapshot.lost = vec!["この変換は仮名への置換です。削除・一般化は行っていません。名前から分かる背景はAIに伝わらず、推測による補完は復元できません。".into()];
        self.snapshot.phase = Phase::Review;
        self.snapshot.job_id = None;
        self.job = None;
        Ok(())
    }

    fn approve(&mut self, revision: u64, now: u64) -> Result<(), String> {
        let allowed_mode = (self.snapshot.mode == "demo" && self.fixture().is_some())
            || (self.snapshot.mode == "local" && self.local_validation == Some(self.fingerprint()));
        if revision != self.snapshot.revision
            || self.snapshot.phase != Phase::Review
            || self.snapshot.transformed.is_empty()
            || !allowed_mode
        {
            return Err(
                "現在の文章はまだ承認できません。最新の変換結果を確認してください。".into(),
            );
        }
        let id = random_id()?;
        self.approval = Some(Approval {
            id: id.clone(),
            case_id: self.snapshot.case_id.clone(),
            revision,
            fingerprint: self.fingerprint(),
            created_ms: now,
            expires_ms: now.saturating_add(APPROVAL_TTL_MS),
        });
        self.snapshot.approval_id = Some(id);
        self.snapshot.phase = Phase::Approved;
        Ok(())
    }

    fn approval_valid(&self, id: &str, now: u64) -> bool {
        self.approval.as_ref().is_some_and(|approval| {
            approval.id == id
                && approval.case_id == self.snapshot.case_id
                && approval.revision == self.snapshot.revision
                && approval.fingerprint == self.fingerprint()
                && now >= approval.created_ms
                && now < approval.expires_ms
                && self.snapshot.phase == Phase::Approved
                && self.job.is_none()
                && self.snapshot.blocked_reason.is_none()
        })
    }

    fn demo_export(&mut self, id: &str, now: u64) -> Result<(), String> {
        if self.snapshot.mode != "demo" || !self.approval_valid(id, now) || self.fixture().is_none()
        {
            return Err("承認が無効です。最新の変換結果を確認してから、もう一度承認してください。コピーはしていません。".into());
        }
        // Only this in-memory test port exists. It cannot touch the OS clipboard.
        self.snapshot.test_copy = Some(self.snapshot.transformed.clone());
        for finding in &self.snapshot.findings {
            if self.snapshot.transformed.contains(&finding.replacement) {
                self.exported_tokens.insert(finding.replacement.clone());
            }
        }
        self.snapshot.phase = Phase::Exported;
        self.has_exported = true;
        Ok(())
    }

    fn export_prepared(&self, _id: &str) -> Result<String, String> {
        // This production boundary remains closed until a verified local worker AND a native
        // clipboard transaction exist. A demo approval never crosses into production.
        if self.snapshot.mode == "demo" {
            Err("デモの文章は本番の持ち出し操作ではコピーできません。テスト用ポートだけで体験できます。".into())
        } else {
            Err(cushion_inference::model_readiness().unwrap_err().into())
        }
    }

    fn restore(&mut self, answer: &str) -> Result<(), String> {
        if answer.chars().count() > MAX_ANSWER_CHARS {
            return Err("回答が24,000文字を超えています。短く分けて復元してください。".into());
        }
        if !self.has_exported {
            return Err("この相談ではまだ仮名を持ち出していません。確認とデモコピーの後で復元してください。".into());
        }
        let mut allowed = BTreeMap::new();
        // Only exact strings registered for tokens actually exported in this case are used.
        for (key, token) in &self.tokens {
            if self.exported_tokens.contains(token) {
                allowed.insert(token.as_str(), key.as_str());
            }
        }
        let mut result = String::new();
        let mut unknown = Vec::new();
        let mut remaining = answer;
        while let Some(start) = remaining.find("[[") {
            result.push_str(&remaining[..start]);
            remaining = &remaining[start..];
            let Some(close) = remaining.find("]]") else {
                unknown.push(remaining.to_string());
                result.push_str(remaining);
                remaining = "";
                break;
            };
            let end = close + 2;
            let candidate = &remaining[..end];
            if let Some(original) = allowed.get(candidate) {
                result.push_str(original);
            } else {
                result.push_str(candidate);
                if !unknown.iter().any(|t| t == candidate) {
                    unknown.push(candidate.to_string());
                }
            }
            remaining = &remaining[end..];
        }
        result.push_str(remaining);
        self.snapshot.restored = result;
        self.snapshot.unknown_tokens = unknown;
        self.snapshot.phase = Phase::RestoredLocal;
        Ok(())
    }
}

fn ignorable(ch: char) -> bool {
    matches!(
        ch,
        '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}'
    )
}
fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| !ignorable(*c))
        .collect::<String>()
        .nfkc()
        .collect()
}

/// Match NFKC/zero-width-normalized characters while preserving original code-point ranges.
/// Conservative union means overlapping dictionary entries protect the entire span.
pub fn dictionary_ranges(original: &str, secrets: &[String]) -> Vec<(usize, usize)> {
    let mut chars = Vec::new();
    let mut offsets = Vec::new();
    let source: Vec<(usize, char)> = original
        .chars()
        .enumerate()
        .filter(|(_, ch)| !ignorable(*ch))
        .collect();
    let filtered: String = source.iter().map(|(_, ch)| ch).collect();
    let mut codepoint = 0;
    for grapheme in filtered.graphemes(true) {
        let count = grapheme.chars().count();
        let start = source[codepoint].0;
        let end = source[codepoint + count - 1].0 + 1;
        for normalized_char in grapheme.nfkc() {
            chars.push(normalized_char);
            offsets.push((start, end));
        }
        codepoint += count;
    }
    let mut ranges = Vec::new();
    for secret in secrets {
        let needle: Vec<char> = normalized(secret).chars().collect();
        if needle.is_empty() || needle.len() > chars.len() {
            continue;
        }
        for (position, window) in chars.windows(needle.len()).enumerate() {
            if window == needle {
                ranges.push((offsets[position].0, offsets[position + needle.len() - 1].1));
            }
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range.0 < last.1 {
                last.1 = last.1.max(range.1);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    pub struct DemoCore {
        core: Core,
    }
    #[wasm_bindgen]
    impl DemoCore {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Result<DemoCore, JsValue> {
            Ok(Self {
                core: Core::new().map_err(|e| JsValue::from_str(&e))?,
            })
        }
        pub fn dispatch(&mut self, request_json: &str) -> Result<String, JsValue> {
            self.core
                .dispatch_json(request_json)
                .map_err(|e| JsValue::from_str(&e))
        }
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    fn prepared() -> Core {
        let mut core = Core::new().unwrap();
        core.update(Patch {
            original: Some(SAMPLE.into()),
            ..Patch::default()
        })
        .unwrap();
        core.begin_scan().unwrap();
        let job = core.snapshot.job_id.clone().unwrap();
        core.finish_scan(&job, Fault::None).unwrap();
        core
    }
    #[test]
    fn approval_expiry_blocks_copy() {
        let mut core = prepared();
        let revision = core.snapshot.revision;
        core.approve(revision, 1_000).unwrap();
        let id = core.snapshot.approval_id.clone().unwrap();
        assert!(core.demo_export(&id, 1_000 + APPROVAL_TTL_MS).is_err());
        assert!(core.snapshot.test_copy.is_none());
    }
    #[test]
    fn backwards_clock_blocks_copy() {
        let mut core = prepared();
        let revision = core.snapshot.revision;
        core.approve(revision, 1_000).unwrap();
        let id = core.snapshot.approval_id.clone().unwrap();
        assert!(core.demo_export(&id, 999).is_err());
    }
    #[test]
    fn full_content_hash_rechecked_at_export() {
        let mut core = prepared();
        let revision = core.snapshot.revision;
        core.approve(revision, 1_000).unwrap();
        let id = core.snapshot.approval_id.clone().unwrap();
        // Simulate an internal bug skipping update(); the export boundary must still reject.
        core.snapshot.transformed.push('。');
        assert!(core.demo_export(&id, 1_001).is_err());
    }
    #[test]
    fn fingerprint_binds_case_and_dictionary() {
        let mut core = prepared();
        let revision = core.snapshot.revision;
        core.approve(revision, 1_000).unwrap();
        let id = core.snapshot.approval_id.clone().unwrap();
        core.snapshot.dictionary_text.push('。');
        assert!(core.demo_export(&id, 1_001).is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fingerprint_binds_native_delivery_even_if_invalidation_is_skipped() {
        let mut core = prepared();
        let revision = core.snapshot.revision;
        core.approve(revision, 1_000).unwrap();
        let id = core.snapshot.approval_id.clone().unwrap();
        // Defend the final boundary even against an internal mutation that
        // accidentally bypasses the native binding method's invalidation.
        core.delivery_config = "a".repeat(64);
        assert!(core.demo_export(&id, 1_001).is_err());
        assert!(core.snapshot.test_copy.is_none());
    }
}
