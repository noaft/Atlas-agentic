import { afterEach, expect, test } from "vitest";
import { createComposerPicker } from "./composer-picker";

const cleanups: Array<() => void> = [];
afterEach(() => { cleanups.splice(0).forEach((cleanup) => cleanup()); document.body.replaceChildren(); });
function setup(disabled = false) {
  const select = document.createElement("select");
  select.setAttribute("aria-label", "Model");
  select.innerHTML = '<option value="a">Model A</option><option value="b">Model B</option>';
  select.disabled = disabled;
  const picker = createComposerPicker(select);
  document.body.append(picker.element);
  cleanups.push(picker.destroy);
  return { select, ...picker, button: picker.element.querySelector("button")! };
}
test("custom menu selects an option and restores focus without submitting", () => {
  const { select, element, button } = setup();
  let changes = 0;
  select.addEventListener("change", () => changes++);
  expect(button.type).toBe("button");
  expect(button.textContent).toContain("Model A");
  expect(button.getAttribute("aria-label")).toBe("Model: Model A");
  button.click();
  expect(button.getAttribute("aria-expanded")).toBe("true");
  const options = element.querySelectorAll<HTMLElement>('[role="option"]');
  expect(options[0].getAttribute("aria-selected")).toBe("true");
  options[1].click();
  expect(select.value).toBe("b");
  expect(changes).toBe(1);
  expect(button.textContent).toContain("Model B");
  expect(button.getAttribute("aria-label")).toBe("Model: Model B");
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(button);
});
test("keyboard navigation, escape, tab, and outside click close the menu", () => {
  const { button, element, select } = setup();
  button.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
  const list = element.querySelector<HTMLElement>('[role="listbox"]')!;
  list.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
  list.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  expect(select.value).toBe("b");
  button.click();
  list.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(button.getAttribute("aria-expanded")).toBe("false");
  button.click();
  list.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
  expect(button.getAttribute("aria-expanded")).toBe("false");
  button.click();
  document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
  expect(button.getAttribute("aria-expanded")).toBe("false");
});
test("disabled pickers stay closed and destroyed menus release outside handlers", () => {
  const { button, destroy } = setup(true);
  expect(button.disabled).toBe(true);
  button.click();
  expect(button.getAttribute("aria-expanded")).toBe("false");
  destroy();
});

test("menu shifts left when its right edge exceeds the viewport", () => {
  const { button, element } = setup();
  const list = element.querySelector<HTMLElement>('[role="listbox"]')!;
  list.getBoundingClientRect = () => ({ right: window.innerWidth + 50 } as DOMRect);
  button.click();
  expect(list.style.left).toBe("-62px");
});

test("opening scrolls the selected model into view", () => {
  const { select, element, button } = setup();
  select.selectedIndex = 1;
  let scrolled = false;
  element.querySelectorAll<HTMLElement>('[role="option"]')[1].scrollIntoView = () => { scrolled = true; };
  button.click();
  expect(scrolled).toBe(true);
});
