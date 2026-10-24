import type { ChatStore } from "./chat-state";
import type { ChatState } from "./types";
import { createMcpSettingsPanel } from "./mcp-settings";

const mcpPanels = new WeakMap<ChatStore, HTMLElement>();

function field(
  labelText: string,
  input: HTMLInputElement,
  hint?: string,
): HTMLLabelElement {
  input.dataset.settingsField = labelText;
  const label = document.createElement("label");
  label.className = "settings-field";
  const title = document.createElement("span");
  title.className = "settings-label";
  title.textContent = labelText;
  label.append(title, input);
  if (hint) {
    const help = document.createElement("span");
    help.className = "settings-hint";
    help.textContent = hint;
    label.append(help);
  }
  return label;
}

function inputFor(
  value: string,
  onInput: (value: string) => void,
  disabled = false,
): HTMLInputElement {
  const input = document.createElement("input");
  input.value = value;
  input.disabled = disabled;
  input.addEventListener("input", () => onInput(input.value));
  return input;
}

export function renderSettings(
  container: HTMLElement,
  state: Readonly<ChatState>,
  store: ChatStore,
): void {
  container.replaceChildren();
  const draft = state.settingsDraft;
  const settings = state.settings;
  if (!draft || !settings) return;

  const panel = document.createElement("section");
  panel.className = "settings-panel";
  panel.setAttribute("aria-labelledby", "settings-title");
  const heading = document.createElement("div");
  heading.className = "settings-heading";
  const title = document.createElement("h1");
  title.id = "settings-title";
  title.textContent = "Settings";
  const subtitle = document.createElement("p");
  subtitle.textContent = "Model access and the starting folder Atlas uses for new chats.";
  heading.append(title, subtitle);

  const form = document.createElement("form");
  form.className = "settings-form";
  form.dataset.settingsForm = "";
  const saving = state.settingsStatus === "saving";
  const profile = settings.providers?.find(p => p.provider === draft.selectedProvider);
  const overridden = new Set(profile?.environmentOverrides ?? settings.environmentOverrides);
  const credentialSource = profile?.apiKeySource ?? settings.apiKeySource;
  const envPrefix = profile?.envPrefix ?? "DEEPSEEK";
  const limitModel = draft.model || profile?.activeModel || settings.activeModel;
  if (settings.providers) {
    const label = document.createElement("label"); label.className = "settings-field";
    const title = document.createElement("span"); title.className = "settings-label"; title.textContent = "Provider";
    const select = document.createElement("select"); select.dataset.settingsProvider = "";
    select.setAttribute("aria-label", "Provider");
    select.disabled = saving || settings.environmentOverrides.includes("provider");
    for (const item of settings.providers) { const option = document.createElement("option"); option.value = item.provider; option.textContent = item.name; select.append(option); }
    select.value = draft.selectedProvider ?? settings.provider ?? "";
    select.addEventListener("change", () => store.selectSettingsProvider(select.value as import("./types").ProviderId));
    label.append(title, select);
    if (settings.environmentOverrides.includes("provider")) { const hint = document.createElement("span"); hint.className = "settings-hint"; hint.textContent = "Controlled by ATLAS_PROVIDER."; label.append(hint); }
    form.append(label);
  }

  const key = inputFor(
    draft.apiKey,
    (apiKey) => store.updateSettingsDraft({ apiKey, clearApiKey: false }),
    saving || overridden.has("apiKey"),
  );
  key.type = "password";
  key.autocomplete = "new-password";
  key.placeholder = credentialSource === "missing" ? "Enter API key" : "Leave blank to keep current key";
  key.dataset.settingsApiKey = "";
  const keySource =
    credentialSource === "environment"
      ? "The API key is currently supplied by the environment and is never copied into Settings."
      : credentialSource === "settings"
        ? "A key is saved locally. Leave this blank to keep it."
        : "No API key is configured.";
  form.append(field(`${profile?.name ?? "DeepSeek"} API key`, key, keySource));

  const clearRow = document.createElement("label");
  clearRow.className = "settings-check";
  const clear = document.createElement("input");
  clear.type = "checkbox";
  clear.checked = draft.clearApiKey;
  clear.disabled = saving || credentialSource === "environment";
  clear.addEventListener("change", () =>
    store.updateSettingsDraft({ clearApiKey: clear.checked, apiKey: "" }),
  );
  const clearText = document.createElement("span");
  clearText.textContent = "Remove the locally saved API key";
  clearRow.append(clear, clearText);
  form.append(clearRow);

  const baseUrl = inputFor(
    draft.baseUrl,
    (value) => store.updateSettingsDraft({ baseUrl: value }),
    saving || overridden.has("baseUrl"),
  );
  form.append(
    field(
      "Base URL",
      baseUrl,
      overridden.has("baseUrl") ? `Controlled by ${envPrefix}_BASE_URL.` : undefined,
    ),
  );

  if (profile) {
    const model = inputFor(draft.model, value => store.updateSettingsDraft({model: value}), saving || overridden.has("model"));
    model.dataset.settingsModel = "";
    model.placeholder = "Enter model ID or choose below";
    form.append(field("Model", model, overridden.has("model") ? `Controlled by ${envPrefix}_MODEL.` : "Enter an exact model ID, or refresh the available models."));
    const refresh = document.createElement("button"); refresh.type = "button"; refresh.className = "secondary-button"; refresh.dataset.refreshSettingsModels = "";
    refresh.textContent = state.settingsModelsStatus === "loading" ? "Loading models…" : "Refresh models";
    refresh.disabled = saving || state.settingsModelsStatus === "loading";
    refresh.addEventListener("click", () => void store.refreshSettingsModels()); form.append(refresh);
    if (state.settingsModels?.length) {
      const select = document.createElement("select"); select.setAttribute("aria-label","Available models"); select.dataset.settingsModelList = "";
      const placeholder = document.createElement("option"); placeholder.value = ""; placeholder.textContent = "Choose a model"; select.append(placeholder);
      for (const item of state.settingsModels) { const option = document.createElement("option"); option.value = item.id; option.textContent = item.name; select.append(option); }
      select.value = draft.model; select.disabled = saving || overridden.has("model");
      select.addEventListener("change", () => { if (select.value) store.updateSettingsDraft({model: select.value}); }); form.append(select);
    }
    if (state.settingsModelsError) { const error = document.createElement("p"); error.className = "settings-error"; error.setAttribute("role","alert"); error.textContent = state.settingsModelsError; form.append(error); }
    const advanced = document.createElement("details"); advanced.className = "settings-advanced";
    const summary = document.createElement("summary"); summary.textContent = "Advanced model options"; advanced.append(summary);
    for (const [name,labelText] of [["temperature","Temperature parameter"],["topP","Top-p parameter"],["jsonMode","JSON response mode"],["streamUsage","Request streamed usage"],["imageInput","Image input"]] as const) {
      if ((name === "streamUsage" && profile.provider !== "openai-compatible") || (name === "jsonMode" && profile.provider === "anthropic")) continue;
      const label = document.createElement("label"); label.className = "settings-field";
      const title = document.createElement("span"); title.textContent = labelText;
      const select = document.createElement("select"); select.setAttribute("aria-label",labelText); select.disabled = saving;
      for (const [value,text] of [["","Provider default"],["true","Enabled"],["false","Disabled"]]) { const option = document.createElement("option"); option.value = value; option.textContent = text; select.append(option); }
      const value = draft.capabilities?.[name]; select.value = value == null ? "" : String(value);
      select.addEventListener("change", () => store.updateSettingsDraft({capabilities: {...store.getState().settingsDraft?.capabilities,[name]:select.value === "" ? null : select.value === "true"}}));
      label.append(title,select); advanced.append(label);
    }
    const output = inputFor(String(draft.requestOutputTokens ?? profile.requestOutputTokens),value => store.updateSettingsDraft({requestOutputTokens: Number(value)}),saving);
    output.type = "number"; output.min = "1"; output.step = "1";
    advanced.append(field("Default output budget (tokens)",output,"Used when the request has no explicit output budget. This does not set model capacity."));
    if (profile.provider === "openai-compatible") {
      const label = document.createElement("label"); label.className = "settings-check";
      const toggle = document.createElement("input"); toggle.type = "checkbox"; toggle.checked = draft.allowKeyless ?? false; toggle.disabled = saving;
      toggle.addEventListener("change", () => store.updateSettingsDraft({allowKeyless: toggle.checked}));
      label.append(toggle, document.createTextNode("This endpoint does not require an API key")); advanced.append(label);
    }
    form.append(advanced);
  } else {
  const modelSummary = document.createElement("div");
  modelSummary.className = "settings-model-summary";
  const modelLabel = document.createElement("span");
  modelLabel.className = "settings-label";
  modelLabel.textContent = "Active model";
  const modelValue = document.createElement("strong");
  modelValue.textContent = settings.activeModel;
  const modelHint = document.createElement("span");
  modelHint.className = "settings-hint";
  modelHint.textContent = overridden.has("model")
    ? "Controlled by DEEPSEEK_MODEL. Remove that environment setting to choose a model in chat."
    : "Choose the active model from the chat composer after saving settings.";
  modelSummary.append(modelLabel, modelValue, modelHint);
  form.append(modelSummary);

  }

  const permission = document.createElement("select");
  permission.dataset.settingsPermissionMode = "";
  permission.disabled = saving;
  permission.setAttribute("aria-label", "Default tool access");
  for (const [value, label] of [
    ["ask", "Ask before changes"],
    ["workspace", "Allow workspace changes"],
    ["fullAccess", "Full access"],
  ] as const) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    permission.append(option);
  }
  permission.value = draft.defaultPermissionMode;
  permission.addEventListener("change", () =>
    store.updateSettingsDraft({
      defaultPermissionMode: permission.value as import("./types").PermissionMode,
    }),
  );
  const permissionField = document.createElement("label");
  permissionField.className = "settings-field";
  const permissionTitle = document.createElement("span");
  permissionTitle.className = "settings-label";
  permissionTitle.textContent = "Default tool access for new chats";
  const permissionHint = document.createElement("span");
  permissionHint.className = "settings-hint";
  permissionHint.textContent =
    draft.defaultPermissionMode === "fullAccess"
      ? "Warning: Atlas can run tools without asking. Your OS account permissions still apply."
      : "Each saved chat keeps its own selection.";
  permissionField.append(permissionTitle, permission, permissionHint);
  form.append(permissionField);

  if (overridden.has("historyLimit")) {
    const history = inputFor(String(draft.historyLimit), () => {}, true);
    form.append(field("Conversation history messages", history, "Legacy history mode: controlled by ATLAS_HISTORY_MESSAGES. Automatic memory is disabled."));
  } else {
    const memoryHint = document.createElement("p");
    memoryHint.className = "settings-hint";
    memoryHint.textContent = "Conversation memory sends all history until compaction. Auto-compact at 90%, preserve the latest 10 messages. Type /compact to compact manually.";
    form.append(memoryHint);
  }
  const limits = draft.modelLimits?.[limitModel];
  const setLimit = (key: "contextWindow" | "maxOutputTokens", value: string) => {
    const next = { ...(store.getState().settingsDraft?.modelLimits ?? {}) };
    const current = next[limitModel] ?? { contextWindow: 0, maxOutputTokens: 0 };
    next[limitModel] = { ...current, [key]: value.trim() ? Number(value) : 0 };
    if (!next[limitModel].contextWindow && !next[limitModel].maxOutputTokens) delete next[limitModel];
    store.updateSettingsDraft({ modelLimits: next });
  };
  for (const [key, label] of [["contextWindow", "Model context limit (tokens)"], ["maxOutputTokens", "Model output limit (tokens)"]] as const) {
    const input = inputFor(limits?.[key] ? String(limits[key]) : "", (value) => setLimit(key, value), saving || !limitModel);
    input.type = "number"; input.min = "1"; input.step = "1";
    form.append(field(label, input, "Optional combined context and output overrides for this model and endpoint. Set both, or leave both blank."));
  }

  const workspaceBlock = document.createElement("div");
  workspaceBlock.className = "settings-workspace";
  const workspaceLabel = document.createElement("span");
  workspaceLabel.className = "settings-label";
  workspaceLabel.textContent = "Default workspace";
  const workspaceValue = document.createElement("div");
  workspaceValue.className = "settings-workspace-value";
  const workspaceName = document.createElement("strong");
  const configuredWorkspace = draft.defaultWorkspace;
  const usingAutomaticWorkspace = configuredWorkspace === null;
  workspaceName.textContent = usingAutomaticWorkspace
    ? "Atlas home (automatic)"
    : configuredWorkspace.name;
  const workspacePath = document.createElement("span");
  workspacePath.textContent = usingAutomaticWorkspace
    ? settings.defaultWorkspaceAutomatic && settings.defaultWorkspace
      ? settings.defaultWorkspace.path
      : "Save settings to use the automatic Atlas home."
    : configuredWorkspace.path;
  workspaceValue.append(workspaceName, workspacePath);
  if (settings.defaultWorkspaceError) {
    const workspaceError = document.createElement("span");
    workspaceError.className = "settings-hint settings-error";
    workspaceError.setAttribute("role", "alert");
    workspaceError.textContent = settings.defaultWorkspaceError;
    workspaceValue.append(workspaceError);
  }
  const workspaceActions = document.createElement("div");
  workspaceActions.className = "settings-workspace-actions";
  const choose = document.createElement("button");
  choose.type = "button";
  choose.className = "secondary-button";
  choose.textContent = "Choose custom folder";
  choose.dataset.chooseDefaultWorkspace = "";
  choose.disabled = saving;
  choose.addEventListener("click", () => void store.chooseDefaultWorkspace());
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "text-button";
  remove.textContent = "Reset to Atlas home";
  remove.dataset.resetDefaultWorkspace = "";
  remove.disabled = saving;
  remove.addEventListener("click", () =>
    store.updateSettingsDraft({ defaultWorkspace: null }),
  );
  workspaceActions.append(choose);
  if (!usingAutomaticWorkspace) workspaceActions.append(remove);
  workspaceBlock.append(workspaceLabel, workspaceValue, workspaceActions);
  form.append(workspaceBlock);

  if (state.settingsErrorMessage) {
    const error = document.createElement("div");
    error.className = "error-notice settings-error";
    error.setAttribute("role", "alert");
    error.textContent = state.settingsErrorMessage;
    form.append(error);
  }

  const actions = document.createElement("div");
  actions.className = "settings-actions";
  const cancel = document.createElement("button");
  cancel.type = "button";
  cancel.className = "secondary-button";
  cancel.textContent = "Cancel";
  cancel.disabled = saving;
  cancel.addEventListener("click", () => store.cancelSettings());
  const save = document.createElement("button");
  save.type = "submit";
  save.className = "primary-button";
  save.textContent = saving ? "Saving…" : "Save settings";
  save.disabled = saving;
  actions.append(cancel, save);
  form.append(actions);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void store.saveSettings();
  });

  panel.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      store.cancelSettings();
    }
  });
  let mcpPanel = mcpPanels.get(store);
  if (!mcpPanel) {
    mcpPanel = createMcpSettingsPanel(store);
    mcpPanels.set(store, mcpPanel);
  }
  panel.append(heading, form, mcpPanel);
  container.append(panel);
}
