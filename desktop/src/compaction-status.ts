import type {CompactionProgress} from "./types";

interface CompactionStatusInput {
  sessionId: string;
  progress: CompactionProgress | null;
  message: string | null;
  failure: string | null;
  retryAvailable: boolean;
  retryDisabled: boolean;
}

export function createCompactionStatus(onCancel: () => void, onRetry: () => void) {
  const element = document.createElement("section");
  element.className = "compaction-card";
  element.dataset.compactionStatus = "";
  element.hidden = true;
  element.setAttribute("aria-label", "Conversation compaction");
  let signature = "";
  function action(label: string, callback: () => void): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button"; button.className = "compaction-action"; button.textContent = label;
    button.addEventListener("click", callback);
    return button;
  }
  return {
    element,
    update(input: CompactionStatusInput) {
      const retry = element.querySelector<HTMLButtonElement>("[data-retry-compaction]");
      if (retry) retry.disabled = input.retryDisabled;
      const {retryDisabled: _disabled, ...content} = input;
      const next = JSON.stringify(content);
      if (next === signature) return;
      signature = next;
      element.hidden = !input.progress && !input.message && !input.failure;
      if (element.hidden) {element.replaceChildren(); return;}
      const stopped = input.failure?.startsWith("Compaction cancelled");
      element.dataset.phase = input.progress ? "running" : input.failure ? (stopped ? "stopped" : "failed") : "done";
      const title = document.createElement("strong");
      title.className = "compaction-title";
      title.textContent = input.progress ? "Compacting context" : input.failure ? (stopped ? "Compaction stopped" : "Compaction failed") : "Compaction finished";
      const summary = document.createElement("div");
      summary.className = "compaction-summary";
      summary.setAttribute("role", input.failure ? "alert" : "status");
      const subtitle = document.createElement("p");
      subtitle.textContent = input.failure ? "History unchanged · Saved memory retained" : input.progress
        ? "Summarizing older messages while keeping recent context available."
        : input.message;
      summary.append(title, subtitle);
      const actions = document.createElement("div"); actions.className = "compaction-actions";
      element.replaceChildren(summary);
      if (input.progress) {
        const {completedBlocks, totalBlocks} = input.progress;
        const progress = document.createElement("progress"); progress.setAttribute("aria-label", "Summary blocks completed");
        const label = document.createElement("p"); label.className = "compaction-progress-label";
        if (totalBlocks > 0) {
          progress.max = totalBlocks; progress.value = completedBlocks;
          label.textContent = completedBlocks >= totalBlocks ? "Saving memory…" : `Block ${completedBlocks + 1} of ${totalBlocks}`;
          const blocks = document.createElement("ol"); blocks.className = "compaction-blocks";
          // The memory policy publishes at most three blocks.
          for (let index = 0; index < Math.min(totalBlocks, 3); index++) {
            const block = document.createElement("li");
            const phase = index < completedBlocks ? "complete" : index === completedBlocks ? "active" : "waiting";
            block.dataset.blockState = phase;
            block.textContent = `${phase === "complete" ? "✓ " : ""}Block ${index + 1}`;
            block.setAttribute("aria-label", `Block ${index + 1}: ${phase}`);
            blocks.append(block);
          }
          element.append(blocks);
        } else {label.textContent = "Preparing conversation…";}
        summary.append(label);
        element.append(progress);
        const cancel = action("Cancel compaction", onCancel); cancel.dataset.cancelCompaction = "";
        actions.append(cancel);
      }
      if (input.failure) {
        if (input.retryAvailable) {
          const retry = action("Retry compaction", onRetry); retry.dataset.retryCompaction = "";
          retry.disabled = input.retryDisabled;
          actions.append(retry);
        }
        const details = document.createElement("details"); details.className = "compaction-details";
        const label = document.createElement("summary"); label.textContent = "Details";
        const error = document.createElement("p"); error.textContent = input.failure;
        details.append(label, error); element.append(details);
      }
      if (actions.childElementCount) element.append(actions);
    },
  };
}
