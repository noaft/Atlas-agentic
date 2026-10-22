use crate::models::ModelLimits;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    #[default]
    Deepseek,
    Openai,
    Anthropic,
    Gemini,
    OpenaiCompatible,
}
impl ProviderId {
    pub const ALL: [Self; 5] = [
        Self::Deepseek,
        Self::Openai,
        Self::Anthropic,
        Self::Gemini,
        Self::OpenaiCompatible,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deepseek => "deepseek",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::OpenaiCompatible => "openai-compatible",
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|id| id.as_str() == value)
            .ok_or_else(|| format!("unknown model provider: {value}"))
    }
    pub fn descriptor(self) -> ProviderDescriptor {
        provider_descriptors()
            .remove(self.as_str())
            .expect("bundled provider descriptor")
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDescriptor {
    pub name: String,
    pub base_url: String,
    pub env_prefix: String,
    pub request_output_tokens: u64,
    pub api_version: Option<String>,
}
pub fn provider_descriptors() -> BTreeMap<String, ProviderDescriptor> {
    toml::from_str(include_str!("../../config/providers.toml"))
        .expect("valid bundled provider descriptors")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSource {
    pub provider: ProviderId,
    pub source_id: String,
}
pub fn normalize_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| "Provider base URL must be an HTTP(S) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Provider base URL must be HTTP(S) without credentials, query or fragment".into(),
        );
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}
pub fn source_for(provider: ProviderId, base_url: &str) -> Result<ModelSource, String> {
    let normalized = normalize_url(base_url)?;
    Ok(ModelSource {
        provider,
        source_id: format!(
            "{:x}",
            Sha256::digest(format!("{}\n{normalized}", provider.as_str()).as_bytes())
        ),
    })
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub temperature: Option<bool>,
    pub top_p: Option<bool>,
    pub json_mode: Option<bool>,
    pub stream_usage: Option<bool>,
    pub image_input: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedModelLimits {
    pub source_id: String,
    pub limits: ModelLimits,
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderSettings {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: Option<String>,
    pub model_limits: BTreeMap<String, ScopedModelLimits>,
    pub capabilities: ProviderCapabilities,
    pub request_output_tokens: Option<u64>,
    pub allow_keyless: bool,
}
impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("api_key_configured", &self.api_key.is_some())
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}
impl ProviderSettings {
    pub fn defaults(provider: ProviderId) -> Self {
        Self {
            base_url: provider.descriptor().base_url,
            ..Self::default()
        }
    }
    pub fn validate(&self, provider: ProviderId) -> Result<(), String> {
        if !self.base_url.is_empty() {
            normalize_url(&self.base_url)?;
        }
        if self.request_output_tokens == Some(0) {
            return Err("Request output tokens must be positive".into());
        }
        if self.allow_keyless && provider != ProviderId::OpenaiCompatible {
            return Err("Only compatible endpoints can be keyless".into());
        }
        if self.model.as_ref().is_some_and(|m| m.trim().is_empty()) {
            return Err("Provider model cannot be blank".into());
        }
        for limits in self.model_limits.values() {
            limits.limits.validate()?;
        }
        Ok(())
    }
}
