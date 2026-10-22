use super::{AtlasSettings, ProviderId, DEFAULT_DEEPSEEK_MODEL, SETTINGS_VERSION};
/// Bind legacy unscoped limits to their original endpoint before any later edits.
pub(super) fn upgrade(settings: &mut AtlasSettings) {
    if settings.version != 1 {
        return;
    }
    if !settings.providers.contains_key("deepseek") {
        let mut profile = settings.provider_profile(ProviderId::Deepseek);
        if settings.deepseek.model == DEFAULT_DEEPSEEK_MODEL {
            profile.model = None;
        }
        settings.providers.insert("deepseek".into(), profile);
        settings.conversation.model_limits.clear();
    }
    settings.version = SETTINGS_VERSION;
}
