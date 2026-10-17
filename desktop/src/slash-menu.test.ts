import {afterEach, expect, test, vi} from "vitest";
import {createSlashMenu} from "./slash-menu";
import {taskFixtures} from "./task-fixtures.test-support";
import type {ChatState, TaskDescriptor} from "./types";
const settings = {apiKeySource: "settings", activeModel: "fake"} as ChatState["settings"];
const available = {activeConversationId: "saved", settings};
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(fn => fn()); document.body.replaceChildren();});
function setup(tasks = taskFixtures) {
  const host = document.createElement("div"); const textarea = document.createElement("textarea");
  document.body.append(host, textarea); const choose = vi.fn(); const retry = vi.fn();
  const menu = createSlashMenu(textarea, host, choose, retry); cleanups.push(() => menu.destroy());
  let data = {tasks, status: "ready" as "ready" | "loading" | "error", error: null as string | null, state: available, sessionId: "one"};
  const update = (patch: Partial<typeof data> = {}) => {data = {...data, ...patch}; menu.update(data);};
  const type = (text: string, caret = text.length) => {textarea.value = text; textarea.focus(); textarea.setSelectionRange(caret, caret); textarea.dispatchEvent(new Event("input")); update();};
  const key = (key: string, init: KeyboardEventInit = {}) => {const event = new KeyboardEvent("keydown", {key, cancelable: true, ...init}); const consumed = menu.handleKeydown(event); return consumed;};
  update(); return {host, textarea, choose, retry, menu, type, key, update};
}
test("menu filters descriptions and selects the task object without inserting text", () => {
  const s = setup(); s.type("/SUMMARIZE");
  expect(s.host.querySelector('[role="listbox"]')).not.toBeNull();
  expect(s.host.textContent).toContain("/compact"); expect(s.textarea.getAttribute("aria-activedescendant")).toBeTruthy();
  expect(s.key("Enter")).toBe(true); expect(s.choose).toHaveBeenCalledExactlyOnceWith(taskFixtures[0]);
  expect(s.textarea.value).toBe("/SUMMARIZE");
  expect(document.activeElement).toBe(s.textarea); s.update();
  expect(s.host.querySelector('[role="listbox"]')).toBeNull(); expect(s.key("Enter")).toBe(false);
});
test("Enter and pointer choose a task directly; Tab leaves text unchanged", () => {
  const second: TaskDescriptor = {...taskFixtures[0], id: "other", command: "other", title: "Other"};
  const s = setup([...taskFixtures, second]); s.type("/");
  s.key("End"); expect(s.key("Enter")).toBe(true); expect(s.choose).toHaveBeenLastCalledWith(second);
  s.type("/c"); s.host.querySelector<HTMLElement>('[role="option"]')!.click(); expect(s.choose).toHaveBeenLastCalledWith(taskFixtures[0]);
  s.type("/co"); s.choose.mockClear(); expect(s.key("Tab")).toBe(false);
  expect(s.choose).not.toHaveBeenCalled(); expect(s.textarea.value).toBe("/co");
  expect(s.host.querySelector('[role="listbox"]')).toBeNull();
});
test("Escape and outside dismissal preserve draft and suppress unchanged reopening", () => {
  const s = setup(); s.type("/c"); expect(s.key("Escape")).toBe(true); s.update();
  expect(s.textarea.value).toBe("/c"); expect(s.host.querySelector('[role="listbox"]')).toBeNull();
  s.type("/co"); document.body.dispatchEvent(new PointerEvent("pointerdown", {bubbles: true})); s.update();
  expect(s.host.querySelector('[role="listbox"]')).toBeNull(); expect(s.textarea.value).toBe("/co");
});
test("IME, shift enter, paths and cursor outside first token preserve text entry", () => {
  const s = setup(); s.type("/c"); expect(s.key("Enter", {isComposing: true})).toBe(false);
  expect(s.key("Enter", {shiftKey: true})).toBe(false); expect(s.choose).not.toHaveBeenCalled();
  for (const text of ["/home/user", "say /compact", "`/compact`", "/compact argument"]) {s.type(text); expect(s.host.querySelector('[role="listbox"]')).toBeNull();}
});
test("requirements stay discoverable and are rechecked on model or session change", () => {
  const s = setup(); s.type("/"); s.update({state: {...available, activeConversationId: null as unknown as string}});
  expect(s.host.textContent).toContain("Start a saved conversation first"); s.key("Enter"); expect(s.choose).not.toHaveBeenCalled();
  s.update({state: {...available, settings: null}}); expect(s.host.textContent).toContain("Configure a model first");
  s.update({state: available}); s.key("Escape"); s.update({sessionId: "two"});
  expect(s.host.querySelector('[role="listbox"]')).not.toBeNull();
});
test("loading, errors, retry, and no results keep input intact", () => {
  const s = setup(); s.type("/"); s.update({status: "loading"}); expect(s.host.textContent).toMatch(/Loading/);
  s.update({status: "error", error: "offline"}); expect(s.host.textContent).toContain("offline");
  s.host.querySelector('button')!.click(); expect(s.retry).toHaveBeenCalledOnce(); expect(s.textarea.value).toBe("/");
  s.update({status: "ready"}); s.type("/nothing"); expect(s.host.textContent).toMatch(/No matching/);
});
test("popup fits a narrow viewport and removes global listeners and aria on destroy", () => {
  const s = setup(); s.type("/");
  const popup = s.host.querySelector<HTMLElement>('[role="listbox"]')!;
  expect(popup.style.maxHeight).toBeTruthy(); expect(popup.style.width).toBeTruthy();
  s.menu.destroy(); expect(s.textarea.hasAttribute("aria-activedescendant")).toBe(false);
  s.textarea.dispatchEvent(new Event("input")); expect(s.host.children.length).toBe(0);
});
test("moving focus away closes the popup without clearing input", () => {
  const s = setup(); s.type("/");
  const button = document.createElement("button"); document.body.append(button); button.focus();
  expect(s.host.querySelector('[role="listbox"]')).toBeNull(); expect(s.textarea.value).toBe("/");
});
test("any edited draft releases dismissal even after ordinary text", () => {
 const s = setup(); s.type("/c"); s.key("Escape"); s.type("ordinary text"); s.type("/c");
 expect(s.host.querySelector('[role="listbox"]')).not.toBeNull();
});
test("catalog retry button remains connected when it receives keyboard focus", () => {
  const s = setup(); s.type("/"); s.update({status: "error", error: "offline"});
  const retry = s.host.querySelector<HTMLButtonElement>("button")!;
  retry.focus(); expect(retry.isConnected).toBe(true); expect(document.activeElement).toBe(retry);
  retry.click(); expect(s.retry).toHaveBeenCalledOnce(); expect(document.activeElement).toBe(s.textarea);
  expect(s.textarea.value).toBe("/");
});
test("Enter retries a failed catalog without submitting the command", () => {
  const s = setup(); s.type("/"); s.update({status: "error", error: "offline"});
  expect(s.key("Enter")).toBe(true); expect(s.retry).toHaveBeenCalledOnce();
  expect(document.activeElement).toBe(s.textarea); expect(s.textarea.value).toBe("/");
});
test.each([
  {status: "streaming" as const}, {status: "loading" as const}, {historyLoaded: false},
  {workspaceBusy: true}, {permissionBusy: true}, {modelListStatus: "saving" as const},
])("busy chat %j explains why tasks cannot run", (busy) => {
  const s = setup(); s.type("/"); s.update({state: {...available, ...busy}});
  expect(s.host.querySelector('[role="option"]')?.getAttribute("aria-disabled")).toBe("true");
  expect(s.host.textContent).toMatch(/busy/i);
  s.key("Enter"); expect(s.choose).not.toHaveBeenCalled();
  expect(s.host.querySelector('[role="listbox"]')).not.toBeNull(); expect(s.textarea.value).toBe("/");
});
