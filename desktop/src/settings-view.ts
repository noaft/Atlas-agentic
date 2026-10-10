import type { ChatStore } from "./chat-state";
import type { ChatState } from "./types";

function field(
  labelText: string,
  input: HTMLInputElement,
  hint?: string,
): HTMLLabelElement {
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
  const overridden = new Set(settings.environmentOverrides);

  const key = inputFor(
    draft.apiKey,
    (apiKey) => store.updateSettingsDraft({ apiKey, clearApiKey: false }),
    saving,
  );
  key.type = "password";
  key.autocomplete = "new-password";
  key.placeholder = settings.apiKeySource === "missing" ? "Enter API key" : "Leave blank to keep current key";
  key.dataset.settingsApiKey = "";
  const keySource =
    settings.apiKeySource === "environment"
      ? "The API key is currently supplied by the environment and is never copied into Settings."
      : settings.apiKeySource === "settings"
        ? "A key is saved locally. Leave this blank to keep it."
        : "No API key is configured.";
  form.append(field("DeepSeek API key", key, keySource));

  const clearRow = document.createElement("label");
  clearRow.className = "settings-check";
  const clear = document.createElement("input");
  clear.type = "checkbox";
  clear.checked = draft.clearApiKey;
  clear.disabled = saving || settings.apiKeySource === "environment";
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
      overridden.has("baseUrl") ? "Controlled by DEEPSEEK_BASE_URL." : undefined,
    ),
  );

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

  const history = inputFor(
    String(draft.historyLimit),
    (value) => store.updateSettingsDraft({ historyLimit: Number(value) }),
    saving || overridden.has("historyLimit"),
  );
  history.type = "number";
  history.min = "1";
  history.step = "1";
  form.append(
    field(
      "Conversation history messages",
      history,
      overridden.has("historyLimit")
        ? "Controlled by ATLAS_HISTORY_MESSAGES."
        : "Newest messages included with each request.",
    ),
  );

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
  panel.append(heading, form);
  container.append(panel);
}
