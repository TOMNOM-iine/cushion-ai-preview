//! A strict, local-only worker boundary for pinned native model configurations.
//! JSON validity is not a claim of semantic accuracy or complete secret detection.
use serde::{Deserialize, Serialize};

mod surface_set;

pub const MAX_WORKER_BYTES: usize = 32_768;
pub const MAX_INPUT_CHARS: usize = 4_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub sentence_id: String,
    pub surface: String,
    pub occurrence: usize,
    pub category: Category,
    pub suggested_action: SuggestedAction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Category {
    Person,
    Organization,
    Identifier,
    Contact,
    ConfidentialContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestedAction {
    Tokenize,
    Remove,
    Block,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerOutput {
    pub schema_version: String,
    pub document_id: String,
    pub complete: bool,
    pub covered_chars: usize,
    pub findings: Vec<Finding>,
    pub context_flags: Vec<String>,
    pub unresolved: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ValidationError {
    TooLarge,
    InvalidJson,
    Version,
    Document,
    Incomplete,
    Unresolved,
    Context,
    Surface,
    Range,
    Duplicate,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedFinding {
    pub start: usize,
    pub end: usize,
    pub surface: String,
}

/// Returns Unicode code-point ranges, never UTF-16 or byte offsets.
/// The trusted caller must compare actual job/revision before calling this function.
pub fn validate(
    raw: &str,
    document: &str,
    expected_id: &str,
) -> Result<Vec<ValidatedFinding>, ValidationError> {
    if raw.len() > MAX_WORKER_BYTES || document.chars().count() > MAX_INPUT_CHARS {
        return Err(ValidationError::TooLarge);
    }
    // Select a version, then deserialize the ORIGINAL bytes strictly. Parsing via
    // a Value first would silently erase duplicate JSON keys.
    #[derive(Deserialize)]
    struct Version {
        schema_version: String,
    }
    let version: Version = serde_json::from_str(raw).map_err(|_| ValidationError::InvalidJson)?;
    if version.schema_version == "surface-set-v1" {
        return surface_set::validate(raw, document, expected_id);
    }
    let output: WorkerOutput =
        serde_json::from_str(raw).map_err(|_| ValidationError::InvalidJson)?;
    if output.schema_version != "1.0" {
        return Err(ValidationError::Version);
    }
    if output.document_id != expected_id {
        return Err(ValidationError::Document);
    }
    if !output.complete || output.covered_chars != document.chars().count() {
        return Err(ValidationError::Incomplete);
    }
    if !output.unresolved.is_empty() {
        return Err(ValidationError::Unresolved);
    }
    if !output.context_flags.is_empty() {
        return Err(ValidationError::Context);
    }
    if output.findings.len() > 128 {
        return Err(ValidationError::TooLarge);
    }
    let mut found: Vec<ValidatedFinding> = Vec::new();
    for f in output.findings {
        if f.surface.is_empty() || f.surface.chars().count() > 256 || f.occurrence == 0 {
            return Err(ValidationError::Surface);
        }
        // Initial contract is one document/chunk. Unknown sentence references fail closed.
        if f.sentence_id != "s001" {
            return Err(ValidationError::Range);
        }
        if !matches!(f.suggested_action, SuggestedAction::Tokenize)
            || matches!(f.category, Category::ConfidentialContext)
        {
            return Err(ValidationError::Action);
        }
        let (byte, _) = document
            .match_indices(&f.surface)
            .nth(f.occurrence - 1)
            .ok_or(ValidationError::Surface)?;
        let start = document[..byte].chars().count();
        let end = start + f.surface.chars().count();
        if found.iter().any(|old| start < old.end && end > old.start) {
            return Err(ValidationError::Duplicate);
        }
        found.push(ValidatedFinding {
            start,
            end,
            surface: f.surface,
        });
    }
    found.sort_by_key(|v| v.start);
    Ok(found)
}

pub fn model_readiness() -> Result<(), &'static str> {
    Err("ローカルモデルが未準備です。検証済みのモデルと推論ワーカーを接続するまでコピーできません。原文はこのPC内に残しています。")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json() -> serde_json::Value {
        serde_json::json!({"schema_version":"1.0","document_id":"case","complete":true,"covered_chars":4,"findings":[{"sentence_id":"s001","surface":"架空","occurrence":1,"category":"PERSON","suggested_action":"TOKENIZE"}],"context_flags":[],"unresolved":[]})
    }
    fn check(v: serde_json::Value) -> Result<Vec<ValidatedFinding>, ValidationError> {
        validate(&v.to_string(), "😀架空名", "case")
    }
    #[test]
    fn unicode_codepoint_range() {
        assert_eq!(check(json()).unwrap()[0].start, 1);
    }
    #[test]
    fn empty_json_blocks() {
        assert_eq!(validate("", "", "case"), Err(ValidationError::InvalidJson));
    }
    #[test]
    fn truncated_json_blocks() {
        assert_eq!(validate("{", "", "case"), Err(ValidationError::InvalidJson));
    }
    #[test]
    fn unknown_category_blocks() {
        let mut v = json();
        v["findings"][0]["category"] = "EXECUTE".into();
        assert_eq!(check(v), Err(ValidationError::InvalidJson));
    }
    #[test]
    fn extra_field_blocks() {
        let mut v = json();
        v["approved"] = true.into();
        assert_eq!(check(v), Err(ValidationError::InvalidJson));
    }
    #[test]
    fn wrong_document_blocks() {
        let mut v = json();
        v["document_id"] = "other".into();
        assert_eq!(check(v), Err(ValidationError::Document));
    }
    #[test]
    fn incomplete_blocks() {
        let mut v = json();
        v["complete"] = false.into();
        assert_eq!(check(v), Err(ValidationError::Incomplete));
    }
    #[test]
    fn coverage_blocks() {
        let mut v = json();
        v["covered_chars"] = 3.into();
        assert_eq!(check(v), Err(ValidationError::Incomplete));
    }
    #[test]
    fn unresolved_blocks() {
        let mut v = json();
        v["unresolved"] = serde_json::json!(["unknown"]);
        assert_eq!(check(v), Err(ValidationError::Unresolved));
    }
    #[test]
    fn context_blocks() {
        let mut v = json();
        v["context_flags"] = serde_json::json!(["MEDICAL"]);
        assert_eq!(check(v), Err(ValidationError::Context));
    }
    #[test]
    fn nonexistent_surface_blocks() {
        let mut v = json();
        v["findings"][0]["surface"] = "存在しない".into();
        assert_eq!(check(v), Err(ValidationError::Surface));
    }
    #[test]
    fn zero_occurrence_blocks() {
        let mut v = json();
        v["findings"][0]["occurrence"] = 0.into();
        assert_eq!(check(v), Err(ValidationError::Surface));
    }
    #[test]
    fn nonexistent_occurrence_blocks() {
        let mut v = json();
        v["findings"][0]["occurrence"] = 2.into();
        assert_eq!(check(v), Err(ValidationError::Surface));
    }
    #[test]
    fn overlapping_candidates_blocks() {
        let mut v = json();
        let duplicate = v["findings"][0].clone();
        v["findings"].as_array_mut().unwrap().push(duplicate);
        assert_eq!(check(v), Err(ValidationError::Duplicate));
    }
    #[test]
    fn wrong_schema_blocks() {
        let mut v = json();
        v["schema_version"] = "2.0".into();
        assert_eq!(check(v), Err(ValidationError::Version));
    }
    #[test]
    fn oversized_output_blocks() {
        assert_eq!(
            validate(&" ".repeat(MAX_WORKER_BYTES + 1), "", "case"),
            Err(ValidationError::TooLarge)
        );
    }
    #[test]
    fn worker_cannot_approve_removal() {
        let mut v = json();
        v["findings"][0]["suggested_action"] = "REMOVE".into();
        assert_eq!(check(v), Err(ValidationError::Action));
    }
    #[test]
    fn unknown_sentence_blocks() {
        let mut v = json();
        v["findings"][0]["sentence_id"] = "s999".into();
        assert_eq!(check(v), Err(ValidationError::Range));
    }
    #[test]
    fn no_runtime_never_ready() {
        assert!(model_readiness().is_err());
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;
