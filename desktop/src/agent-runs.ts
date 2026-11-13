import type { AgentRunView, AgentRunStatus } from "./agent-types";
import type { ChatState } from "./types";

// A newly requested/finished call or parent completion can reveal runs after an empty check.
// Streaming answer tokens leave this key unchanged and do not cause native requests.
export function agentRunsRevision(state: Pick<ChatState, "activeConversationId" | "status" | "toolActivity">): string {
  return JSON.stringify([state.activeConversationId, state.status,
    state.toolActivity.map(call => [call.id, call.status])]);
}

const MAX_VISIBLE_ACTIVITY = 128;
interface Source {
  list(owner: string): Promise<AgentRunView[]>;
  cancel(owner: string, id: string): Promise<AgentRunView>;
  approve?(owner: string, id: string, approval: string, allow: boolean): Promise<void>;
}
const pending = (run: AgentRunView) => ["queued", "running", "waiting_approval", "cancelling"].includes(run.status);
const labels: Record<AgentRunStatus, string> = {queued: "Queued", running: "Running", waiting_approval: "Waiting for approval", cancelling: "Cancelling", completed: "Completed", failed: "Failed", cancelled: "Cancelled", timed_out: "Timed out"};
function text(tag: string, value: string) {const node = document.createElement(tag); node.textContent = value; return node;}
export function createAgentRuns(source: Source) {
  const element = document.createElement("details"); element.className = "agent-runs";
  element.setAttribute("aria-label", "Agent runs");
  const summary = text("summary", "Agent runs"), notice = text("p", "Expand to check agent runs."), content = document.createElement("div");
  const refreshButton = document.createElement("button");
  refreshButton.type = "button";
  refreshButton.className = "secondary-button";
  refreshButton.textContent = "Refresh";
  refreshButton.setAttribute("aria-label", "Refresh agent runs");
  refreshButton.addEventListener("click", () => {void refresh();});
  notice.setAttribute("role", "status"); element.append(summary, notice, refreshButton, content);
  let owner: string | null = null, revision = "", epoch = 0, request = 0, disposed = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let runs: AgentRunView[] = [];
  let observedOpen = false;
  const answeredApprovals = new Set<string>();
  const openChats = new Set<string>();
  const rows = new Map<string, {element: HTMLDetailsElement; summary: HTMLElement; body: HTMLElement; stop: HTMLButtonElement; approval: HTMLElement; approvalId: string | null}>();
  const busy = new Map<string, symbol>();
  function clearTimer() {if (timer !== null) clearTimeout(timer); timer = null;}
  function valid(expected: number) {return !disposed && epoch === expected && !!owner && element.open;}
  function schedule() {clearTimer(); if (!disposed && owner && element.open && runs.some(pending)) timer = setTimeout(() => {void refresh();}, 1000);}
  function render() {
    summary.textContent = runs.length ? `Agent runs (${runs.length})` : "Agent runs";
    notice.textContent = runs.length ? "" : "No agent runs in this chat."; notice.hidden = !!runs.length;
    for (const [id, row] of rows) if (!runs.some(run => run.run_id === id)) {row.element.remove(); rows.delete(id);}
    for (const run of runs) {
      let row = rows.get(run.run_id);
      if (!row) {
        const detail = document.createElement("details"); detail.dataset.agentRun = run.run_id;
        const heading = text("summary", ""), body = document.createElement("div"), approval = document.createElement("section");
        const stop = document.createElement("button"); stop.type = "button"; stop.dataset.stopAgent = run.run_id; stop.className = "secondary-button";
        stop.addEventListener("click", () => {void act(run.run_id, () => source.cancel(owner!, run.run_id));});
        detail.append(heading, stop, approval, body); content.append(detail);
        row = {element: detail, summary: heading, body, stop, approval, approvalId: null}; rows.set(run.run_id, row);
      }
      row.summary.textContent = `${run.agent} · ${labels[run.status]} · ${Math.max(0, Math.round(run.duration_ms / 1000))}s`;
      row.stop.hidden = !pending(run); row.stop.disabled = run.status === "cancelling" || busy.has(run.run_id);
      row.stop.textContent = run.status === "cancelling" ? "Cancelling…" : "Stop"; row.stop.setAttribute("aria-label", `Stop ${run.agent}`);
      const nodes = [text("p", run.task), text("p", `${run.context_mode === "fork" ? "Forked context" : "Task only"} · ${run.execution_mode === "parallel" ? "Parallel" : "Wait"}${run.model ? ` · ${run.model}` : ""} · ${run.tool_calls} tool calls`)];
      if (run.progress) nodes.push(text("p", run.progress));
      if (run.current_tool) nodes.push(text("p", `Current tool: ${run.current_tool}`));
      if (run.tools.length) nodes.push(text("p", `Tools: ${run.tools.join(", ")}`));
      if (run.error) nodes.push(text("p", run.error));
      if (run.activity?.length) {
        const history = document.createElement("section");
        history.dataset.agentHistory = "";
        history.setAttribute("aria-label", "Agent tool history");
        history.append(text("strong", "Tool history"));
        if (run.activity.length > MAX_VISIBLE_ACTIVITY) history.append(text("p", "Earlier tool activity omitted."));
        const list = document.createElement("ol");
        for (const activity of run.activity.slice(-MAX_VISIBLE_ACTIVITY)) {
          const outcome = activity.success === true ? "Completed" : activity.success === false ? "Failed" : "Outcome unknown";
          list.append(text("li", `${activity.tool} · ${outcome}: ${activity.summary}`));
        }
        history.append(list); nodes.push(history);
      }
      if (run.result) {
        nodes.push(text("p", `${run.result.outcome}: ${run.result.summary}`));
        for (const [label, values] of [["Work done", run.result.work_done], ["Findings", run.result.findings], ["Artifacts", run.result.artifacts], ["Verification", run.result.verification], ["Remaining", run.result.remaining]] as const) if (values.length) nodes.push(text("strong", label), text("pre", values.map(value => typeof value === "string" ? value : JSON.stringify(value, null, 2)).join("\n")));
      }
      row.body.replaceChildren(...nodes);
      const approval = run.pending_approval;
      if (row.approvalId !== (approval?.id ?? null)) {
        row.approvalId = approval?.id ?? null; row.approval.replaceChildren();
        if (approval) {
          row.approval.setAttribute("aria-label", `Allow ${approval.tool}?`);
          row.approval.append(text("strong", `Allow ${approval.tool}?`), text("p", approval.reason), text("pre", approval.arguments));
          for (const [label, allow] of [["Deny", false], ["Allow once", true]] as const) {
            const button = document.createElement("button"); button.type = "button"; button.textContent = label; button.className = "secondary-button";
            button.addEventListener("click", () => {void act(run.run_id, async () => {if (!source.approve) throw new Error("Agent approvals are unavailable"); answeredApprovals.add(approval.id);
              try {await source.approve(owner!, run.run_id, approval.id, allow);} catch (error) {answeredApprovals.delete(approval.id); throw error;}});});
            row.approval.append(button);
          }
        }
      }
      for (const button of row.approval.querySelectorAll("button")) button.disabled = busy.has(run.run_id) || !!approval && answeredApprovals.has(approval.id);
    }
  }
  async function refresh() {
    clearTimer(); if (!owner || !element.open || disposed) return;
    const expected = epoch, sequence = ++request, capturedOwner = owner;
    try {const result = await source.list(capturedOwner); if (!valid(expected) || sequence !== request) return; runs = result; render(); schedule();}
    catch (error) {if (!valid(expected) || sequence !== request) return; notice.hidden = false; notice.textContent = `Unable to check agent runs: ${String(error)}`; schedule();}
  }
  async function act(id: string, action: () => Promise<AgentRunView | void>) {
    if (!owner || disposed || busy.has(id)) return;
    const expected = epoch, actionOwner = owner, actionToken = Symbol(id);
    busy.set(id, actionToken); clearTimer(); ++request; render();
    try {
      const result = await action(); if (!valid(expected)) return;
      if (result) {runs = runs.map(run => run.run_id === id ? result : run); busy.delete(id); render(); schedule();}
      else {await refresh();}
    } catch (error) {if (!valid(expected)) return; notice.hidden = false; notice.textContent = `Agent action failed: ${String(error)}`; schedule();}
    finally {
      // Closing a disclosure invalidates displayed results, but does not change action ownership.
      // A later action in another chat (or a reopened owner) has its own token.
      if (owner === actionOwner && busy.get(id) === actionToken) {
        busy.delete(id);
        const row = rows.get(id);
        if (row) {
          for (const button of row.approval.querySelectorAll("button")) button.disabled = !!row.approvalId && answeredApprovals.has(row.approvalId);
          row.stop.disabled = runs.find(run => run.run_id === id)?.status === "cancelling";
        }
      }
    }
  }
  element.addEventListener("toggle", () => {
    if (disposed || !owner || observedOpen === element.open) return;
    observedOpen = element.open;
    if (element.open) {openChats.add(owner); void refresh();}
    else {openChats.delete(owner); ++epoch; clearTimer();}
  });
  return {element, update(nextOwner: string | null, nextRevision = "") {
    if (disposed) return;
    const changed = nextOwner !== owner, refreshNeeded = changed || nextRevision !== revision; revision = nextRevision;
    if (changed) {
      if (owner && element.open) openChats.add(owner);
      ++epoch; ++request; clearTimer(); owner = nextOwner; runs = []; rows.clear(); busy.clear(); content.replaceChildren();
      element.open = !!owner && openChats.has(owner); observedOpen = element.open; render();
    }
    element.hidden = !owner; if (refreshNeeded && element.open) void refresh();
  }, destroy() {disposed = true; ++epoch; clearTimer();}};
}
