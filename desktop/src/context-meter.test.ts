import { afterEach, expect, test } from "vitest";
import { createContextMeter } from "./context-meter";

const meters: ReturnType<typeof createContextMeter>[] = [];
function setup() {
  const meter = createContextMeter();
  meters.push(meter);
  document.body.append(meter.element);
  return meter;
}
afterEach(() => {
  for (const meter of meters.splice(0)) meter.destroy();
  document.body.replaceChildren();
});

test("unknown usage never looks like zero percent", () => {
  const meter = setup();
  meter.update({ maxTokens: 128000, usedTokens: null });
  expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe("—");
  expect(meter.element.querySelector("[data-context-ratio]")?.textContent).toBe("— / 128,000 tokens");
  expect(meter.element.querySelector("[role='progressbar']")).toBeNull();
});

test("known usage shows the percentage with exact token details", () => {
  const meter = setup();
  meter.update({ maxTokens: 128000, usedTokens: 16000 });
  expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe("13%");
  expect(meter.element.querySelector("[data-context-ratio]")?.textContent).toBe("16,000 / 128,000 tokens");
  expect(meter.element.querySelector("[role='progressbar']")?.getAttribute("aria-valuenow")).toBe("12.5");
  meter.update({ maxTokens: 128000, usedTokens: 0 });
  expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe("0%");
});

test("invalid limits or usage do not produce a misleading percentage", () => {
  const meter = setup();
  for (const [maxTokens, usedTokens] of [[0, 10], [NaN, 10], [128000, -1], [128000, Infinity]]) {
    meter.update({ maxTokens, usedTokens });
    expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe("—");
  }
  meter.update({ maxTokens: 100, usedTokens: 150 });
  expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe(">100%");
  expect(meter.element.querySelector("[data-context-ratio]")?.textContent).toBe("150 / 100 tokens");
});

test("details work with focus, escape, pointer, touch, and outside dismissal", () => {
  const meter = setup();
  meter.update({ maxTokens: 128000, usedTokens: null });
  const button = meter.element.querySelector<HTMLButtonElement>("button")!;
  const tooltip = meter.element.querySelector<HTMLElement>("[role='tooltip']")!;
  expect(button.type).toBe("button");
  button.focus();
  expect(tooltip.hidden).toBe(false);
  button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(tooltip.hidden).toBe(true);
  button.blur();
  meter.element.dispatchEvent(new Event("pointerenter"));
  expect(tooltip.hidden).toBe(false);
  meter.element.dispatchEvent(new Event("pointerleave"));
  expect(tooltip.hidden).toBe(true);
  button.click();
  expect(tooltip.hidden).toBe(false);
  document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
  expect(tooltip.hidden).toBe(true);
});

test("hover details stay reachable and Escape dismisses them while the composer has focus", () => {
  const meter = setup();
  meter.update({ maxTokens: 128000, usedTokens: null });
  const composer = document.createElement("textarea");
  document.body.append(composer);
  composer.focus();
  const button = meter.element.querySelector<HTMLButtonElement>("button")!;
  const tooltip = meter.element.querySelector<HTMLElement>("[role='tooltip']")!;
  button.dispatchEvent(new Event("pointerenter"));
  meter.element.dispatchEvent(new Event("pointerenter"));
  expect(tooltip.hidden).toBe(false);
  button.dispatchEvent(new Event("pointerleave"));
  tooltip.dispatchEvent(new Event("pointerenter"));
  expect(tooltip.hidden).toBe(false);
  composer.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(tooltip.hidden).toBe(true);
  expect(document.activeElement).toBe(composer);
});

test("small positive usage stays distinguishable from an unused context", () => {
  const meter = setup();
  meter.update({ maxTokens: 1048576, usedTokens: 512 });
  expect(meter.element.querySelector("[data-context-percent]")?.textContent).toBe("<1%");
  expect(meter.element.querySelector("[data-context-ratio]")?.textContent).toBe("512 / 1,048,576 tokens");
});
