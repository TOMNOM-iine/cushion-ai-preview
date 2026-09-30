//! A dedicated offline process. All text uses inherited pipes, never HTTP or argv.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

#[path = "model_profiles.rs"]
mod model_profiles;
pub use model_profiles::{ModelProfile, DEFAULT_MODEL};

#[cfg(test)]
const BASELINE_MODEL_SHA: &str = "b58fe056b5435070240de259f3f981aa38fee96825bbd78c088d5fd90e46f2b5";
// Approval binds the entire local + outbound pipeline, not only model weights.
// Bump this whenever provider wire/auth/approval policy or Core local
// policy changes. Existing inspections/approvals must then be performed again.
pub const CPU_PIPELINE_CONTRACT: &str = "core-local-conservative-v2|workflow:support-record-v1|identity-ambiguity-stop-v2|delivery-bind-v1|siwc-plan-v7|pkce-jwt-keychain|plan:strict-sse-body-completed-reasoning-annotations-discard|accept:event-stream|delivery-stop:typed-empty-http-v3|plan-framing:bom-crlf-v1|plan-diagnostics:shape-output-v2|plan-answer:stream-ledger-v1|plan-cancel:independent-generation-v1|plan-wire:1048576|plan-answer:24000|api-v1|openai:gpt-4.1-mini|claude:claude-sonnet-4-6|single-user-text|store:false|response:2048|backend:cpu|threads:8|gpu-layers:0|kv-offload:false|op-offload:false|timeout:240";
// Fixed Metal default was promoted only after real 9B + Core delivery checks.
// Never retry CPU after a Metal failure under one approval.
pub const PIPELINE_CONTRACT: &str = "core-local-conservative-v2|workflow:support-record-v1|identity-ambiguity-stop-v2|delivery-bind-v1|siwc-plan-v7|pkce-jwt-keychain|plan:strict-sse-body-completed-reasoning-annotations-discard|accept:event-stream|delivery-stop:typed-empty-http-v3|plan-framing:bom-crlf-v1|plan-diagnostics:shape-output-v2|plan-answer:stream-ledger-v1|plan-cancel:independent-generation-v1|plan-wire:1048576|plan-answer:24000|api-v1|openai:gpt-4.1-mini|claude:claude-sonnet-4-6|single-user-text|store:false|response:2048|runtime:metal-precompiled-v2|extract:surface-set-v1|view:drop-five-invisibles-v1|backend:metal|threads:8|gpu-layers:99|kv-offload:true|op-offload:true|ctx:8192|out:1024|greedy|timeout:240";
const METAL_DIRECTORY: &str = ".tools/llama/metal-precompiled-b11146/build/bin";
const METAL_FILES: &[&str] = &[
    "cushion-worker",
    "default.metallib",
    "ggml-tensor.metallib",
    "libllama.dylib",
    "libllama.0.dylib",
    "libllama.0.5.0.dylib",
    "libggml.dylib",
    "libggml.0.dylib",
    "libggml.0.25.1.dylib",
    "libggml-base.dylib",
    "libggml-base.0.dylib",
    "libggml-base.0.25.1.dylib",
    "libggml-cpu.dylib",
    "libggml-cpu.0.dylib",
    "libggml-cpu.0.25.1.dylib",
    "libggml-blas.dylib",
    "libggml-blas.0.dylib",
    "libggml-blas.0.25.1.dylib",
    "libggml-metal.dylib",
    "libggml-metal.0.dylib",
    "libggml-metal.0.25.1.dylib",
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeProfile {
    CpuRelease,
    MetalPrecompiled,
}
impl RuntimeProfile {
    fn worker(self) -> String {
        match self {
            Self::CpuRelease => ".tools/llama/cushion-worker".into(),
            Self::MetalPrecompiled => format!("{METAL_DIRECTORY}/cushion-worker"),
        }
    }
    fn libraries(self) -> &'static str {
        match self {
            Self::CpuRelease => ".tools/llama/llama-b11146",
            Self::MetalPrecompiled => METAL_DIRECTORY,
        }
    }
    fn lock(self) -> &'static str {
        match self {
            Self::CpuRelease => "models/runtime-lock.json",
            Self::MetalPrecompiled => ".tools/llama/metal-precompiled-b11146/runtime-lock.json",
        }
    }
    fn required(self) -> Vec<String> {
        match self {
            Self::CpuRelease => REQUIRED_RUNTIME.iter().map(|p| p.to_string()).collect(),
            Self::MetalPrecompiled => METAL_FILES
                .iter()
                .map(|p| format!("{METAL_DIRECTORY}/{p}"))
                .collect(),
        }
    }
    fn version(self) -> u32 {
        match self {
            Self::CpuRelease => 1,
            Self::MetalPrecompiled => 2,
        }
    }
    fn contract(self) -> &'static str {
        match self {
            Self::CpuRelease => CPU_PIPELINE_CONTRACT,
            Self::MetalPrecompiled => PIPELINE_CONTRACT,
        }
    }
    fn backend(self) -> &'static str {
        match self {
            Self::CpuRelease => "cpu",
            Self::MetalPrecompiled => "metal",
        }
    }
    fn valid_ready(self, value: &serde_json::Value) -> bool {
        *value
            == serde_json::json!({"ready":true,"contextTokens":8192,"maxOutputTokens":1024,"backend":self.backend(),"threads":8,"gpuLayers":if self == Self::CpuRelease {0} else {99}})
    }
}
const MAX_LOCK_BYTES: usize = 65_536;
const MAX_RUNTIME_FILE_BYTES: u64 = 536_870_912;
const MAX_RUNTIME_TOTAL_BYTES: u64 = 2_147_483_648;
const REQUIRED_RUNTIME: &[&str] = &[
    ".tools/llama/cushion-worker",
    ".tools/llama/llama-b11146/libllama.dylib",
    ".tools/llama/llama-b11146/libllama.0.dylib",
    ".tools/llama/llama-b11146/libggml.dylib",
    ".tools/llama/llama-b11146/libggml.0.dylib",
    ".tools/llama/llama-b11146/libggml-base.dylib",
    ".tools/llama/llama-b11146/libggml-base.0.dylib",
    ".tools/llama/llama-b11146/libggml-cpu.dylib",
    ".tools/llama/llama-b11146/libggml-cpu.0.dylib",
    ".tools/llama/llama-b11146/libggml-blas.dylib",
    ".tools/llama/llama-b11146/libggml-blas.0.dylib",
    ".tools/llama/llama-b11146/libggml-metal.dylib",
    ".tools/llama/llama-b11146/libggml-metal.0.dylib",
    ".tools/llama/llama-b11146/libggml-rpc.dylib",
    ".tools/llama/llama-b11146/libggml-rpc.0.dylib",
];
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: String,
    pub model_name: String,
    pub model_config: Option<String>,
    pub reason: Option<String>,
    pub context_tokens: Option<usize>,
    pub source_verified: bool,
    pub network_isolated: bool,
}
impl ModelStatus {
    pub fn initial() -> Self {
        Self {
            state: "verifying".into(),
            model_name: DEFAULT_MODEL.artifact().name.clone(),
            model_config: None,
            reason: Some("モデルのファイルと推論環境を確認しています。".into()),
            context_tokens: Some(8192),
            source_verified: false,
            network_isolated: false,
        }
    }
    pub fn ready(fingerprint: &str) -> Self {
        Self {
            state: "ready".into(),
            model_config: Some(fingerprint.into()),
            reason: None,
            source_verified: true,
            network_isolated: true,
            ..Self::initial()
        }
    }
    pub fn failed(message: &str) -> Self {
        Self {
            state: "unavailable".into(),
            reason: Some(message.into()),
            ..Self::initial()
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeLock {
    version: u32,
    files: Vec<LockedFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LockedFile {
    path: String,
    sha256: String,
}
fn parse_lock_for(bytes: &[u8], profile: RuntimeProfile) -> Result<RuntimeLock, String> {
    if bytes.len() > MAX_LOCK_BYTES {
        return Err(fixed_err());
    }
    let lock: RuntimeLock = serde_json::from_slice(bytes).map_err(|_| fixed_err())?;
    let required = profile.required();
    if lock.version != profile.version()
        || lock.files.len() < required.len()
        || lock.files.len() > 100
        || (profile == RuntimeProfile::MetalPrecompiled && lock.files.len() != required.len())
    {
        return Err(fixed_err());
    }
    let mut paths = std::collections::BTreeSet::new();
    for file in &lock.files {
        let safe_library = file
            .path
            .strip_prefix(".tools/llama/llama-b11146/")
            .is_some_and(|name| {
                name.starts_with("lib")
                    && name.ends_with(".dylib")
                    && !name.contains("..")
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
            });
        let allowed = match profile {
            RuntimeProfile::CpuRelease => safe_library || file.path == profile.worker(),
            RuntimeProfile::MetalPrecompiled => required.contains(&file.path),
        };
        if !allowed
            || !paths.insert(file.path.as_str())
            || file.sha256.len() != 64
            || !file
                .sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(fixed_err());
        }
    }
    if required.iter().any(|path| !paths.contains(path.as_str())) {
        return Err(fixed_err());
    }
    Ok(lock)
}
#[cfg(test)]
fn pipeline_fingerprint(lock_bytes: &[u8], contract: &str) -> String {
    model_pipeline_fingerprint(lock_bytes, contract, ModelProfile::Qwen9bQ8)
}
fn model_pipeline_fingerprint(lock_bytes: &[u8], contract: &str, model: ModelProfile) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}|{}|{contract}",
                model.artifact().sha256,
                format_args!("{:x}", Sha256::digest(lock_bytes))
            )
            .as_bytes()
        )
    )
}
fn digest(path: &Path) -> Result<String, String> {
    let mut f = File::open(path)
        .map_err(|_| "モデル・実行ファイルが見つかりません。導入スクリプトを実行してください。")?;
    let mut hash = Sha256::new();
    let mut b = [0u8; 131072];
    loop {
        let n = f
            .read(&mut b)
            .map_err(|_| "ファイルを確認できませんでした。")?;
        if n == 0 {
            break;
        }
        hash.update(&b[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn fixed_err() -> String {
    "ローカル推論を続けられません。原文は送信していません。モデルを再読込してください。".into()
}
fn verify_runtime_files(
    root: &Path,
    lock: &RuntimeLock,
    profile: RuntimeProfile,
) -> Result<(), String> {
    let library_root = root
        .join(if profile == RuntimeProfile::CpuRelease {
            ".tools/llama"
        } else {
            METAL_DIRECTORY
        })
        .canonicalize()
        .map_err(|_| fixed_err())?;
    let mut total = 0_u64;
    for file in &lock.files {
        let path = root
            .join(&file.path)
            .canonicalize()
            .map_err(|_| fixed_err())?;
        let metadata = path.metadata().map_err(|_| fixed_err())?;
        if !path.starts_with(&library_root)
            || !metadata.is_file()
            || metadata.len() > MAX_RUNTIME_FILE_BYTES
        {
            return Err(fixed_err());
        }
        total = total.checked_add(metadata.len()).ok_or_else(fixed_err)?;
        if total > MAX_RUNTIME_TOTAL_BYTES || digest(&path)? != file.sha256 {
            return Err("推論プログラムのSHA-256が一致しません。再導入してください。".into());
        }
    }
    Ok(())
}
fn verified_model_path(root: &Path, model: ModelProfile) -> Result<PathBuf, String> {
    let directory = root
        .join("models")
        .canonicalize()
        .map_err(|_| fixed_err())?;
    let expected = directory.join(&model.artifact().file);
    let canonical = expected.canonicalize().map_err(|_| fixed_err())?;
    let metadata = canonical.metadata().map_err(|_| fixed_err())?;
    if canonical != expected || !metadata.is_file() || metadata.len() != model.artifact().bytes {
        return Err("登録モデルのファイル・容量を確認できません。処理を停止しました。".into());
    }
    Ok(canonical)
}
fn sandbox(root: &Path, model: ModelProfile) -> Result<String, String> {
    let quoted = |p: &Path| serde_json::to_string(&p.to_string_lossy()).map_err(|_| fixed_err());
    let libs = root.join(".tools/llama");
    let model = root.join("models").join(&model.artifact().file);
    Ok(format!("(version 1) (allow default) (deny network*) (deny file-write*) (deny process-fork) (deny file-read* (subpath \"/Users\")) (allow file-read* (subpath {}) (literal {}))",quoted(&libs)?,quoted(&model)?))
}
fn command(root: &Path, profile: RuntimeProfile, model: ModelProfile) -> Result<Command, String> {
    let mut c = Command::new("/usr/bin/sandbox-exec");
    c.arg("-p")
        .arg(sandbox(root, model)?)
        .arg(root.join(profile.worker()));
    c.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GGML_METAL_LOG_LEVEL", "0")
        .current_dir("/")
        .stderr(Stdio::null());
    Ok(c)
}
fn bounded_network_probe(mut command: Command, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| fixed_err())?;
    let output = match child.stdout.take() {
        Some(output) => output,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(fixed_err());
        }
    };
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output.take(33).read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(result);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(fixed_err());
            }
        }
    };
    let bytes = rx
        .recv_timeout(Duration::from_millis(100))
        .map_err(|_| fixed_err())?
        .map_err(|_| fixed_err())?;
    if !status.success() || bytes != b"blocked\n" {
        return Err(fixed_err());
    }
    Ok(())
}
pub struct Runtime {
    child: Child,
    input: ChildStdin,
    rx: Receiver<Result<String, String>>,
    pub fingerprint: String,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Runtime {
    /// Nonblocking process check for the host's final copy/API permission gate.
    /// An OS error is unavailable, never an assumption that the worker is alive.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub fn start(root: &Path) -> Result<Self, String> {
        Self::start_profile(root, RuntimeProfile::MetalPrecompiled, DEFAULT_MODEL)
    }
    /// Explicit evaluation entrypoint for the same pinned Metal profile.
    /// No arbitrary executable/library path or automatic fallback is accepted.
    pub fn start_metal_candidate(root: &Path) -> Result<Self, String> {
        Self::start_profile(root, RuntimeProfile::MetalPrecompiled, DEFAULT_MODEL)
    }
    /// Explicit CLI evaluation only. All models remain compile-time hash pinned;
    /// model selection never bypasses Core approval or the synthetic-only gate.
    pub fn start_evaluation(root: &Path, model: ModelProfile) -> Result<Self, String> {
        Self::start_profile(root, RuntimeProfile::MetalPrecompiled, model)
    }
    fn start_profile(
        root: &Path,
        profile: RuntimeProfile,
        model: ModelProfile,
    ) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|_| fixed_err())?;
        let model_path = verified_model_path(&root, model)?;
        if digest(&model_path)? != model.artifact().sha256 {
            return Err("モデルのSHA-256が配布記録と一致しません。コピーを停止しました。".into());
        }
        let mut lock_bytes = Vec::new();
        File::open(root.join(profile.lock()))
            .map_err(|_| fixed_err())?
            .take((MAX_LOCK_BYTES + 1) as u64)
            .read_to_end(&mut lock_bytes)
            .map_err(|_| fixed_err())?;
        let lock = parse_lock_for(&lock_bytes, profile)?;
        verify_runtime_files(&root, &lock, profile)?;
        let mut probe = command(&root, profile, model)?;
        probe.arg("--probe-network");
        bounded_network_probe(probe, Duration::from_secs(5)).map_err(|_| {
            "推論ワーカーのネットワーク制限を確認できません。処理を停止しました。".to_string()
        })?;
        let mut child = command(&root, profile, model)?
            .arg(model_path)
            .arg(root.join(profile.libraries()))
            .arg(format!("--{}", profile.backend()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| fixed_err())?;
        let input = child.stdin.take().ok_or_else(fixed_err)?;
        let output = child.stdout.take().ok_or_else(fixed_err)?;
        let (tx, rx) = mpsc::sync_channel(2);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let result = reader.by_ref().take(65537).read_until(b'\n', &mut bytes);
                let line = match result {
                    Ok(n) if n > 0 && n <= 65536 && bytes.last() == Some(&b'\n') => {
                        String::from_utf8(bytes).map_err(|_| fixed_err())
                    }
                    _ => Err(fixed_err()),
                };
                let failed = line.is_err();
                if tx.send(line).is_err() || failed {
                    break;
                }
            }
        });
        let fingerprint = model_pipeline_fingerprint(&lock_bytes, profile.contract(), model);
        let rt = Self {
            child,
            input,
            rx,
            fingerprint,
        };
        let ready = rt
            .rx
            .recv_timeout(Duration::from_secs(120))
            .map_err(|_| fixed_err())??;
        let v: serde_json::Value = serde_json::from_str(&ready).map_err(|_| fixed_err())?;
        if !profile.valid_ready(&v) {
            return Err(fixed_err());
        }
        Ok(rt)
    }
    pub fn infer(
        &mut self,
        document: &str,
        id: &str,
        cancel: &AtomicBool,
    ) -> Result<InferenceResult, String> {
        if document.chars().count() > super::MAX_INPUT_CHARS
            || document.is_empty()
            || id.len() > 128
        {
            return Err("入力上限を超えています。短くして再検査してください。".into());
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(fixed_err());
        }
        let request = serde_json::json!({"document":document,"document_id":id}).to_string() + "\n";
        self.input
            .write_all(request.as_bytes())
            .and_then(|_| self.input.flush())
            .map_err(|_| fixed_err())?;
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            if cancel.load(Ordering::SeqCst) || Instant::now() > deadline {
                let _ = self.child.kill();
                return Err("推論を中止しました。モデルの再読込後に再検査してください。".into());
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(line)) => {
                    let v: serde_json::Value =
                        serde_json::from_str(&line).map_err(|_| fixed_err())?;
                    if v.get("ok") != Some(&serde_json::Value::Bool(true)) {
                        return Err(
                            "推論が完了しませんでした。未処理のままコピーすることはできません。"
                                .into(),
                        );
                    }
                    let result: InferenceResult =
                        serde_json::from_value(v).map_err(|_| fixed_err())?;
                    if !result.ok
                        || result
                            .input_tokens
                            .checked_add(1024)
                            .is_none_or(|n| n > 8192)
                        || result.output_tokens > 1024
                    {
                        return Err(fixed_err());
                    }
                    if result.result.to_string().len() > super::MAX_WORKER_BYTES {
                        return Err(fixed_err());
                    }
                    return Ok(result);
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => return Err(fixed_err()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct InferenceResult {
    pub ok: bool,
    pub result: serde_json::Value,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub elapsed_ms: u64,
}

pub fn development_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn model_changes_bind_approval_and_reject_unregistered_ids() {
        let mut hashes = std::collections::BTreeSet::new();
        for model in ModelProfile::ALL {
            assert_eq!(ModelProfile::from_id(model.id()), Some(model));
            assert!(hashes.insert(model_pipeline_fingerprint(
                b"same runtime",
                PIPELINE_CONTRACT,
                model
            )));
            let artifact = model.artifact();
            assert_eq!(artifact.sha256.len(), 64);
            assert!(artifact.sha256.bytes().all(|c| c.is_ascii_hexdigit()));
            assert_eq!(Path::new(&artifact.file).components().count(), 1);
        }
        assert_eq!(ModelProfile::Qwen9bQ8.artifact().sha256, BASELINE_MODEL_SHA);
        for unknown in [
            "",
            "latest",
            "../../model",
            "qwen3.5-4b",
            "https://example.test/model.gguf",
        ] {
            assert!(ModelProfile::from_id(unknown).is_none());
        }
        assert_eq!(
            model_pipeline_fingerprint(b"same runtime", PIPELINE_CONTRACT, ModelProfile::Qwen9bQ8),
            format!(
                "{:x}",
                Sha256::digest(
                    format!(
                        "{BASELINE_MODEL_SHA}|{:x}|{PIPELINE_CONTRACT}",
                        Sha256::digest(b"same runtime")
                    )
                    .as_bytes()
                )
            )
        );
    }
    #[test]
    fn comparison_sandbox_only_permits_the_selected_weight_file() {
        let root = Path::new("/cushion-test/project");
        for selected in ModelProfile::ALL {
            let policy = sandbox(root, selected).unwrap();
            assert!(policy.contains("(deny network*)"));
            assert!(policy.contains("(deny file-write*)"));
            assert!(policy.contains("(deny process-fork)"));
            for other in ModelProfile::ALL {
                assert_eq!(policy.contains(&other.artifact().file), other == selected);
            }
        }
    }
    #[test]
    fn model_path_refuses_wrong_size_and_symlink_substitution() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        std::fs::create_dir(&models).unwrap();
        let profile = ModelProfile::Qwen4bQ4;
        let file = models.join(&profile.artifact().file);
        std::fs::write(&file, b"synthetic-truncated-weight").unwrap();
        assert!(verified_model_path(dir.path(), profile).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&file).unwrap();
            let outside = dir.path().join("outside.gguf");
            File::create(&outside)
                .unwrap()
                .set_len(profile.artifact().bytes)
                .unwrap();
            std::os::unix::fs::symlink(&outside, &file).unwrap();
            assert!(verified_model_path(dir.path(), profile).is_err());
        }
    }
    #[test]
    #[cfg(unix)]
    fn runtime_liveness_rejects_a_terminated_synthetic_child() {
        let mut child = Command::new("/bin/sleep")
            .arg("10")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let (_tx, rx) = mpsc::sync_channel(1);
        let mut runtime = Runtime {
            child,
            input,
            rx,
            fingerprint: "synthetic-only".into(),
        };
        assert!(runtime.is_alive());
        runtime.child.kill().unwrap();
        runtime.child.wait().unwrap();
        assert!(!runtime.is_alive());
        assert!(!runtime.is_alive());
    }
    fn manifest() -> serde_json::Value {
        serde_json::json!({"version":1,"files":REQUIRED_RUNTIME.iter().map(|path|serde_json::json!({"path":path,"sha256":"a".repeat(64)})).collect::<Vec<_>>()})
    }
    fn accepts(v: &serde_json::Value) -> bool {
        parse_lock_for(&serde_json::to_vec(v).unwrap(), RuntimeProfile::CpuRelease).is_ok()
    }
    fn metal_manifest() -> serde_json::Value {
        serde_json::json!({"version":2,"files":RuntimeProfile::MetalPrecompiled.required().iter().map(|path|serde_json::json!({"path":path,"sha256":format!("{:x}",Sha256::digest(b"synthetic-public-runtime"))})).collect::<Vec<_>>()})
    }
    fn metal_lock(value: &serde_json::Value) -> Result<RuntimeLock, String> {
        parse_lock_for(
            &serde_json::to_vec(value).unwrap(),
            RuntimeProfile::MetalPrecompiled,
        )
    }
    #[test]
    fn metal_manifest_requires_exact_worker_libraries_and_both_shaders() {
        let valid = metal_manifest();
        assert!(metal_lock(&valid).is_ok());
        for index in 0..METAL_FILES.len() {
            let mut missing = valid.clone();
            missing["files"].as_array_mut().unwrap().remove(index);
            assert!(metal_lock(&missing).is_err());
        }
        for filename in [
            "libggml-rpc.dylib",
            "extra.metallib",
            "../outside",
            "cushion-worker",
        ] {
            let mut extra = valid.clone();
            extra["files"].as_array_mut().unwrap().push(serde_json::json!({"path":format!("{METAL_DIRECTORY}/{filename}"),"sha256":"a".repeat(64)}));
            assert!(metal_lock(&extra).is_err());
        }
        let mut old = valid.clone();
        old["version"] = 1.into();
        assert!(metal_lock(&old).is_err());
        assert!(parse_lock_for(
            &serde_json::to_vec(&valid).unwrap(),
            RuntimeProfile::CpuRelease
        )
        .is_err());
    }
    #[test]
    fn metal_shader_bytes_are_verified_and_outside_symlinks_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimeProfile::MetalPrecompiled.required();
        std::fs::create_dir_all(dir.path().join(METAL_DIRECTORY)).unwrap();
        for path in &paths {
            std::fs::write(dir.path().join(path), b"synthetic-public-runtime").unwrap();
        }
        let lock = metal_lock(&metal_manifest()).unwrap();
        assert!(verify_runtime_files(dir.path(), &lock, RuntimeProfile::MetalPrecompiled).is_ok());
        let shader = dir.path().join(METAL_DIRECTORY).join("default.metallib");
        std::fs::write(&shader, b"modified-public-shader").unwrap();
        assert!(verify_runtime_files(dir.path(), &lock, RuntimeProfile::MetalPrecompiled).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&shader).unwrap();
            let outside = dir.path().join("outside-public-fixture");
            std::fs::write(&outside, b"synthetic-public-runtime").unwrap();
            std::os::unix::fs::symlink(&outside, &shader).unwrap();
            assert!(
                verify_runtime_files(dir.path(), &lock, RuntimeProfile::MetalPrecompiled).is_err()
            );
        }
    }
    #[test]
    fn backend_ready_and_approval_identity_cannot_cross_cpu_and_metal() {
        let metal = serde_json::json!({"ready":true,"contextTokens":8192,"maxOutputTokens":1024,"backend":"metal","threads":8,"gpuLayers":99});
        assert!(RuntimeProfile::MetalPrecompiled.valid_ready(&metal));
        assert!(!RuntimeProfile::CpuRelease.valid_ready(&metal));
        for key in [
            "backend",
            "threads",
            "gpuLayers",
            "contextTokens",
            "maxOutputTokens",
        ] {
            let mut wrong = metal.clone();
            wrong[key] = serde_json::Value::Null;
            assert!(!RuntimeProfile::MetalPrecompiled.valid_ready(&wrong));
        }
        let mut extra = metal.clone();
        extra["other"] = true.into();
        assert!(!RuntimeProfile::MetalPrecompiled.valid_ready(&extra));
        assert_ne!(
            pipeline_fingerprint(b"same", CPU_PIPELINE_CONTRACT),
            pipeline_fingerprint(b"same", PIPELINE_CONTRACT)
        );
    }
    #[test]
    #[cfg(unix)]
    fn network_probe_protocol_is_bounded_and_requires_exact_success() {
        let mut success = Command::new("/bin/echo");
        success.arg("blocked");
        assert!(bounded_network_probe(success, Duration::from_secs(1)).is_ok());
        let mut failure = Command::new("/bin/echo");
        failure.arg("not-blocked");
        assert!(bounded_network_probe(failure, Duration::from_secs(1)).is_err());
        let mut hang = Command::new("/bin/sleep");
        hang.arg("10");
        let started = Instant::now();
        assert!(bounded_network_probe(hang, Duration::from_millis(30)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn runtime_manifest_requires_every_loaded_library_and_worker() {
        let valid = manifest();
        assert!(accepts(&valid));
        for index in 0..REQUIRED_RUNTIME.len() {
            let mut missing = valid.clone();
            missing["files"].as_array_mut().unwrap().remove(index);
            assert!(!accepts(&missing));
        }
    }
    #[test]
    fn runtime_manifest_refuses_duplicate_paths_not_only_file_count() {
        let mut value = manifest();
        let duplicate = value["files"][0].clone();
        value["files"].as_array_mut().unwrap().push(duplicate);
        assert!(!accepts(&value));
    }
    #[test]
    fn runtime_manifest_refuses_path_escape_unsafe_names_bad_hash_and_extra_fields() {
        for path in [
            "../outside",
            ".tools/llama/../../outside",
            ".tools/llama/llama-b11146/sub/libx.dylib",
            ".tools/llama/llama-b11146/libx.dylib/",
            ".tools/llama/llama-b11146/libx\u{0}.dylib",
        ] {
            let mut value = manifest();
            value["files"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"path":path,"sha256":"a".repeat(64)}));
            assert!(!accepts(&value));
        }
        let mut value = manifest();
        value["files"][0]["sha256"] = "wrong".into();
        assert!(!accepts(&value));
        let mut value = manifest();
        value["unexpected"] = true.into();
        assert!(!accepts(&value));
    }
    #[test]
    fn runtime_manifest_is_bounded_and_pipeline_config_changes_fingerprint() {
        assert!(
            parse_lock_for(&vec![b' '; MAX_LOCK_BYTES + 1], RuntimeProfile::CpuRelease).is_err()
        );
        let bytes = serde_json::to_vec(&manifest()).unwrap();
        assert_ne!(
            pipeline_fingerprint(&bytes, PIPELINE_CONTRACT),
            pipeline_fingerprint(&bytes, "different outbound model")
        );
        assert_ne!(
            pipeline_fingerprint(&bytes, PIPELINE_CONTRACT),
            pipeline_fingerprint(b"other runtime", PIPELINE_CONTRACT)
        );
    }
}
