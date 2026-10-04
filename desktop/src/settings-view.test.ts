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
  environmentOverrides: ["apiKey"],
};

function fixture() {
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
    loadSettings: async () => settings,
    saveSettings: async (input: SaveSettingsInput) => {
      saves.push(input);
      return { ...settings, apiKeySource: "settings" as const };
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
