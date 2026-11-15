import type {ChatState, ModelInfo, ModelLimits} from "./types";

function normalizedEndpoint(value: string): string {
  try { return new URL(value.trim()).toString().replace(/\/+$/, ""); }
  catch { return value.trim().replace(/\/+$/, ""); }
}

const capacity = (value: number | null | undefined): number | undefined =>
  value != null && Number.isSafeInteger(value) && value > 0 ? value : undefined;

/** Display discovery values without turning them into saved capacity overrides. */
export function settingsModelLimits(state: Readonly<ChatState>): {
  model: string; reported: Partial<ModelLimits>; override?: ModelLimits;
} {
  const {settings, settingsDraft: draft} = state;
  if (!settings || !draft) return {model: "", reported: {}};
  const provider = draft.selectedProvider ?? settings.provider;
  const profile = settings.providers?.find(item => item.provider === provider);
  const overridden = profile?.environmentOverrides ?? settings.environmentOverrides;
  const model = overridden.includes("model")
    ? profile?.activeModel ?? settings.activeModel
    : draft.model || (profile ? profile.activeModel : settings.activeModel);
  const savedSource = profile?.source ?? (provider === settings.provider ? settings.source : null);
  const sameEndpoint = normalizedEndpoint(draft.baseUrl) === normalizedEndpoint(profile?.baseUrl ?? settings.baseUrl);
  const matchingModel = (item: ModelInfo) => item.id === model && !!item.source?.sourceId &&
    item.source.provider === provider;
  // Settings discovery is bound to its draft by the store's generation checks.
  // Chat discovery is usable only for the unchanged active provider and endpoint.
  const discovered = state.settingsModels?.find(item => matchingModel(item) &&
    (!sameEndpoint || !savedSource || item.source!.sourceId === savedSource.sourceId));
  const active = sameEndpoint && provider === settings.provider && savedSource
    ? state.availableModels.find(item => matchingModel(item) && item.source!.sourceId === savedSource.sourceId)
    : undefined;
  const metadata = discovered ?? active;
  return {
    model,
    reported: {
      contextWindow: capacity(metadata?.contextWindow),
      maxOutputTokens: capacity(metadata?.maxOutputTokens),
    },
    override: draft.modelLimits?.[model],
  };
}
