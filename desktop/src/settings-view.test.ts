import { emptySkillsApi } from "./skills-api.test-support";
import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { renderSettings, updateSettingsChrome, updateSettingsSection } from "./settings-view";
import type { SaveSettingsInput, SettingsView } from "./types";

const settings: SettingsView = {
  apiKeySource: "environment",
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-flash",
  activeModel: "deepseek-flash",
  historyLimit: 40,
  defaultWorkspace: {
    path: "/work/<img src=x onerror=alert(1)>",
    name: "<script>unsafe</script>",
  },
  defaultWorkspaceAutomatic: false,
  defaultWorkspaceError: null,
  defaultPermissionMode: "ask",
  environmentOverrides: ["apiKey"],
};

function fixture(currentSettings = settings) {
  const saves: SaveSettingsInput[] = [];
  const api = {
    ...emptySkillsApi,
    listMcpConnections: async () => [],
    connectMcpServer: async (_server: string) => {},
    disconnectMcpServer: async (_server: string) => {},
    cancelMcpConnection: async (_server: string) => {},
    cancelCompaction: async () => {},
    listTasks: async () => [],
    runTask: async (taskId) => ({taskId, outcome: {kind: "compaction", result: {memory: null, message: "Done"}}}),
    compactConversation: async () => ({ memory: null, message: "Nothing older to compact" }),
    listConversations: async () => [],
    loadConversation: async () => {
      throw new Error("unused");
    },
    ask: async () => {
      throw new Error("unused");
    },
    renameConversation: async () => {
      throw new Error("unused");
    },
    deleteConversation: async () => {},
    loadSettings: async () => currentSettings,
    saveSettings: async (input: SaveSettingsInput) => {
      saves.push(input);
      return { ...currentSettings, apiKeySource: "settings" as const };
    },
    listModels: async () => [],
    setModel: async (model: string) => ({
      ...currentSettings,
      model,
      activeModel: model,
    }),
    chooseWorkspace: async () => null,
    validateWorkspace: async () => {
      throw new Error("unused");
    },
    assignConversationWorkspace: async () => {
      throw new Error("unused");
    },
    setConversationPermissionMode: async () => {
      throw new Error("unused");
    },
    resolveToolApproval: async () => {},
    cancelToolApprovals: async () => {},
    toggleFullscreen: async () => false,
  } as AtlasApi;
  const store = new ChatStore(api);
  const root = document.createElement("div");
  return { root, saves, store, api };
}

test("settings renders a blank masked key and explains environment authority", async () => {
  const { root, store } = fixture();
  await store.initialize();
  store.openSettings();

  renderSettings(root, store.getState(), store);

  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  expect(key.type).toBe("password");
  expect(key.value).toBe("");
  expect(root.textContent).toContain("environment");
  expect(root.textContent).toContain("<script>unsafe</script>");
  expect(root.querySelector("script")).toBeNull();
  expect(root.querySelector("img")).toBeNull();
});

test("settings directs model selection to the chat composer", async () => {
  const { root, store } = fixture();
  await store.initialize();
  store.openSettings();

  renderSettings(root, store.getState(), store);

  expect(root.textContent).toContain(
    "Choose the active model from the chat composer after saving settings.",
  );
  expect(
    [...root.querySelectorAll<HTMLInputElement>(".settings-field input")].some(
      (input) => input.value === settings.model,
    ),
  ).toBe(false);
});

test("settings Enter saves changes with a blank key and Escape preserves edits", async () => {
  const { root, saves, store } = fixture();
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  store.updateSettingsDraft({ defaultPermissionMode: "workspace" });
  root.querySelector<HTMLFormElement>("[data-settings-form]")!.dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(saves[0]?.apiKey).toBeNull();

  store.openSettings();
  store.updateSettingsDraft({ baseUrl: "https://preserved.test" });
  renderSettings(root, store.getState(), store);
  root
    .querySelector<HTMLInputElement>("[data-settings-api-key]")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(store.getState().screen).toBe("chat");
  store.openSettings();
  expect(store.getState().settingsDraft?.baseUrl).toBe("https://preserved.test");
});

test("automatic workspace is labeled with its effective path", async () => {
  const automatic = {
    ...settings,
    defaultWorkspace: { path: "/home/user/Atlas", name: "Atlas" },
    defaultWorkspaceAutomatic: true,
  };
  const { root, store } = fixture(automatic);
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  expect(root.textContent).toContain("Atlas home (automatic)");
  expect(root.textContent).toContain("/home/user/Atlas");
  expect(root.querySelector("[data-choose-default-workspace]")?.textContent).toBe(
    "Choose custom folder",
  );
  expect(root.querySelector("[data-reset-default-workspace]")).toBeNull();
});

test("custom default can reset to Atlas home", async () => {
  const { root, store } = fixture();
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  expect(root.textContent).toContain("<script>unsafe</script>");
  expect(root.querySelector("[data-reset-default-workspace]")?.textContent).toBe(
    "Reset to Atlas home",
  );
  root.querySelector<HTMLButtonElement>("[data-reset-default-workspace]")!.click();
  renderSettings(root, store.getState(), store);

  expect(root.textContent).toContain("Atlas home (automatic)");
  expect(root.querySelector("[data-choose-default-workspace]")?.textContent).toBe(
    "Choose custom folder",
  );
  expect(root.querySelector("[data-reset-default-workspace]")).toBeNull();
});

test("an_invalid_default_remains_editable_and_shows_its_error", async () => {
  const invalid = {
    ...settings,
    defaultWorkspaceError: "workspace does not exist: /missing/project",
  };
  const { root, store } = fixture(invalid);
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  expect(store.getState().screen).toBe("settings");
  expect(root.textContent).toContain("workspace does not exist");
  expect(root.querySelector("[data-reset-default-workspace]")).not.toBeNull();
});

test("settings hydrates and saves the default permission mode with a full access warning", async () => {
  const { root, saves, store } = fixture({
    ...settings,
    defaultPermissionMode: "workspace",
  });
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  const selected = root.querySelector<HTMLInputElement>(
    "[data-settings-permission-mode]:checked",
  )!;
  expect(selected.value).toBe("workspace");
  const fullAccess = root.querySelector<HTMLInputElement>('[data-settings-permission-mode][value="fullAccess"]')!;
  document.body.append(root);
  fullAccess.click();
  root.remove();
  renderSettings(root, store.getState(), store);
  expect(root.textContent).toContain("run tools without asking");

  root.querySelector<HTMLFormElement>("[data-settings-form]")!.dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(saves[0]?.defaultPermissionMode).toBe("fullAccess");
});

test("editing both model limits preserves the first value before saving", async () => {
 const { root, store } = fixture(); await store.initialize(); store.openSettings(); renderSettings(root,store.getState(),store);
 const fields=root.querySelectorAll<HTMLInputElement>('input[type="number"]');
 fields[0].value="100000";fields[0].dispatchEvent(new Event("input"));
 fields[1].value="4096";fields[1].dispatchEvent(new Event("input"));
 expect(store.getState().settingsDraft?.modelLimits?.[settings.activeModel]).toEqual({contextWindow:100000,maxOutputTokens:4096});
});

const multiSettings = {
  ...settings, provider: "deepseek", source: { provider: "deepseek", sourceId: "deepseek-source" },
  providers: [
    { provider: "deepseek", name: "DeepSeek", envPrefix: "DEEPSEEK", apiKeySource: "settings", baseUrl: "https://api.deepseek.com", model: "deepseek-flash", activeModel: "deepseek-flash", source: {provider:"deepseek",sourceId:"deepseek-source"}, environmentOverrides: [], modelLimits: {}, capabilities: {}, allowKeyless: false, requestOutputTokens: 4096 },
    { provider: "openai", name: "OpenAI", envPrefix: "OPENAI", apiKeySource: "missing", baseUrl: "https://api.openai.com/v1", model: "", activeModel: "", source: {provider:"openai",sourceId:"openai-source"}, environmentOverrides: [], modelLimits: {}, capabilities: {}, allowKeyless: false, requestOutputTokens: 4096 },
  ],
} as SettingsView;

test("provider switching preserves separate unsaved credentials and manual models", async () => {
  const { store, saves } = fixture(multiSettings);
  await store.initialize(); store.openSettings();
  store.updateSettingsDraft({ apiKey: "deepseek-draft" });
  store.selectSettingsProvider("openai");
  expect(store.getState().settingsDraft?.apiKey).toBe("");
  store.updateSettingsDraft({ apiKey: "openai-draft", model: "manual-alias" });
  store.selectSettingsProvider("deepseek");
  expect(store.getState().settingsDraft?.apiKey).toBe("deepseek-draft");
  store.selectSettingsProvider("openai");
  expect(store.getState().settingsDraft?.model).toBe("manual-alias");
  expect(saves).toHaveLength(0);
  store.cancelSettings(); store.openSettings();
  expect(store.getState().settingsDraft?.apiKey).toBe("");
});

test("provider settings exposes provider selection manual model and refresh", async () => {
  const {store,root} = fixture(multiSettings);
  await store.initialize(); store.openSettings();
  renderSettings(root,store.getState(),store);
  const provider = root.querySelector<HTMLSelectElement>("[data-settings-provider]");
  expect(provider).not.toBeNull();
  expect([...provider!.options].map(o=>o.textContent)).toEqual(["DeepSeek","OpenAI"]);
  expect(root.querySelector<HTMLInputElement>("[data-settings-model]")).not.toBeNull();
  expect(root.querySelector("[data-refresh-settings-models]")).not.toBeNull();
});

test("editing discovery credentials resets loading and rejects stale catalog responses", async () => {
  const {store,api,saves} = fixture(multiSettings);
  await store.initialize(); store.openSettings(); store.selectSettingsProvider("openai");
  let resolve!: (models: import("./types").ModelInfo[]) => void;
  const requests: unknown[] = [];
  api.listModels = async input => { requests.push(input); return new Promise(done => { resolve = done; }); };
  store.updateSettingsDraft({apiKey:"transient-key"});
  const pending = store.refreshSettingsModels();
  expect(store.getState().settingsModelsStatus).toBe("loading");
  expect(requests).toEqual([expect.objectContaining({provider:"openai",patch:expect.objectContaining({apiKey:"transient-key"})})]);
  expect(saves).toHaveLength(0);
  store.updateSettingsDraft({apiKey:"new-key"});
  expect(store.getState().settingsModelsStatus).toBe("idle");
  resolve([{id:"stale-model",name:"Stale"}]); await pending;
  expect(store.getState().settingsModels).toEqual([]);
});

test("fresh limits entered for a changed endpoint bind to the draft endpoint", async () => {
  const {store,saves}=fixture(multiSettings);await store.initialize();store.openSettings();store.selectSettingsProvider("openai");
  store.updateSettingsDraft({baseUrl:"https://another.test/v1",model:"alias"});
  store.updateSettingsDraft({modelLimits:{alias:{contextWindow:6000,maxOutputTokens:1000}}});
  await store.saveSettings();
  expect(saves[0].profilePatches?.find(p=>p.provider==="openai")?.limitsSourceId).toBeNull();
});

test("inherited limits survive trailing slash edits but clear on a different endpoint", async () => {
  const current=structuredClone(multiSettings);
  current.providers!.find(p=>p.provider==="openai")!.modelLimits={alias:{contextWindow:6000,maxOutputTokens:1000}};
  const {store}=fixture(current);await store.initialize();store.openSettings();store.selectSettingsProvider("openai");
  store.updateSettingsDraft({baseUrl:"https://api.openai.com/v1/"});
  expect(store.getState().settingsDraft?.modelLimits?.alias.contextWindow).toBe(6000);
  store.updateSettingsDraft({baseUrl:"https://another.test/v1"});
  expect(store.getState().settingsDraft?.modelLimits).toEqual({});
});

test("reopening Settings rejects discovery owned by a discarded draft", async () => {
  const {store,api}=fixture(multiSettings);await store.initialize();store.openSettings();
  let resolve!: (models: import("./types").ModelInfo[]) => void;
  api.listModels=async()=>new Promise(done=>{resolve=done;});
  store.updateSettingsDraft({baseUrl:"https://discarded.test/v1"});
  const pending=store.refreshSettingsModels();await store.newChat();store.openSettings();
  resolve([{id:"discarded-model",name:"Discarded"}]);await pending;
  expect(store.getState().settingsModels).toEqual([]);
});


test("MCP status refresh preserves the mounted model settings draft and focus", async () => {
  const { root, store, api } = fixture({...settings, apiKeySource: "missing", environmentOverrides: []});
  let resolveStatus!: (value: import("./types").McpConnectionStatus[]) => void;
  api.listMcpConnections = () => new Promise(resolve => {resolveStatus = resolve;});
  await store.initialize(); store.openSettings();
  renderSettings(root, store.getState(), store); document.body.append(root);
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  key.value = "private-draft"; key.dispatchEvent(new Event("input")); key.focus(); key.setSelectionRange(2, 5); root.scrollTop = 100;
  resolveStatus([{name:"service",transport:"http",oauth:true,connected:true}]);
  for (let i = 0; i < 10; i++) await Promise.resolve();
  expect(root.textContent).toContain("Connected");
  expect(root.querySelector("[data-settings-api-key]")).toBe(key);
  expect(key.value).toBe("private-draft"); expect(store.getState().settingsDraft?.apiKey).toBe("private-draft");
  expect(document.activeElement).toBe(key); expect(key.selectionStart).toBe(2); expect(root.scrollTop).toBe(100);
  root.remove();
});


test("settings categories show only their controls while retaining draft inputs and MCP panel", async () => {
  const { root, store, api } = fixture(multiSettings);
  let refreshes = 0;
  api.listMcpConnections = async () => { refreshes++; return []; };
  await store.initialize(); store.openSettings();
  renderSettings(root, store.getState(), store);
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  key.value = "draft-secret"; key.dispatchEvent(new Event("input"));
  const mcp = root.querySelector(".mcp-settings");
  expect(root.querySelector<HTMLElement>('[data-settings-section="models"]')?.hidden).toBe(false);
  expect(root.querySelector<HTMLElement>('[data-settings-section="workspace"]')?.hidden).toBe(true);
  expect(root.querySelector('[data-settings-section="models"] [data-choose-default-workspace]')).toBeNull();
  expect(root.querySelector('[data-settings-section="models"] [data-settings-permission-mode]')).toBeNull();
  expect(root.querySelector('[data-settings-section="workspace"] [data-settings-api-key]')).toBeNull();
  updateSettingsSection(root, "connections");
  expect(root.querySelector<HTMLElement>('[data-settings-section="models"]')?.hidden).toBe(true);
  expect(root.querySelector<HTMLElement>('[data-settings-section="connections"]')?.hidden).toBe(false);
  expect(root.querySelector<HTMLElement>(".settings-actions")?.hidden).toBe(true);
  updateSettingsSection(root, "workspace");
  expect(root.querySelector<HTMLElement>('[data-settings-section="workspace"]')?.hidden).toBe(false);
  expect(root.querySelector<HTMLElement>(".settings-actions")?.hidden).toBe(false);
  updateSettingsSection(root, "models");
  expect(root.querySelector("[data-settings-api-key]")).toBe(key);
  expect(key.value).toBe("draft-secret");
  expect(root.querySelector(".mcp-settings")).toBe(mcp);
  expect(refreshes).toBe(1);
});

test("settings footer saves changes in place and discards without leaving Settings", async () => {
  const { root, store, saves } = fixture();
  await store.initialize(); store.openSettings();
  renderSettings(root, store.getState(), store);
  expect(root.querySelector<HTMLButtonElement>('[type="submit"]')?.disabled).toBe(true);
  store.updateSettingsDraft({ defaultPermissionMode: "workspace" });
  renderSettings(root, store.getState(), store);
  root.querySelector<HTMLFormElement>("[data-settings-form]")!.dispatchEvent(new Event("submit", {cancelable: true}));
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(saves).toHaveLength(1);
  expect(store.getState().screen).toBe("settings");
  store.updateSettingsDraft({ baseUrl: "https://draft.test" });
  renderSettings(root, store.getState(), store);
  root.querySelector<HTMLButtonElement>("[data-discard-settings]")!.click();
  expect(store.getState().screen).toBe("settings");
  expect(store.getState().settingsDraft?.baseUrl).toBe(settings.baseUrl);
});


test("settings chrome reflects edits without replacing focused inputs", async () => {
  const {root, store} = fixture({...settings, apiKeySource: "missing", environmentOverrides: []});
  await store.initialize(); store.openSettings();
  renderSettings(root, store.getState(), store);
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  key.value = "draft-key"; key.dispatchEvent(new Event("input"));
  updateSettingsChrome(root, store.getState(), store);
  expect(root.querySelector<HTMLInputElement>("[data-settings-api-key]")).toBe(key);
  expect(root.querySelector<HTMLButtonElement>('[type="submit"]')?.disabled).toBe(false);
  expect(root.querySelector(".settings-draft-status")?.textContent).toBe("Unsaved changes");
  store.discardSettingsChanges();
  updateSettingsChrome(root, store.getState(), store);
  expect(root.querySelector<HTMLButtonElement>('[type="submit"]')?.disabled).toBe(true);
});


test("Back and reopen reject late discovery while preserving the draft", async () => {
  const {store,api}=fixture(multiSettings);await store.initialize();store.openSettings();
  let resolve!: (models: import("./types").ModelInfo[]) => void;
  api.listModels=async()=>new Promise(done=>{resolve=done;});
  store.updateSettingsDraft({apiKey:"keep-private-draft"});
  const pending=store.refreshSettingsModels();store.backToChat();store.openSettings();
  resolve([{id:"late-model",name:"Late"}]);await pending;
  expect(store.getState().settingsModels).toEqual([]);
  expect(store.getState().settingsModelsStatus).toBe("idle");
  expect(store.getState().settingsDraft?.apiKey).toBe("keep-private-draft");
});

test("invalid fields reveal their category before native validation", async () => {
  const {root,store}=fixture(multiSettings);await store.initialize();store.openSettings();
  renderSettings(root,store.getState(),store);
  const invalid=root.querySelector<HTMLInputElement>('input[type="number"]')!;invalid.value="0";
  store.selectSettingsSection("workspace");updateSettingsSection(root,"workspace");
  invalid.dispatchEvent(new Event("invalid",{cancelable:true}));
  expect(store.getState().settingsSection).toBe("models");
  expect(invalid.closest<HTMLDetailsElement>("details")?.open).toBe(true);
});


test("Skills activates lazily, retains import input and hides model Save actions", async () => {
  const { root, store, api } = fixture();
  let loads = 0;
  api.listSkills = async () => { loads++; return {skills: [], issues: []}; };
  await store.initialize(); store.openSettings();
  renderSettings(root, store.getState(), store);
  expect(loads).toBe(0);
  store.selectSettingsSection("skills");
  updateSettingsChrome(root, store.getState(), store);
  await Promise.resolve();
  expect(loads).toBe(1);
  expect(root.querySelector<HTMLElement>(".settings-actions")!.hidden).toBe(true);
  const url = root.querySelector<HTMLInputElement>('[aria-label="GitHub skill URL"]')!;
  url.value = "https://github.com/team/repo"; url.dispatchEvent(new Event("input"));
  store.selectSettingsSection("models"); updateSettingsChrome(root, store.getState(), store);
  renderSettings(root, store.getState(), store);
  expect(root.querySelector('[aria-label="GitHub skill URL"]')).toBe(url);
  expect(url.value).toContain("team/repo");
  store.selectSettingsSection("skills"); updateSettingsChrome(root, store.getState(), store);
  expect(loads).toBe(1);
});

test("lightweight Settings updates change Project ownership without replacing model or Skills drafts", async () => {
  const {root, store, api} = fixture();
  const contexts: Array<{conversationId: string | null; workspacePath: string | null}> = [];
  api.listSkills = async context => { contexts.push(context); return {skills: [], issues: []}; };
  await store.initialize(); store.openSettings(); store.selectSettingsSection("skills");
  renderSettings(root, store.getState(), store);
  await Promise.resolve();
  const key = root.querySelector<HTMLInputElement>("[data-settings-api-key]")!;
  key.value = "draft key"; key.dispatchEvent(new Event("input"));
  const url = root.querySelector<HTMLInputElement>('[aria-label="GitHub skill URL"]')!;
  url.value = "https://github.com/team/repo"; url.dispatchEvent(new Event("input"));
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  scope.value = "project"; scope.dispatchEvent(new Event("change")); await Promise.resolve();
  api.chooseWorkspace = async () => "/private/new-project";
  api.validateWorkspace = async path => ({path, name: "new-project"});
  await store.chooseWorkspaceForDraft();
  updateSettingsChrome(root, store.getState(), store); await Promise.resolve();
  expect(contexts.at(-1)?.workspacePath).toBe("/private/new-project");
  expect(root.querySelector(".skills-settings")?.textContent).toContain("/private/new-project");
  expect(root.querySelector('[aria-label="GitHub skill URL"]')).toBe(url);
  expect(url.value).toBe("https://github.com/team/repo");
  expect(root.querySelector("[data-settings-api-key]")).toBe(key);
  expect(key.value).toBe("draft key");
  expect(store.getState().settingsDraft?.apiKey).toBe("draft key");
});

test("Skills creation textarea survives category navigation and Escape back to chat", async () => {
  const {root, store} = fixture();
  await store.initialize(); store.openSettings(); store.selectSettingsSection("skills");
  renderSettings(root, store.getState(), store);
  const textarea = root.querySelector<HTMLTextAreaElement>('[aria-label="Skill instructions"]')!;
  textarea.value = "Keep this unfinished skill."; textarea.dispatchEvent(new Event("input"));
  store.selectSettingsSection("models"); updateSettingsChrome(root, store.getState(), store);
  store.selectSettingsSection("skills"); updateSettingsChrome(root, store.getState(), store);
  expect(root.querySelector('[aria-label="Skill instructions"]')).toBe(textarea);
  textarea.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true}));
  expect(store.getState().screen).toBe("chat");
  store.openSettings(); renderSettings(root, store.getState(), store);
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill instructions"]')!.value).toBe("Keep this unfinished skill.");
});
