//! Unique candidate surfaces over a reversible, local-only inspection view.
//! The LLM does not count occurrences, choose offsets, or authorize an export.
use super::{Category, ValidatedFinding, ValidationError};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    surface: String,
    category: Category,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    schema_version: String,
    document_id: String,
    complete: bool,
    covered_chars: usize,
    findings: Vec<Candidate>,
    context_flags: Vec<String>,
    unresolved: Vec<String>,
}

fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
    )
}

pub(super) fn validate(
    raw: &str,
    original: &str,
    id: &str,
) -> Result<Vec<ValidatedFinding>, ValidationError> {
    let result: Output = serde_json::from_str(raw).map_err(|_| ValidationError::InvalidJson)?;
    if result.schema_version != "surface-set-v1" {
        return Err(ValidationError::Version);
    }
    if result.document_id != id {
        return Err(ValidationError::Document);
    }
    let source: Vec<char> = original.chars().collect();
    let offsets: Vec<usize> = source
        .iter()
        .enumerate()
        .filter_map(|(i, c)| (!invisible(*c)).then_some(i))
        .collect();
    let view: Vec<char> = offsets.iter().map(|i| source[*i]).collect();
    if view.is_empty() || !result.complete || result.covered_chars != view.len() {
        return Err(ValidationError::Incomplete);
    }
    if !result.context_flags.is_empty() {
        return Err(ValidationError::Context);
    }
    if !result.unresolved.is_empty() {
        return Err(ValidationError::Unresolved);
    }
    if result.findings.len() > 128 {
        return Err(ValidationError::TooLarge);
    }
    let mut surfaces = BTreeSet::new();
    let mut found: Vec<ValidatedFinding> = Vec::new();
    for candidate in result.findings {
        if matches!(candidate.category, Category::ConfidentialContext) {
            return Err(ValidationError::Action);
        }
        let needle: Vec<char> = candidate.surface.chars().collect();
        if needle.is_empty()
            || needle.len() > 256
            || needle.len() > view.len()
            || needle.iter().any(|c| invisible(*c))
        {
            return Err(ValidationError::Surface);
        }
        if !surfaces.insert(candidate.surface) {
            return Err(ValidationError::Duplicate);
        }
        let mut matched = false;
        for (i, window) in view.windows(needle.len()).enumerate() {
            if window != needle {
                continue;
            }
            matched = true;
            let (start, end) = (offsets[i], offsets[i + needle.len() - 1] + 1);
            if found.iter().any(|old| start < old.end && end > old.start) {
                return Err(ValidationError::Duplicate);
            }
            found.push(ValidatedFinding {
                start,
                end,
                surface: source[start..end].iter().collect(),
            });
        }
        if !matched {
            return Err(ValidationError::Surface);
        }
    }
    found.sort_by_key(|v| v.start);
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(original: &str, surfaces: &[&str]) -> serde_json::Value {
        serde_json::json!({"schema_version":"surface-set-v1","document_id":"job","complete":true,
            "covered_chars":original.chars().filter(|c| !invisible(*c)).count(),
            "findings":surfaces.iter().map(|s|serde_json::json!({"surface":s,"category":"ORGANIZATION"})).collect::<Vec<_>>(),
            "context_flags":[],"unresolved":[]})
    }
    #[test]
    fn all_occurrences_map_back_to_exact_original_unicode() {
        let text = "😀架\u{200b}空堂と架空堂と架\u{2060}空堂";
        let result =
            super::super::validate(&payload(text, &["架空堂"]).to_string(), text, "job").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(
            (result[0].start, result[0].end, result[0].surface.as_str()),
            (1, 5, "架\u{200b}空堂")
        );
        for f in result {
            assert_eq!(
                text.chars()
                    .skip(f.start)
                    .take(f.end - f.start)
                    .collect::<String>(),
                f.surface
            );
        }
    }
    #[test]
    fn every_ignored_character_is_mapped_without_changing_visible_spelling() {
        for ch in ['\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}'] {
            let text = format!("Ａ{ch}Ｂ");
            assert_eq!(
                validate(&payload(&text, &["ＡＢ"]).to_string(), &text, "job").unwrap()[0].surface,
                text
            );
            assert_eq!(
                validate(&payload(&text, &["AB"]).to_string(), &text, "job"),
                Err(ValidationError::Surface)
            );
        }
    }
    #[test]
    fn duplicate_overlap_hallucination_and_unknown_fields_stop() {
        for surfaces in [vec!["架空堂", "架空堂"], vec!["架空堂", "架空"]] {
            assert_eq!(
                validate(&payload("架空堂", &surfaces).to_string(), "架空堂", "job"),
                Err(ValidationError::Duplicate)
            );
        }
        assert_eq!(
            validate(&payload("架空堂", &["別の堂"]).to_string(), "架空堂", "job"),
            Err(ValidationError::Surface)
        );
        let mut value = payload("架空堂", &["架空堂"]);
        value["findings"][0]["occurrence"] = 1.into();
        assert_eq!(
            validate(&value.to_string(), "架空堂", "job"),
            Err(ValidationError::InvalidJson)
        );
    }
    #[test]
    fn incomplete_unresolved_context_wrong_job_or_coverage_stop() {
        for (key, value, expected) in [
            (
                "complete",
                serde_json::json!(false),
                ValidationError::Incomplete,
            ),
            (
                "covered_chars",
                serde_json::json!(2),
                ValidationError::Incomplete,
            ),
            (
                "document_id",
                serde_json::json!("other"),
                ValidationError::Document,
            ),
            (
                "unresolved",
                serde_json::json!(["UNCERTAIN"]),
                ValidationError::Unresolved,
            ),
            (
                "context_flags",
                serde_json::json!(["HIGH_CONFIDENTIALITY"]),
                ValidationError::Context,
            ),
        ] {
            let mut v = payload("架空堂", &["架空堂"]);
            v[key] = value;
            assert_eq!(validate(&v.to_string(), "架空堂", "job"), Err(expected));
        }
    }
    #[test]
    fn duplicate_json_keys_are_not_silently_accepted() {
        let raw = payload("架空堂", &["架空堂"]).to_string().replacen(
            "\"complete\":true",
            "\"complete\":false,\"complete\":true",
            1,
        );
        assert_eq!(
            super::super::validate(&raw, "架空堂", "job"),
            Err(ValidationError::InvalidJson)
        );
    }
}
