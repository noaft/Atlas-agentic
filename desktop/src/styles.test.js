import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { afterEach, beforeEach, expect, test } from "vitest";

const styles = readFileSync(resolve(process.cwd(), "src/styles.css"), "utf8");

beforeEach(() => {
  const style = document.createElement("style");
  style.dataset.testStyles = "";
  style.textContent = styles;
  document.head.append(style);
});

afterEach(() => {
  document.querySelector("[data-test-styles]")?.remove();
  document.body.replaceChildren();
});

test("long conversation history scrolls inside the flexible sidebar", () => {
  document.body.innerHTML = `
    <aside class="sidebar">
      <div class="brand">Atlas</div>
      <button class="new-chat">New chat</button>
      <section class="workspace-card">Workspace</section>
      <div class="section-label">Conversations</div>
      <nav class="conversation-list">Conversation list</nav>
      <footer class="sidebar-footer">Settings</footer>
    </aside>`;

  const sidebar = getComputedStyle(document.querySelector(".sidebar"));
  const list = getComputedStyle(document.querySelector(".conversation-list"));

  expect(sidebar.minHeight).toBe("0px");
  expect(sidebar.overflow).toBe("hidden");
  expect(list.flexGrow).toBe("1");
  expect(list.minHeight).toBe("0px");
  expect(list.overflowY).toBe("auto");
});

test("chat messages and settings use the available height as scroll regions", () => {
  document.body.innerHTML = `
    <main class="chat-main"><section class="messages">Messages</section></main>
    <main class="settings-main"><section class="settings-scroll">Settings</section></main>`;

  const chat = getComputedStyle(document.querySelector(".chat-main"));
  const messages = getComputedStyle(document.querySelector(".messages"));
  const settings = getComputedStyle(document.querySelector(".settings-main"));
  const settingsScroll = getComputedStyle(document.querySelector(".settings-scroll"));

  expect(chat.minHeight).toBe("0px");
  expect(chat.overflow).toBe("hidden");
  expect(messages.minHeight).toBe("0px");
  expect(messages.overflowY).toBe("auto");
  expect(settings.minHeight).toBe("0px");
  expect(settings.overflow).toBe("hidden");
  expect(settingsScroll.minHeight).toBe("0px");
  expect(settingsScroll.overflowY).toBe("auto");
});
