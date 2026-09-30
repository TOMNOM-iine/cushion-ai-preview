//! Native-host entrypoints. None of these functions is a serde Request or a WASM export.
//! The host verifies model files/runtime/sandbox, then calls these under its Core mutex.
use super::*;
use regex::Regex;
use std::sync::OnceLock;

pub const LOCAL_POLICY: &str = "local-conservative-v2";
const MAX_SESSION_BYTES: usize = 1_048_576;

/// Local-only worker input. Do not serialize to logs, argv, environment, or a browser helper.
pub struct LocalScanJob {
    pub job_id: String,
    pub case_id: String,
    pub revision: u64,
    pub document_id: String,
    pub original: String,
    pub purpose: String,
    pub policy_version: String,
    pub model_fingerprint: String,
}

#[derive(Clone, Copy)]
pub enum LocalFailure {
    Stopped,
    Timeout,
    InvalidOutput,
    Incomplete,
    ModelUnavailable,
}

impl LocalFailure {
    fn message(self) -> &'static str {
        match self {
            Self::Stopped => "ローカル推論が停止しました。原文は残しています。コピーも外部送信もしていません。",
            Self::Timeout => "ローカル推論が制限時間内に終わりませんでした。文章を短くして再確認してください。",
            Self::InvalidOutput => "ローカルモデルの結果を検証できませんでした。形式・分類・原文との対応が確認できないため、持ち出しを止めています。",
            Self::Incomplete => "文章の最後まで確認できませんでした。未処理の部分があるため、持ち出しはできません。",
            Self::ModelUnavailable => "検証済みのローカルモデルを利用できません。モデル・実行環境を確認してから再検査してください。",
        }
    }
}

/// Trusted native-host failure classification only. No arbitrary response, token,
/// server error body or UI-supplied string can enter a Core stop reason here.
#[derive(Clone, Copy)]
pub enum NativeDeliveryFailure {
    Network,
    HttpDenied,
    HttpLimit,
    UsageUnavailable,
    HttpOther,
    HttpNoContent,
    EmptyBody,
    ContentType,
    ContentTypeJson,
    ContentTypeHtml,
    ContentTypeMissing,
    ContentTypeOther,
    Truncated,
    Json,
    EventOrder,
    UnknownEvent,
    UnexpectedOutput,
    Refusal,
    UnsupportedItem,
    UnsupportedPart,
    UnexpectedRole,
    IncompleteMessage,
    EmptyAnswer,
    MalformedOutput,
    ResponseIdentity,
    ByteLimit,
    TextLimit,
    Partial,
    Authentication,
    Disconnected,
    ModelUnavailable,
    Cancelled,
    Unavailable,
}

impl NativeDeliveryFailure {
    fn message(self) -> &'static str {
        match self {
            Self::Network => "ChatGPTへの接続または回答の受信を完了できませんでした（通信）。変換文が届いた可能性があります。自動再送はしません。",
            Self::HttpDenied => "ChatGPT接続の認証または利用権限で要求が拒否されました（認証・利用権限）。接続設定を確認してください。自動再送はしません。",
            Self::HttpLimit => "ChatGPTの利用上限により回答を取得できませんでした（利用上限）。契約の利用状況を確認してください。自動再送はしません。",
            Self::UsageUnavailable => "ChatGPTの利用可能枠を確認できませんでした。接続情報は保持しています。時間をおいて再検査してください。変換文が届いた可能性があります。自動再送はしません。",
            Self::HttpOther => "ChatGPTの要求受付でエラーが返りました（HTTP）。変換文が届いた可能性があります。自動再送はしません。",
            Self::HttpNoContent => "ChatGPTから本文を返さない応答（HTTP 204/205）が返り、回答の完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::EmptyBody => "ChatGPTの応答本文が空のため、回答の完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::ContentType => "ChatGPTの回答が対応する受信形式ではありませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::ContentTypeJson => "ChatGPTから受信形式（JSON）の応答が返り、必要な完了イベントを確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::ContentTypeHtml => "ChatGPTからページ形式（HTML）の応答が返り、回答の完了を確認できませんでした。ページは実行していません。変換文が届いた可能性があります。自動再送はしません。",
            Self::ContentTypeMissing => "ChatGPTの応答に受信形式の指定がなく、回答の完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::ContentTypeOther => "ChatGPTから対応していない受信形式が通知され、回答の完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::Truncated => "ChatGPTの回答の途中で受信が終わり、完了を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::Json => "ChatGPTの応答イベントのJSON形式を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::EventOrder => "ChatGPTの応答イベントの順序を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::UnknownEvent => "ChatGPTからこの接続で対応していない応答イベントが返りました。変換文が届いた可能性があります。自動再送はしません。",
            Self::UnexpectedOutput => "ChatGPTからこの接続で扱えない種類の出力が返りました。変換文が届いた可能性があります。自動再送はしません。",
            Self::Refusal => "ChatGPTの応答に拒否を示す通知が含まれるため、回答の復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::UnsupportedItem => "ChatGPTの応答にこの接続で対応していない出力項目が含まれるため、復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::UnsupportedPart => "ChatGPTの応答にこの接続で対応していない本文要素が含まれるため、復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::UnexpectedRole => "ChatGPTの回答の発言者を確認できないため、復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::IncompleteMessage => "ChatGPTの回答本文の完了状態を確認できないため、復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::EmptyAnswer => "ChatGPTの完了した応答に回答の本文が見つかりませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::MalformedOutput => "ChatGPTの応答に必要な配列や文字列の形式を確認できないため、復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::ResponseIdentity => "ChatGPTの回答と今回の要求の対応を確認できませんでした。変換文が届いた可能性があります。自動再送はしません。",
            Self::ByteLimit => "ChatGPTの回答の受信サイズが上限を超えたため停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::TextLimit => "ChatGPTの完成した回答が24,000文字の上限を超えたため停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::Partial => "ChatGPTの回答が未完了・拒否・失敗のため復元を停止しました。変換文が届いた可能性があります。自動再送はしません。",
            Self::Authentication => "ChatGPTの認証を更新または確認できませんでした。接続設定でもう一度ログインしてください。送信処理は開始していません。",
            Self::Disconnected => "ChatGPTの契約での接続が未完了です。接続設定でログインとモデルを確認してください。送信処理は開始していません。",
            Self::ModelUnavailable => "ローカルモデルの動作を確認できませんでした。モデルを再読込して再検査してください。送信処理は開始していません。",
            Self::Cancelled => "この画面の接続が停止されたため送信を止めました。原文は保持しています。送信処理は開始していません。",
            Self::Unavailable => "送信に必要な設定または処理状態を確認できませんでした。接続設定と現在の文章を確認してください。送信処理は開始していません。",
        }
    }
}

/// This data is plaintext until a native encrypted vault seals it. No IPC Request
/// exposes it. Approval, pending jobs and inspection status deliberately do not exist.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedCaseData {
    schema_version: u32,
    source_case_id: String,
    original: String,
    purpose: String,
    destination: String,
    mode: String,
    dictionary_text: String,
    policy_version: String,
    model_config: String,
    tokens: BTreeMap<String, String>,
    exported_tokens: BTreeSet<String>,
    has_exported: bool,
}

impl Core {
    /// Record a known native delivery failure without changing case/revision.
    /// SDKs may read this same-context fixed reason, never a raw host exception.
    /// The native host must keep the Core lock from export through this stop.
    pub fn stop_native_delivery(&mut self, failure: NativeDeliveryFailure) -> Snapshot {
        self.block(failure.message());
        self.snapshot()
    }

    /// Bind the current delivery mode/account/model/configuration selected by the
    /// trusted native host. The value is either the empty manual/API default or
    /// a nonsecret SHA-256 identity. Never pass credentials or document text.
    /// The host must hold the same Core Mutex used for approval and export.
    /// This entrypoint is intentionally absent from serde Request and WASM.
    pub fn bind_delivery_config(&mut self, config: &str) -> Result<Snapshot, String> {
        if !config.is_empty()
            && (config.len() != 64
                || !config
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
        {
            let reason = "送信先の設定を確認できませんでした。設定を選び直してから再検査してください。コピー・外部送信は行っていません。";
            self.block(reason);
            return Err(reason.into());
        }
        if self.delivery_config == config {
            return Ok(self.snapshot());
        }
        let Some(revision) = self.snapshot.revision.checked_add(1) else {
            let reason = "更新回数の上限に達しました。新しい相談を開いてください。";
            self.block(reason);
            return Err(reason.into());
        };
        self.delivery_config = config.into();
        self.snapshot.revision = revision;
        self.invalidate();
        self.snapshot.phase = Phase::Draft;
        self.snapshot.findings.clear();
        self.snapshot.residual.clear();
        self.snapshot.lost.clear();
        if !self.rejected_fields.is_empty() {
            self.block(REJECTED_INPUT_REASON);
        }
        Ok(self.snapshot())
    }

    /// `ready` and `model_fingerprint` MUST come from a trusted native host's
    /// verified model manifest/runtime, never from a UI field or worker JSON.
    pub fn begin_local_scan(
        &mut self,
        model_fingerprint: &str,
        ready: bool,
    ) -> Result<LocalScanJob, String> {
        if !self.rejected_fields.is_empty() {
            self.block(REJECTED_INPUT_REASON);
            return Err(REJECTED_INPUT_REASON.into());
        }
        if self.snapshot.mode != "local" {
            return Err(
                "実推論にはローカルモードを選んでください。デモでは本番推論を実行しません。".into(),
            );
        }
        if self.snapshot.phase == Phase::Scanning {
            return Err(
                "確認処理はすでに進行中です。キャンセルしてから再確認してください。".into(),
            );
        }
        self.invalidate();
        if !ready || !valid_model_fingerprint(model_fingerprint) {
            self.block(LocalFailure::ModelUnavailable.message());
            return Err(LocalFailure::ModelUnavailable.message().into());
        }
        if self.snapshot.original.trim().is_empty() {
            self.block("原文を入力してから確認してください。");
            return Err("原文がまだありません。".into());
        }
        if self.snapshot.original.chars().count() > cushion_inference::MAX_INPUT_CHARS {
            self.block("実推論の入力上限は4,000文字です。原文を短く分けてください。黙って一部を切り捨てることはありません。");
            return Err("原文が実推論の入力上限を超えています。".into());
        }
        if let Some(reason) = blocked_context(&self.snapshot.original) {
            self.block(reason);
            return Err(reason.into());
        }
        // The host selects this policy and measured model identity. Merely typing
        // either string in the UI cannot set the private validation capability.
        self.update(Patch {
            policy_version: Some(LOCAL_POLICY.into()),
            model_config: Some(model_fingerprint.into()),
            ..Patch::default()
        })?;
        self.snapshot.transformed.clear();
        self.snapshot.findings.clear();
        self.snapshot.residual.clear();
        self.snapshot.lost.clear();
        let id = random_id()?;
        self.job = Some(Job {
            id: id.clone(),
            revision: self.snapshot.revision,
            fingerprint: self.fingerprint(),
            kind: JobKind::Local {
                model_fingerprint: model_fingerprint.into(),
            },
        });
        self.snapshot.job_id = Some(id.clone());
        self.snapshot.phase = Phase::Scanning;
        Ok(LocalScanJob {
            job_id: id.clone(),
            document_id: id,
            case_id: self.snapshot.case_id.clone(),
            revision: self.snapshot.revision,
            original: self.snapshot.original.clone(),
            purpose: self.snapshot.purpose.clone(),
            policy_version: LOCAL_POLICY.into(),
            model_fingerprint: model_fingerprint.into(),
        })
    }

    fn local_job_matches(&self, job_id: &str) -> bool {
        self.job.as_ref().is_some_and(|job| {
            job.id == job_id
                && job.revision == self.snapshot.revision
                && job.fingerprint == self.fingerprint()
                && self.snapshot.phase == Phase::Scanning
                && self.snapshot.mode == "local"
                && matches!(&job.kind, JobKind::Local { model_fingerprint }
                    if model_fingerprint == &self.snapshot.model_config)
        })
    }

    pub fn fail_local_scan(&mut self, job_id: &str, failure: LocalFailure) -> Snapshot {
        if self.local_job_matches(job_id) {
            self.block(failure.message());
        }
        self.snapshot()
    }

    /// Accept bytes from the dedicated local worker only. Revalidates JSON even if
    /// the worker/host claimed it was valid. Stale results never mutate current state.
    pub fn finish_local_scan(
        &mut self,
        job_id: &str,
        worker_json: &str,
    ) -> Result<Snapshot, String> {
        if !self.local_job_matches(job_id) {
            return Ok(self.snapshot());
        }
        let candidates =
            match cushion_inference::validate(worker_json, &self.snapshot.original, job_id) {
                Ok(candidates) => candidates,
                Err(_) => {
                    self.block(LocalFailure::InvalidOutput.message());
                    return Ok(self.snapshot());
                }
            };
        if let Some(reason) = blocked_context(&self.snapshot.original) {
            self.block(reason);
            return Ok(self.snapshot());
        }
        let mut secrets: Vec<String> = self
            .snapshot
            .dictionary_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        // Every occurrence of a model-identified surface is protected, not only
        // the one occurrence the model happened to enumerate.
        for candidate in candidates {
            if !secrets.contains(&candidate.surface) {
                secrets.push(candidate.surface);
            }
        }
        let mut ranges = dictionary_ranges(&self.snapshot.original, &secrets);
        ranges.extend(deterministic_ranges(&self.snapshot.original));
        let ranges = merge_ranges(ranges);
        let original: Vec<char> = self.snapshot.original.chars().collect();
        let mut output = String::new();
        let mut findings = Vec::new();
        let mut cursor = 0;
        for (start, end) in ranges {
            if start >= end || end > original.len() || start < cursor {
                self.block(LocalFailure::InvalidOutput.message());
                return Ok(self.snapshot());
            }
            output.extend(original[cursor..start].iter());
            let surface: String = original[start..end].iter().collect();
            let token = match self.tokens.get(&surface) {
                Some(token) => token.clone(),
                None => {
                    let token = loop {
                        let value = format!("[[CUSHION_{}]]", random_id()?);
                        if !self.tokens.values().any(|old| old == &value) {
                            break value;
                        }
                    };
                    self.tokens.insert(surface.clone(), token.clone());
                    token
                }
            };
            output.push_str(&token);
            findings.push(Finding { start, end, original: surface,
                replacement: token, action: "置換".into(),
                reason: "辞書・ローカル推論の候補・数値や連絡先の規則を合わせ、この相談専用の仮名に置き換えました。".into() });
            cursor = end;
        }
        output.extend(original[cursor..].iter());
        let instruction = purpose_instruction(&self.snapshot.purpose);
        let transformed = format!("{instruction}\n角括弧の仮名トークンは変更しないでください。仮名の数値を推測・計算しないでください。\n\n{output}");
        // Omit only tokens generated for this inspected revision from the residual
        // scan, so their random hex digits are not mistaken for leaked numbers.
        let mut residual = transformed.clone();
        for finding in &findings {
            residual = residual.replace(&finding.replacement, "");
        }
        if !dictionary_ranges(&residual, &secrets).is_empty()
            || !deterministic_ranges(&residual).is_empty()
        {
            self.block("依頼文を含む変換文に、登録された情報や数値・連絡先が残っています。持ち出しを止めました。");
            return Ok(self.snapshot());
        }
        self.snapshot.transformed = transformed;
        self.snapshot.findings = findings;
        self.snapshot.residual = vec![
            "仮名以外の出来事・関係・依頼の目的・否定や条件は外へ残ります。固有名詞を隠しても、文脈から推測される可能性があります。".into(),
            "ローカルモデルと規則で候補を検査しましたが、未知の秘密の見落としがないことを保証するものではありません。全文を確認してください。".into(),
        ];
        self.snapshot.lost = vec![
            "数値・日付・数量も既定で仮名に置き換えています。外部AIは具体値の計算や日付の比較をできません。登録トークンの完全一致だけを戻し、推測された値は復元しません。".into(),
            "削除や自由な一般化は行っていません。文脈自体が秘密の場合は持ち出さず、原文または目的を見直してください。".into(),
        ];
        if self.snapshot.purpose == "support_record" {
            self.snapshot.residual.push("活動の種類、観察した行動、職員の関わり、否定や条件、出来事の順序は外へ残ります。組合せで個人を推測できないか確認してください。".into());
            self.snapshot.lost.push("氏名・施設名・具体的な日付・数量を隠します。能力や支援効果の評価、診断の推測、事実の補完を外部AIに求めない依頼です。".into());
        }
        self.snapshot.phase = Phase::Review;
        self.snapshot.job_id = None;
        self.job = None;
        self.local_validation = Some(self.fingerprint());
        Ok(self.snapshot())
    }

    /// The host must hold its Core Mutex for the entire call. The callback receives
    /// ONLY the final transformed text. It must not append unreviewed history or tools.
    /// Exported-token registration occurs only after the effect reports success.
    /// Until independent model/operational acceptance, only compiled synthetic
    /// fixtures with fully protected oracle spans may cross this prototype port.
    pub fn export_approved_with<T, F>(&mut self, approval_id: &str, effect: F) -> Result<T, String>
    where
        F: FnOnce(&str) -> Result<T, String>,
    {
        if self.snapshot.mode != "local"
            || self.local_validation != Some(self.fingerprint())
            || !self.approval_valid(approval_id, now_ms())
        {
            return Err(
                "有効な実推論の検査と承認を確認できません。コピー・外部送信は行っていません。"
                    .into(),
            );
        }
        if !self.prototype_export_permitted() {
            let reason = "独立した1,000件評価と運用受入が未完了のため、本番の持ち出しは停止中です。試作では登録済みの架空文が完全一致し、正解の機密範囲をすべて隠せた場合だけコピー・API送信を試せます。";
            self.block(reason);
            return Err(reason.into());
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            effect(&self.snapshot.transformed)
        }));
        let value = match result {
            Ok(Ok(value)) => value,
            Ok(Err(_)) | Err(_) => {
                self.invalidate();
                self.snapshot.phase = Phase::Review;
                // Preserve completed inspection after failed effect, but revoke the
                // one-time approval. User must inspect/approve again to retry.
                self.local_validation = Some(self.fingerprint());
                return Err("コピーまたは送信処理の成功を確認できませんでした。承認は失効しました。外部側で受け付けられたかを確認してから、再度承認してください。".into());
            }
        };
        for finding in &self.snapshot.findings {
            if self.snapshot.transformed.contains(&finding.replacement) {
                self.exported_tokens.insert(finding.replacement.clone());
            }
        }
        self.snapshot.phase = Phase::Exported;
        self.has_exported = true;
        Ok(value)
    }

    fn prototype_export_permitted(&self) -> bool {
        let Some(fixture) = self.fixture() else {
            return false;
        };
        if fixture.expected_state != "REVIEW" {
            return false;
        }
        // The fixture oracle is a FINAL STOP guard. It never augments model
        // candidates, the secret dictionary, or the result used for evaluation.
        let expected: BTreeSet<usize> = fixture
            .expected_ranges
            .iter()
            .map(|range| (range.start, range.end))
            .chain(dictionary_ranges(&fixture.original, &fixture.prohibited))
            .flat_map(|(start, end)| start..end)
            .collect();
        let covered: BTreeSet<usize> = self
            .snapshot
            .findings
            .iter()
            .filter(|finding| {
                finding.action == "置換"
                    && self.tokens.get(&finding.original) == Some(&finding.replacement)
                    && self.snapshot.transformed.contains(&finding.replacement)
            })
            .flat_map(|finding| finding.start..finding.end)
            .collect();
        if !expected.is_subset(&covered) {
            return false;
        }
        let mut residual = self.snapshot.transformed.clone();
        for finding in &self.snapshot.findings {
            residual = residual.replace(&finding.replacement, "");
        }
        dictionary_ranges(&residual, &fixture.prohibited).is_empty()
    }

    /// Plaintext bytes for a native encrypted vault ONLY. The Core performs no IO.
    pub fn export_session(&self) -> Result<Vec<u8>, String> {
        if !self.rejected_fields.is_empty() {
            return Err(REJECTED_INPUT_REASON.into());
        }
        let saved = SavedCaseData {
            schema_version: 1,
            source_case_id: self.snapshot.case_id.clone(),
            original: self.snapshot.original.clone(),
            purpose: self.snapshot.purpose.clone(),
            destination: self.snapshot.destination.clone(),
            mode: self.snapshot.mode.clone(),
            dictionary_text: self.snapshot.dictionary_text.clone(),
            policy_version: self.snapshot.policy_version.clone(),
            model_config: self.snapshot.model_config.clone(),
            tokens: self.tokens.clone(),
            exported_tokens: self.exported_tokens.clone(),
            has_exported: self.has_exported,
        };
        validate_saved(&saved)?;
        let bytes =
            serde_json::to_vec(&saved).map_err(|_| "相談を保存用に整理できませんでした。")?;
        if bytes.len() > MAX_SESSION_BYTES {
            return Err("相談の保存サイズが上限を超えています。".into());
        }
        Ok(bytes)
    }

    /// Input MUST already have passed the encrypted vault's authentication check.
    /// Validation is transactional: invalid input leaves the current case untouched.
    pub fn import_session(&mut self, bytes: &[u8]) -> Result<Snapshot, String> {
        if bytes.len() > MAX_SESSION_BYTES {
            return Err("保存相談のサイズが上限を超えています。".into());
        }
        let saved: SavedCaseData =
            serde_json::from_slice(bytes).map_err(|_| "保存相談の形式を確認できませんでした。")?;
        validate_saved(&saved)?;
        let mut loaded = Core::new()?;
        // Saved data cannot choose a delivery account or restore its permission.
        // Keep the currently selected trusted host configuration for this run.
        loaded.delivery_config = self.delivery_config.clone();
        // A new running case identity rejects every old job/approval response.
        // Token mappings still represent the same authenticated saved consultation.
        loaded.snapshot.original = saved.original;
        loaded.snapshot.purpose = saved.purpose;
        loaded.snapshot.destination = saved.destination;
        loaded.snapshot.mode = saved.mode;
        loaded.snapshot.dictionary_text = saved.dictionary_text;
        loaded.snapshot.policy_version = saved.policy_version;
        loaded.snapshot.model_config = saved.model_config;
        loaded.tokens = saved.tokens;
        loaded.exported_tokens = saved.exported_tokens;
        loaded.has_exported = saved.has_exported;
        *self = loaded;
        Ok(self.snapshot())
    }
}

fn valid_model_fingerprint(value: &str) -> bool {
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    hex.len() == 64 && hex.bytes().all(|c| c.is_ascii_hexdigit())
}

fn blocked_context(text: &str) -> Option<&'static str> {
    let folded = normalized(text).to_lowercase();
    if text
        .chars()
        .any(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        || folded.contains("[[cushion_")
    {
        return Some("方向制御文字または既存の技術トークンを含むため、原文の対応を確定できません。新しい文章として修正してください。");
    }
    // Explicit identity ambiguity cannot be solved by a name->token table.
    // This is a conservative language rule, not a lookup of evaluation truth.
    let identity_subject = ["担当者", "人物", "人名", "候補", "同じ名前", "同じ氏名"]
        .iter()
        .any(|word| folded.contains(word));
    let unresolved_identity = [
        "同定できない",
        "同定できません",
        "同定ができない",
        "同定ができません",
        "特定できない",
        "特定できません",
        "特定ができない",
        "特定ができません",
        "区別できない",
        "区別できません",
        "区別がつかない",
        "区別がつきません",
    ]
    .iter()
    .any(|word| folded.contains(word));
    if folded.contains("同姓同名") || (identity_subject && unresolved_identity) {
        return Some("誰を指すか、同じ名前が同じ人かを確定できない記述があります。推測して対応表を作れないため停止しました。原文の人物・候補の区別を確認してから見直してください。");
    }
    if [
        "パスワード",
        "秘密鍵",
        "認証情報",
        "apiキー",
        "api key",
        "api_key",
        "private key",
        "password",
        "access_token",
        "bearer ",
    ]
    .iter()
    .any(|word| folded.contains(word))
    {
        return Some("認証情報を含む可能性があるため、コピー・外部送信を止めています。認証情報を除いた文章に作り直してください。");
    }
    if [
        "診断",
        "服薬",
        "病歴",
        "家庭事情",
        "家庭状況",
        "虐待",
        "要配慮",
        "アレルギー",
        "障害名",
        "medical diagnosis",
        "medication",
        "medical history",
    ]
    .iter()
    .any(|word| folded.contains(word))
    {
        return Some("診断・服薬・詳細な家庭事情などの高機密情報を含む可能性があります。初版では持ち出さず、個別情報を使わない目的に見直してください。");
    }
    if [
        "未発表",
        "非公開計画",
        "極秘",
        "存在自体が秘密",
        "撤退計画",
        "confidential plan",
        "unannounced",
    ]
    .iter()
    .any(|word| folded.contains(word))
    {
        return Some("計画や関係そのものが秘密の可能性があります。名前を変えるだけでは持ち出せないため、ここで止めています。");
    }
    None
}

fn patterns() -> &'static Vec<Regex> {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| [
        r"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
        r#"(?:https?://|www\.)[^\s<>"'、。]+"#,
        r"[+＋]?[0-9０-９][0-9０-９ ()（）.．/／:：,，\-－ー]{0,60}[0-9０-９]",
        r"[0-9０-９]+(?:[.,，．][0-9０-９]+)*",
        r"[〇零一二三四五六七八九十百千万億兆]+(?:円|人|個|箱|台|件|日|月|年|時|分|割|倍|％|パーセント)",
    ].iter().map(|pattern| Regex::new(pattern).expect("static privacy pattern invalid")).collect())
}

fn deterministic_ranges(text: &str) -> Vec<(usize, usize)> {
    let folded = normalized(text);
    let mut surfaces = Vec::new();
    for pattern in patterns() {
        for matched in pattern.find_iter(&folded) {
            surfaces.push(matched.as_str().to_owned());
        }
    }
    let mut ranges = dictionary_ranges(text, &surfaces);
    // Non-ASCII numeric characters (e.g. Arabic digits and Roman numerals) must
    // not escape a rule that only recognizes normalized western notation.
    ranges.extend(
        text.chars()
            .enumerate()
            .filter(|(_, ch)| ch.is_numeric())
            .map(|(i, _)| (i, i + 1)),
    );
    merge_ranges(ranges)
}

fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if range.0 < previous.1 {
                previous.1 = previous.1.max(range.1);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

fn valid_hex_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn valid_token(value: &str) -> bool {
    value
        .strip_prefix("[[CUSHION_")
        .and_then(|s| s.strip_suffix("]]"))
        .is_some_and(valid_hex_id)
}
fn validate_saved(saved: &SavedCaseData) -> Result<(), String> {
    let valid = saved.schema_version == 1
        && valid_hex_id(&saved.source_case_id)
        && saved.original.len() <= MAX_REQUEST_BYTES / 2
        && saved.dictionary_text.len() <= MAX_DICTIONARY_BYTES
        && saved.policy_version.len() <= 128
        && saved.model_config.len() <= 256
        && ["polish", "summary", "perspectives", "support_record"]
            .contains(&saved.purpose.as_str())
        && ["ChatGPT", "Claude", "Gemini"].contains(&saved.destination.as_str())
        && ["demo", "local"].contains(&saved.mode.as_str())
        && saved.tokens.len() <= 4096
        && saved.exported_tokens.len() <= 4096
        && (saved.has_exported || saved.exported_tokens.is_empty())
        && saved.tokens.iter().all(|(original, token)| {
            !original.is_empty()
                && original.chars().count() <= cushion_inference::MAX_INPUT_CHARS
                && valid_token(token)
        })
        && saved.tokens.values().collect::<BTreeSet<_>>().len() == saved.tokens.len()
        && saved
            .exported_tokens
            .iter()
            .all(|token| saved.tokens.values().any(|known| known == token));
    if !valid {
        return Err(
            "保存相談の内容や対応表を検証できませんでした。現在の相談は変更していません。".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod final_export_guard_tests {
    use super::*;

    #[test]
    fn expected_stop_fixture_cannot_export_even_after_an_internal_inspection_bug() {
        let fixture = synthetic_fixtures()
            .into_iter()
            .find(|fixture| fixture.id == "unknown")
            .unwrap();
        let mut core = Core::new().unwrap();
        // Simulate a bug in inspection, without exposing a bypass in the public API.
        // Even an otherwise valid approval must not let a known stop case export.
        core.snapshot.mode = "local".into();
        core.snapshot.original = fixture.original;
        core.snapshot.transformed = "架空の検査バグを想定した文です。".into();
        core.snapshot.phase = Phase::Review;
        core.local_validation = Some(core.fingerprint());
        core.approve(core.snapshot.revision, now_ms()).unwrap();
        let approval = core.snapshot.approval_id.clone().unwrap();
        assert!(core
            .export_approved_with(&approval, |_| -> Result<(), String> {
                panic!("the external effect must never run")
            })
            .is_err());
        assert!(core.snapshot.phase == Phase::Blocked);
        assert!(!core.has_exported);
    }
}
