import {afterEach, expect, test, vi} from "vitest";
import {createCompactionStatus} from "./compaction-status";
afterEach(() => document.body.replaceChildren());
const input = {sessionId: "one", progress: null, message: null, failure: null, retryAvailable: true, retryDisabled: false};
test("progress explains the active block and supports cancellation", () => {
  const cancel = vi.fn(); const view = createCompactionStatus(cancel, vi.fn()); document.body.append(view.element);
  view.update({...input, progress: {completedBlocks: 1, totalBlocks: 3}});
  expect(view.element.textContent).toContain("Block 2 of 3");
  expect(view.element.querySelector('progress')?.value).toBe(1);
  expect(view.element.querySelector('progress')?.max).toBe(3);
  expect(view.element.querySelectorAll('[data-block-state="complete"]')).toHaveLength(1);
  view.element.querySelector<HTMLButtonElement>('[data-cancel-compaction]')!.click(); expect(cancel).toHaveBeenCalledOnce();
});
test("preparation has indeterminate progress instead of a fabricated percentage", () => {
  const view = createCompactionStatus(vi.fn(), vi.fn()); view.update({...input, progress: {completedBlocks: 0, totalBlocks: 0}});
  expect(view.element.textContent).toContain("Preparing"); expect(view.element.querySelector('progress')?.hasAttribute("value")).toBe(false);
});
test("failed compaction offers retry and stable expandable details", () => {
  const retry = vi.fn(); const view = createCompactionStatus(vi.fn(), retry); document.body.append(view.element);
  const failed = {...input, failure: "Summary reached its 2048-token output limit"}; view.update(failed);
  expect(view.element.textContent).toContain("Compaction failed"); expect(view.element.textContent).toContain("History unchanged");
  const details = view.element.querySelector('details')!; details.open = true;
  const button = view.element.querySelector<HTMLButtonElement>('[data-retry-compaction]')!; button.focus();
  view.update({...failed}); expect(view.element.querySelector('details')).toBe(details);
  expect(details.open).toBe(true); expect(document.activeElement).toBe(button);
  button.click(); expect(retry).toHaveBeenCalledOnce();
  view.update({...failed, retryDisabled: true}); expect(view.element.querySelector("details")?.open).toBe(true); expect(view.element.querySelector<HTMLButtonElement>('[data-retry-compaction]')?.disabled).toBe(true);
});
test("success and cancellation are distinct; an inactive status is hidden", () => {
  const view = createCompactionStatus(vi.fn(), vi.fn()); view.update({...input, message: "Saved 2 memory blocks"});
  expect(view.element.textContent).toContain("Compaction finished");
  view.update({...input, failure: "Compaction cancelled; original memory retained"}); expect(view.element.textContent).toContain("Compaction stopped");
  view.update(input); expect(view.element.hidden).toBe(true);
});
