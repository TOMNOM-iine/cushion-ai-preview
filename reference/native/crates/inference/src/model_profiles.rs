//! Compile-time pinned local models. Never accepts an arbitrary path or URL.
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelProfile {
    Qwen9bQ8,
    Qwen9bQ4,
    Qwen4bQ8,
    Qwen4bQ4,
}
// Development-fixture speed comparison only; the Core prototype export gate
// remains in place. Q4 had no consistent advantage over Q8 on this Mac.
pub const DEFAULT_MODEL: ModelProfile = ModelProfile::Qwen4bQ8;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ModelArtifact {
    pub id: String,
    pub name: String,
    pub official: String,
    pub official_revision: String,
    pub quantizer: String,
    pub revision: String,
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_manifest_matches_compiled_default() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../models/manifest.json")).unwrap();
        let model = DEFAULT_MODEL.artifact();
        assert_eq!(manifest["model"], model.official);
        assert_eq!(manifest["officialRevision"], model.official_revision);
        assert_eq!(manifest["quantizer"], model.quantizer);
        assert_eq!(manifest["quantizerRevision"], model.revision);
        assert_eq!(manifest["file"], model.file);
        assert_eq!(manifest["sha256"], model.sha256);
        assert_eq!(manifest["bytes"], model.bytes);
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Catalog {
    schema_version: u32,
    checked_at: String,
    license: String,
    template_sha256: String,
    models: Vec<ModelArtifact>,
}
impl ModelProfile {
    pub const ALL: [Self; 4] = [
        Self::Qwen9bQ8,
        Self::Qwen9bQ4,
        Self::Qwen4bQ8,
        Self::Qwen4bQ4,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Qwen9bQ8 => "qwen3.5-9b-q8_0",
            Self::Qwen9bQ4 => "qwen3.5-9b-q4_k_m",
            Self::Qwen4bQ8 => "qwen3.5-4b-q8_0",
            Self::Qwen4bQ4 => "qwen3.5-4b-q4_k_m",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|model| model.id() == id)
    }
    pub fn artifact(self) -> &'static ModelArtifact {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        let catalog = CATALOG.get_or_init(|| {
            let catalog: Catalog =
                serde_json::from_str(include_str!("../../../models/comparison-catalog.json"))
                    .expect("compile-time pinned model catalog");
            assert_eq!(catalog.schema_version, 1);
            assert_eq!(catalog.checked_at, "2026-09-28");
            assert_eq!(catalog.license, "Apache-2.0");
            assert_eq!(
                catalog.template_sha256,
                "a4aee8afcf2e0711942cf848899be66016f8d14a889ff9ede07bca099c28f715"
            );
            assert_eq!(catalog.models.len(), Self::ALL.len());
            catalog
        });
        catalog
            .models
            .iter()
            .find(|m| m.id == self.id())
            .expect("registered model")
    }
}
