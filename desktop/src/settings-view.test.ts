import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { renderSettings } from "./settings-view";
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

test("settings Enter saves null for a blank key and Escape cancels", async () => {
  const { root, saves, store } = fixture();
  await store.initialize();
  store.openSettings();
  renderSettings(root, store.getState(), store);

  root.querySelector<HTMLFormElement>("[data-settings-form]")!.dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(saves[0]?.apiKey).toBeNull();

  store.openSettings();
  renderSettings(root, store.getState(), store);
  root
    .querySelector<HTMLInputElement>("[data-settings-api-key]")!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(store.getState().screen).toBe("chat");
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

  const select = root.querySelector<HTMLSelectElement>(
    "[data-settings-permission-mode]",
  )!;
  expect(select.value).toBe("workspace");
  select.value = "fullAccess";
  select.dispatchEvent(new Event("change", { bubbles: true }));
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
