use atlas_agentic::{
    models::{ModelInfo, ModelLimits},
    settings::{
        AtlasSettings, EffectiveSettings, ModelSource, ProviderCapabilities, ProviderId,
        ScopedModelLimits, SettingField,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsView {
    pub provider: ProviderId,
    pub name: String,
    pub env_prefix: String,
    pub source: Option<ModelSource>,
    pub api_key_source: &'static str,
    pub base_url: String,
    pub model: String,
    pub active_model: String,
    pub environment_overrides: Vec<&'static str>,
    pub model_limits: BTreeMap<String, ModelLimits>,
    pub capabilities: ProviderCapabilities,
    pub allow_keyless: bool,
    pub request_output_tokens: u64,
}
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettingsPatch {
    pub provider: ProviderId,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    pub base_url: String,
    pub model: Option<String>,
    pub model_limits: Option<BTreeMap<String, ModelLimits>>,
    pub limits_source_id: Option<String>,
    pub capabilities: Option<ProviderCapabilities>,
    pub allow_keyless: Option<bool>,
    pub request_output_tokens: Option<u64>,
}
impl std::fmt::Debug for ProviderSettingsPatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSettingsPatch")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscoveryInput {
    pub provider: ProviderId,
    pub patch: Option<ProviderSettingsPatch>,
}

pub fn field_name(field: &SettingField) -> &'static str {
    match field {
        SettingField::Provider => "provider",
        SettingField::ApiKey => "apiKey",
        SettingField::BaseUrl => "baseUrl",
        SettingField::Model => "model",
        SettingField::HistoryLimit => "historyLimit",
    }
}
pub fn for_provider(
    saved: &AtlasSettings,
    provider: ProviderId,
) -> Result<EffectiveSettings, String> {
    let mut selected = saved.clone();
    selected.selected_provider = Some(provider);
    selected.resolve_with(|name| {
        if name == "ATLAS_PROVIDER" {
            None
        } else {
            std::env::var_os(name)
        }
    })
}
pub fn views(saved: &AtlasSettings) -> Vec<ProviderSettingsView> {
    ProviderId::ALL
        .into_iter()
        .map(|provider| {
            let profile = saved.provider_profile(provider);
            // Inactive invalid environment values must not prevent editing another provider.
            let effective = for_provider(saved, provider).unwrap_or_else(|_| {
                let mut selected = saved.clone();
                selected.selected_provider = Some(provider);
                selected
                    .resolve_with(|_| None)
                    .expect("validated saved profile")
            });
            let descriptor = provider.descriptor();
            ProviderSettingsView {
                provider,
                name: descriptor.name,
                env_prefix: descriptor.env_prefix,
                source: effective.source.clone(),
                api_key_source: if effective
                    .environment_overrides
                    .contains(&SettingField::ApiKey)
                {
                    "environment"
                } else if profile.api_key.is_some() {
                    "settings"
                } else {
                    "missing"
                },
                base_url: profile.base_url,
                model: profile.model.unwrap_or_default(),
                active_model: effective.model,
                environment_overrides: effective
                    .environment_overrides
                    .iter()
                    .map(field_name)
                    .collect(),
                model_limits: effective.model_limits,
                capabilities: profile.capabilities,
                allow_keyless: profile.allow_keyless,
                request_output_tokens: effective.request_output_tokens,
            }
        })
        .collect()
}
pub fn apply_profile(
    saved: &mut AtlasSettings,
    patch: &ProviderSettingsPatch,
) -> Result<(), String> {
    let provider = patch.provider;
    let effective = for_provider(saved, provider)?;
    let mut profile = saved.provider_profile(provider);
    let prefix = provider.descriptor().env_prefix;
    for (field, changed) in [
        (
            SettingField::ApiKey,
            patch.clear_api_key || patch.api_key.is_some(),
        ),
        (SettingField::BaseUrl, profile.base_url != patch.base_url),
        (
            SettingField::Model,
            profile.model.as_deref().unwrap_or("") != patch.model.as_deref().unwrap_or(""),
        ),
    ] {
        if changed && effective.environment_overrides.contains(&field) {
            return Err(format!(
                "{} is controlled by {}_{}",
                field_name(&field),
                prefix,
                match field {
                    SettingField::ApiKey => "API_KEY",
                    SettingField::BaseUrl => "BASE_URL",
                    _ => "MODEL",
                }
            ));
        }
    }
    if patch.clear_api_key {
        profile.api_key = None;
    } else if let Some(key) = &patch.api_key {
        if key.trim().is_empty() {
            return Err("Provider API key cannot be blank".into());
        }
        profile.api_key = Some(key.clone());
    }
    profile.base_url = patch.base_url.trim().to_string();
    profile.model = patch
        .model
        .as_ref()
        .map(|m| m.trim())
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    if let Some(caps) = &patch.capabilities {
        profile.capabilities = caps.clone();
    }
    if let Some(allow) = patch.allow_keyless {
        profile.allow_keyless = allow;
    }
    if let Some(bound) = patch.request_output_tokens {
        profile.request_output_tokens = Some(bound);
    }
    if let Some(limits) = &patch.model_limits {
        let source = patch
            .limits_source_id
            .clone()
            .or_else(|| {
                atlas_agentic::settings::providers::source_for(provider, &profile.base_url)
                    .ok()
                    .map(|s| s.source_id)
            })
            .unwrap_or_default();
        profile.model_limits = limits
            .iter()
            .map(|(m, l)| {
                (
                    m.clone(),
                    ScopedModelLimits {
                        source_id: source.clone(),
                        limits: *l,
                    },
                )
            })
            .collect();
    }
    profile.validate(provider)?;
    if provider == ProviderId::Deepseek {
        saved.deepseek.api_key = profile.api_key.clone();
        saved.deepseek.base_url = profile.base_url.clone();
        if let Some(model) = &profile.model {
            saved.deepseek.model = model.clone();
        }
        // The legacy map is read only when no scoped profile exists.
        saved.conversation.model_limits.clear();
    }
    saved.providers.insert(provider.as_str().into(), profile);
    Ok(())
}
pub fn discover(
    saved: &AtlasSettings,
    input: &ModelDiscoveryInput,
) -> Result<Vec<ModelInfo>, String> {
    let mut transient = saved.clone();
    if let Some(patch) = &input.patch {
        if patch.provider != input.provider {
            return Err("Discovery profile does not match provider".into());
        }
        apply_profile(&mut transient, patch)?;
    }
    let effective = for_provider(&transient, input.provider)?;
    atlas_agentic::models::discovery::discover_models(&effective)
}
