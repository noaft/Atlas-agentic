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

test("chat content expands with the available window width", () => {
  document.body.innerHTML = `
    <section class="messages">
      <article class="message message-assistant">Assistant</article>
      <article class="message message-user">User</article>
    </section>
    <form class="composer-wrap">
      <div class="composer-box">Composer</div>
      <div class="model-picker-feedback">Feedback</div>
      <p class="composer-note">Note</p>
    </form>`;

  expect(getComputedStyle(document.querySelector(".message-assistant")).maxWidth).toBe(
    "none",
  );
  const userMessage = getComputedStyle(document.querySelector(".message-user"));
  expect(userMessage.width).toBe("fit-content");
  expect(userMessage.maxWidth).toBe("76%");
  expect(getComputedStyle(document.querySelector(".composer-box")).maxWidth).toBe("none");
  expect(getComputedStyle(document.querySelector(".model-picker-feedback")).maxWidth).toBe(
    "none",
  );
  expect(getComputedStyle(document.querySelector(".composer-note")).maxWidth).toBe("none");
});

test("model picker is visually borderless while keeping an accessible label", () => {
  document.body.innerHTML = `
    <div class="model-picker"><label><select aria-label="Model"><option>DeepSeek</option></select></label></div>`;

  const select = document.querySelector("select");
  const style = getComputedStyle(select);
  expect(select.getAttribute("aria-label")).toBe("Model");
  expect(style.borderStyle).toBe("none");
  expect(style.backgroundColor).toBe("rgba(0, 0, 0, 0)");
  expect(style.appearance).toBe("none");
});

test("sidebar controls remain visible on desktop", () => {
  document.body.innerHTML = `
    <button class="sidebar-toggle">Toggle</button>
    <header class="settings-mobile-header">Settings</header>`;

  expect(getComputedStyle(document.querySelector(".sidebar-toggle")).display).toBe("grid");
  expect(getComputedStyle(document.querySelector(".settings-mobile-header")).display).toBe(
    "flex",
  );
});

test("activity details stay inside narrow message layouts", () => {
  document.body.innerHTML = `
    <article class="message message-assistant">
      <details class="activity-disclosure">
        <div class="activity-row"><pre>very long bounded tool output</pre></div>
      </details>
    </article>`;

  const disclosure = getComputedStyle(document.querySelector(".activity-disclosure"));
  const output = getComputedStyle(document.querySelector(".activity-row pre"));
  expect(disclosure.maxWidth).toBe("100%");
  expect(output.maxWidth).toBe("100%");
  expect(output.whiteSpace).toBe("pre-wrap");
  expect(output.overflowWrap).toBe("anywhere");
});
