import { expect, test } from "vitest";

import type { AtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { renderSettings } from "./settings-view";
import type { SaveSettingsInput, SettingsView } from "./types";

const settings: SettingsView = {
  apiKeySource: "environment",
  baseUrl: "https://api.deepseek.com",
  model: "deepseek-flash",
  historyLimit: 40,
  defaultWorkspace: {
    path: "/work/<img src=x onerror=alert(1)>",
    name: "<script>unsafe</script>",
  },
  defaultWorkspaceAutomatic: false,
  defaultWorkspaceError: null,
  environmentOverrides: ["apiKey"],
};

function fixture(currentSettings = settings) {
  const saves: SaveSettingsInput[] = [];
  const api = {
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
    chooseWorkspace: async () => null,
    validateWorkspace: async () => {
      throw new Error("unused");
    },
    assignConversationWorkspace: async () => {
      throw new Error("unused");
    },
  } as AtlasApi;
  const store = new ChatStore(api);
  const root = document.createElement("div");
  return { root, saves, store };
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
