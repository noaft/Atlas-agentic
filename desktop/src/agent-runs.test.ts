import { createTauriAtlasApi } from "./atlas-api";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { agentRunsRevision, createAgentRuns } from "./agent-runs";
import type { ChatState, ToolActivity } from "./types";
import type { AgentRunView } from "./agent-types";

const run = (status: AgentRunView["status"] = "running"): AgentRunView => ({
  run_id: "run-1", agent: "Researcher", task: "Review <script>unsafe</script>", context_mode: "fork",
  execution_mode: "parallel", status, tools: ["read_file"], progress: "Reading", current_tool: "read_file",
  started_at: "2026-11-13T00:00:00Z", duration_ms: 2500, tool_calls: 2, result: null, error: null,
});
const flush = async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); };
function open(view: ReturnType<typeof createAgentRuns>) {
  view.element.open = true;
  view.element.dispatchEvent(new Event("toggle"));
}
afterEach(() => { clearMocks(); vi.useRealTimers(); document.body.replaceChildren(); });
describe("agent runs", () => {
  it("loads on expansion and renders actual status and plain-text result details", async () => {
    const completed = {...run("completed"), result: {outcome: "partial" as const, summary: "Reviewed", work_done: ["Read files"], findings: [{path: "a.rs"}], artifacts: ["notes.txt"], verification: ["cargo check"], remaining: ["Run tests"]}};
    const list = vi.fn().mockResolvedValue([completed]);
    const view = createAgentRuns({list, cancel: vi.fn()});
    view.update("chat-a"); expect(list).not.toHaveBeenCalled();
    open(view); await flush();
    expect(view.element.textContent).toContain("Completed");
    expect(view.element.textContent).toContain("Run tests");
    expect(view.element.textContent).toContain("Review <script>unsafe</script>");
    expect(view.element.querySelector("script")).toBeNull();
    expect(view.element.querySelector<HTMLButtonElement>("button[data-stop-agent]")!.hidden).toBe(true);
    view.destroy();
  });
  it("rejects navigation-stale responses and restores each chat disclosure", async () => {
    let resolve!: (runs: AgentRunView[]) => void;
    const list = vi.fn().mockImplementationOnce(() => new Promise(r => {resolve = r;})).mockResolvedValue([]);
    const view = createAgentRuns({list, cancel: vi.fn()});
    view.update("chat-a"); open(view); view.update("chat-b");
    resolve([run()]); await flush();
    expect(view.element.textContent).not.toContain("Researcher");
    expect(view.element.open).toBe(false);
    view.update("chat-a"); await flush(); expect(view.element.open).toBe(true);
    view.destroy();
  });
  it("polls pending runs only while open and cleans up on close and destroy", async () => {
    vi.useFakeTimers();
    const list = vi.fn().mockResolvedValue([run()]);
    const view = createAgentRuns({list, cancel: vi.fn()});
    view.update("chat-a"); open(view); await flush();
    await vi.advanceTimersByTimeAsync(1000); expect(list).toHaveBeenCalledTimes(2);
    view.element.open = false; view.element.dispatchEvent(new Event("toggle"));
    await vi.advanceTimersByTimeAsync(2000); expect(list).toHaveBeenCalledTimes(2);
    open(view); await flush(); view.destroy();
    const calls = list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(2000); expect(list).toHaveBeenCalledTimes(calls);
  });
  it("keeps Stop as Cancelling until the backend reports completion and preserves details", async () => {
    vi.useFakeTimers();
    const list = vi.fn().mockResolvedValue([run()]);
    const cancel = vi.fn().mockResolvedValue(run("cancelling"));
    const view = createAgentRuns({list, cancel});
    view.update("chat-a"); open(view); await flush();
    const details = view.element.querySelector<HTMLDetailsElement>("[data-agent-run]")!;
    details.open = true;
    view.element.querySelector<HTMLButtonElement>("[data-stop-agent]")!.click(); await flush();
    expect(cancel).toHaveBeenCalledWith("chat-a", "run-1");
    expect(view.element.textContent).toContain("Cancelling");
    expect(details.open).toBe(true);
    list.mockResolvedValue([run("cancelled")]);
    await vi.advanceTimersByTimeAsync(1000);
    expect(view.element.textContent).toContain("Cancelled");
    expect(vi.getTimerCount()).toBeLessThanOrEqual(1);
    view.destroy();
  });
});

it("resolves only the child approval and keeps buttons disabled until native state clears", async () => {
  vi.useFakeTimers();
  const approval = {id: "approval-1", tool: "exec_command", arguments: "<script>hi</script>", reason: "Run command"};
  const list = vi.fn().mockResolvedValue([{...run("waiting_approval"), pending_approval: approval}]);
  const approve = vi.fn().mockResolvedValue(undefined);
  const view = createAgentRuns({list, cancel: vi.fn(), approve});
  view.update("child-chat"); open(view); await flush();
  const buttons = [...view.element.querySelectorAll<HTMLButtonElement>("section button")];
  expect(buttons.map(button => button.textContent)).toEqual(["Deny", "Allow once"]);
  buttons[1].click(); await flush();
  expect(approve).toHaveBeenCalledWith("child-chat", "run-1", "approval-1", true);
  expect(buttons[1].disabled).toBe(true);
  expect(view.element.querySelector("script")).toBeNull();
  list.mockResolvedValue([{...run(), pending_approval: null}]);
  await vi.advanceTimersByTimeAsync(1000);
  expect(view.element.querySelector("section button")).toBeNull();
  view.destroy();
});

it("uses native owner context and the shared process client for list, stop and approval", async () => {
  const calls: Array<{command: string; payload: unknown}> = [];
  mockIPC((command, payload) => {
    calls.push({command, payload});
    if (command === "initialize_process_client") return "client-1";
    if (command === "list_agent_runs") return [run()];
    if (command === "cancel_agent_run") return run("cancelling");
  });
  const api = createTauriAtlasApi();
  const context = {conversationId: "chat-1", scopeId: "scope-1"};
  await api.listAgentRuns!(context);
  await api.cancelAgentRun!(context, "run-1");
  await api.resolveAgentRunApproval!(context, "run-1", "approval-1", false);
  expect(calls).toEqual([
    {command: "initialize_process_client", payload: {}},
    {command: "list_agent_runs", payload: {client: "client-1", context}},
    {command: "cancel_agent_run", payload: {client: "client-1", context, runId: "run-1"}},
    {command: "resolve_agent_run_approval", payload: {client: "client-1", context, runId: "run-1", approvalId: "approval-1", allow: false}},
  ]);
});

it("discovers new runs after an empty check when call_agent events or the parent status change", async () => {
  vi.useFakeTimers();
  const list = vi.fn().mockResolvedValue([]);
  const view = createAgentRuns({list, cancel: vi.fn()});
  const state: Pick<ChatState, "activeConversationId" | "status" | "toolActivity"> = {
    activeConversationId: null, status: "idle", toolActivity: [],
  };
  const update = () => view.update("chat-a", agentRunsRevision(state));
  update(); open(view); await flush();
  await vi.advanceTimersByTimeAsync(2000);
  expect(list).toHaveBeenCalledTimes(1);
  state.status = "streaming"; update(); await flush();
  expect(list).toHaveBeenCalledTimes(2);
  const call: ToolActivity = {id: "tool-1", sequence: 1, kind: "tool", tool: "call_agent", status: "requested", summary: "Calling researcher", arguments: "{}", details: null};
  state.toolActivity = [call]; update(); await flush();
  expect(list).toHaveBeenCalledTimes(3);
  // Unchanged state during answer deltas does not produce checks.
  update(); update(); await flush();
  expect(list).toHaveBeenCalledTimes(3);
  list.mockResolvedValue([run("completed")]);
  state.toolActivity = [{...call, status: "succeeded"}]; update(); await flush();
  expect(view.element.textContent).toContain("Researcher · Completed");
  state.status = "idle"; update(); await flush();
  expect(list).toHaveBeenCalledTimes(5);
  await vi.advanceTimersByTimeAsync(2000);
  expect(list).toHaveBeenCalledTimes(5);
  view.destroy();
});

it("retains approval focus while refresh updates bounded plain-text tool history", async () => {
  vi.useFakeTimers();
  const approval = {id: "approval-1", tool: "exec_command", arguments: "{}", reason: "Run command"};
  const current = {...run("waiting_approval"), pending_approval: approval,
    activity: [{tool: "read_file", success: true, summary: "Read source"}]};
  const list = vi.fn().mockResolvedValue([current]);
  const view = createAgentRuns({list, cancel: vi.fn(), approve: vi.fn()});
  document.body.append(view.element);
  view.update("chat-a"); open(view); await flush();
  const details = view.element.querySelector<HTMLDetailsElement>("[data-agent-run]")!;
  details.open = true;
  const allow = [...details.querySelectorAll<HTMLButtonElement>("section button")][1];
  allow.focus(); expect(document.activeElement).toBe(allow);
  list.mockResolvedValue([{...current, activity: Array.from({length: 130}, (_, index) => ({
    tool: "read_file", success: index % 2 === 0, summary: index === 129 ? "<script>latest</script>" : `History ${index}`,
  }))}]);
  await vi.advanceTimersByTimeAsync(1000);
  expect(document.activeElement).toBe(allow);
  expect(details.open).toBe(true);
  expect(view.element.querySelectorAll("[data-agent-history] li")).toHaveLength(128);
  expect(view.element.textContent).toContain("Earlier tool activity omitted.");
  expect(view.element.textContent).toContain("Failed: <script>latest</script>");
  expect(view.element.querySelector("script")).toBeNull();
  view.destroy();
});

it("releases Stop action ownership when its response settles while the disclosure is collapsed", async () => {
  let resolve!: (value: AgentRunView) => void;
  const cancel = vi.fn().mockImplementation(() => new Promise<AgentRunView>(done => {resolve = done;}));
  const list = vi.fn().mockResolvedValue([run()]);
  const view = createAgentRuns({list, cancel});
  view.update("chat-a"); open(view); await flush();
  const stop = view.element.querySelector<HTMLButtonElement>("[data-stop-agent]")!;
  stop.click(); expect(stop.disabled).toBe(true);
  view.element.open = false; view.element.dispatchEvent(new Event("toggle"));
  resolve(run()); await flush();
  open(view); await flush();
  expect(view.element.querySelector<HTMLButtonElement>("[data-stop-agent]")!.disabled).toBe(false);
  view.destroy();
});

it("releases approval action ownership while collapsed so a new approval in the same run is usable", async () => {
  let resolve!: () => void;
  const approve = vi.fn().mockImplementation(() => new Promise<void>(done => {resolve = done;}));
  const approval = {id: "approval-1", tool: "exec_command", arguments: "{}", reason: "Run command"};
  const list = vi.fn().mockResolvedValue([{...run("waiting_approval"), pending_approval: approval}]);
  const view = createAgentRuns({list, cancel: vi.fn(), approve});
  view.update("chat-a"); open(view); await flush();
  view.element.querySelector<HTMLButtonElement>("[data-agent-run] section button")!.click();
  view.element.open = false; view.element.dispatchEvent(new Event("toggle"));
  resolve(); await flush();
  list.mockResolvedValue([{...run("waiting_approval"), pending_approval: {...approval, id: "approval-2"}}]);
  open(view); await flush();
  expect(view.element.querySelector<HTMLButtonElement>("[data-agent-run] section button")!.disabled).toBe(false);
  view.destroy();
});

it("retries a transient monitoring failure while pending and stops retries when collapsed", async () => {
  vi.useFakeTimers();
  const list = vi.fn().mockResolvedValueOnce([run()]).mockRejectedValueOnce(new Error("Temporary connection error")).mockResolvedValue([run("completed")]);
  const view = createAgentRuns({list, cancel: vi.fn()});
  view.update("chat-a"); open(view); await flush();
  await vi.advanceTimersByTimeAsync(1000);
  expect(view.element.textContent).toContain("Temporary connection error");
  await vi.advanceTimersByTimeAsync(1000);
  expect(list).toHaveBeenCalledTimes(3);
  expect(view.element.textContent).toContain("Completed");
  list.mockResolvedValue([run()]);
  view.update("chat-a", "new-run"); await flush();
  view.element.open = false; view.element.dispatchEvent(new Event("toggle"));
  await vi.advanceTimersByTimeAsync(3000);
  expect(list).toHaveBeenCalledTimes(4);
  view.destroy();
});

it("provides an explicit retry after an initial check fails", async () => {
  const list = vi.fn().mockRejectedValueOnce(new Error("Offline")).mockResolvedValue([run("completed")]);
  const view = createAgentRuns({list, cancel: vi.fn()});
  view.update("chat-a"); open(view); await flush();
  const refresh = view.element.querySelector<HTMLButtonElement>("[aria-label='Refresh agent runs']");
  expect(refresh).not.toBeNull(); refresh!.click(); await flush();
  expect(view.element.textContent).toContain("Completed");
  view.destroy();
});

it("does not let an old action clear a newer action after navigating away and back", async () => {
  const resolutions: Array<(value: AgentRunView) => void> = [];
  const cancel = vi.fn().mockImplementation(() => new Promise<AgentRunView>(done => {resolutions.push(done);}));
  const view = createAgentRuns({list: vi.fn().mockResolvedValue([run()]), cancel});
  view.update("chat-a"); open(view); await flush();
  view.element.querySelector<HTMLButtonElement>("[data-stop-agent]")!.click();
  view.update("chat-b"); view.update("chat-a"); await flush();
  const currentStop = view.element.querySelector<HTMLButtonElement>("[data-stop-agent]")!;
  currentStop.click(); expect(currentStop.disabled).toBe(true);
  resolutions[0](run()); await flush();
  expect(currentStop.disabled).toBe(true);
  resolutions[1](run()); await flush();
  expect(currentStop.disabled).toBe(false);
  view.destroy();
});

it("cancels a scheduled transient-error retry on navigation", async () => {
  vi.useFakeTimers();
  const list = vi.fn().mockResolvedValueOnce([run()]).mockRejectedValue(new Error("Offline"));
  const view = createAgentRuns({list, cancel: vi.fn()});
  view.update("chat-a"); open(view); await flush();
  await vi.advanceTimersByTimeAsync(1000);
  expect(list).toHaveBeenCalledTimes(2);
  view.update("chat-b");
  await vi.advanceTimersByTimeAsync(3000);
  expect(list).toHaveBeenCalledTimes(2);
  view.destroy();
});

it("inline agents stay hidden until used, then show active runs without a separate bar", async () => {
  vi.useFakeTimers();
  const list = vi.fn().mockResolvedValue([run()]);
  const view = createAgentRuns({list, cancel: vi.fn()}, {inline: true});
  view.update("chat-a", "initial", false);
  expect(view.element.hidden).toBe(true); expect(list).not.toHaveBeenCalled();
  view.update("chat-a", "called", true); await flush();
  expect(view.element.hidden).toBe(false);
  expect(view.element.querySelector("summary")?.hidden).toBe(true);
  expect(view.element.textContent).toContain("Researcher · Running");
  list.mockResolvedValue([run("completed")]);
  await vi.advanceTimersByTimeAsync(1000);
  expect(view.element.hidden).toBe(true);
  view.update("chat-b", "other", false);
  expect(view.element.hidden).toBe(true);
  view.destroy();
});

it("inline child approvals remain visible and actionable", async () => {
  const approval = {id: "child-approval", tool: "exec_command", arguments: "{}", reason: "Run command"};
  const list = vi.fn().mockResolvedValue([{...run("waiting_approval"), pending_approval: approval}]);
  const approve = vi.fn().mockResolvedValue(undefined);
  const view = createAgentRuns({list, cancel: vi.fn(), approve}, {inline: true});
  view.update("chat-a", "called", true); await flush();
  expect(view.element.querySelector<HTMLDetailsElement>("[data-agent-run]")?.open).toBe(true);
  const allow = [...view.element.querySelectorAll("button")].find(button => button.textContent === "Allow once")!;
  allow.click(); await flush();
  expect(approve).toHaveBeenCalledWith("chat-a", "run-1", "child-approval", true);
  view.destroy();
});


it("inline monitoring discovers a child registered after the parent's call starts", async () => {
  vi.useFakeTimers();
  const list = vi.fn().mockResolvedValueOnce([]).mockResolvedValue([run("waiting_approval")]);
  const view = createAgentRuns({list, cancel: vi.fn()}, {inline: true});
  view.update("chat-a", "starting", true, true); await flush();
  expect(view.element.hidden).toBe(true);
  await vi.advanceTimersByTimeAsync(1000);
  expect(list).toHaveBeenCalledTimes(2);
  expect(view.element.hidden).toBe(false);
  view.destroy();
});

it("keeps the compact Stop control in the agent header without toggling its details", async () => {
  const cancel = vi.fn().mockResolvedValue(run("cancelling"));
  const view = createAgentRuns({list: vi.fn().mockResolvedValue([run()]), cancel}, {inline: true});
  view.update("chat-a", "called", true); await flush();
  const details = view.element.querySelector<HTMLDetailsElement>("[data-agent-run]")!;
  const stop = details.querySelector<HTMLButtonElement>("summary .agent-stop")!;
  expect(stop).not.toBeNull();
  expect(stop.getAttribute("aria-label")).toBe("Stop Researcher");
  stop.click(); await flush();
  expect(cancel).toHaveBeenCalledWith("chat-a", "run-1");
  expect(details.open).toBe(false);
  expect(stop.disabled).toBe(true);
  expect(stop.getAttribute("aria-busy")).toBe("true");
  expect(stop.dataset.state).toBe("cancelling");
  view.destroy();
});
