export interface ContextReading {
  maxTokens: number | null;
  usedTokens: number | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  loading?: boolean;
  provider?: string;
  model?: string;
  inputTokenLimit?: number | null;
}

export interface ContextMeter {
  element: HTMLElement;
  update(reading: ContextReading): void;
  destroy(): void;
}

let nextTooltip = 0;
const tokenCount = (value: number | null, minimum: number): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= minimum;
const formatTokens = (value: number) => value.toLocaleString("en-US");

export function createContextMeter(): ContextMeter {
  const element = document.createElement("div");
  element.className = "context-meter";
  element.innerHTML = `
    <button type="button" class="context-meter-toggle" data-context-meter>
      <span class="context-ring" data-context-ring><span data-context-percent>—</span></span>
    </button>
    <div class="context-tooltip" role="tooltip" data-context-tooltip hidden>
      <strong>Context window</strong>
      <span data-context-source hidden></span>
      <span data-input-limit hidden></span>
      <span class="context-token-ratio" data-context-ratio></span>
      <span data-model-context></span>
      <span data-context-breakdown hidden></span>
      <span class="context-help" data-context-help></span>
    </div>`;
  const button = element.querySelector<HTMLButtonElement>("button")!;
  const ring = element.querySelector<HTMLElement>("[data-context-ring]")!;
  const percent = element.querySelector<HTMLElement>("[data-context-percent]")!;
  const tooltip = element.querySelector<HTMLElement>("[data-context-tooltip]")!;
  const ratio = element.querySelector<HTMLElement>("[data-context-ratio]")!;
  const maximum = element.querySelector<HTMLElement>("[data-model-context]")!;
  const source = element.querySelector<HTMLElement>("[data-context-source]")!;
  const inputLimit = element.querySelector<HTMLElement>("[data-input-limit]")!;
  const breakdown = element.querySelector<HTMLElement>("[data-context-breakdown]")!;
  const help = element.querySelector<HTMLElement>("[data-context-help]")!;
  tooltip.id = `context-tooltip-${++nextTooltip}`;
  button.setAttribute("aria-describedby", tooltip.id);
  let pinned = false;
  const close = () => { pinned = false; tooltip.hidden = true; };
  button.addEventListener("focus", () => { tooltip.hidden = false; });
  button.addEventListener("blur", close);
  element.addEventListener("pointerenter", () => { tooltip.hidden = false; });
  element.addEventListener("pointerleave", () => {
    if (!pinned && document.activeElement !== button) tooltip.hidden = true;
  });
  button.addEventListener("click", () => {
    pinned = !pinned;
    tooltip.hidden = !pinned;
  });
  const dismissEscape = (event: KeyboardEvent) => {
    if (event.key === "Escape" && !tooltip.hidden) { event.preventDefault(); close(); }
  };
  document.addEventListener("keydown", dismissEscape);
  const dismissOutside = (event: Event) => {
    if (event.target instanceof Node && !element.contains(event.target)) close();
  };
  document.addEventListener("pointerdown", dismissOutside);

  return {
    element,
    update({ maxTokens, usedTokens, inputTokens = null, outputTokens = null, loading = false, provider, model, inputTokenLimit = null }) {
      source.hidden = !provider || !model; source.textContent = source.hidden ? "" : `${provider} · ${model}`;
      inputLimit.hidden = !tokenCount(inputTokenLimit, 1); inputLimit.textContent = inputLimit.hidden ? "" : `Input limit: ${formatTokens(inputTokenLimit!)} tokens`;
      const max = !loading && tokenCount(maxTokens, 1) ? maxTokens : null;
      const used = tokenCount(usedTokens, 0) ? usedTokens : null;
      const percentage = max !== null && used !== null ? used / max * 100 : null;
      const label = percentage === null ? "—" : percentage > 100 ? ">100%" : percentage > 0 && percentage < 1 ? "<1%" : `${Math.round(percentage)}%`;
      percent.textContent = label;
      ring.classList.toggle("context-ring-unknown", percentage === null);
      if (percentage === null) {
        ring.style.removeProperty("--context-progress");
        ring.removeAttribute("role");
        ring.removeAttribute("aria-valuenow");
        ring.removeAttribute("aria-valuemin");
        ring.removeAttribute("aria-valuemax");
        ring.removeAttribute("aria-label");
      } else {
        ring.style.setProperty("--context-progress", `${Math.min(100, percentage)}%`);
        ring.setAttribute("role", "progressbar");
        ring.setAttribute("aria-label", "Context usage");
        ring.setAttribute("aria-valuemin", "0");
        ring.setAttribute("aria-valuemax", "100");
        ring.setAttribute("aria-valuenow", String(Math.min(100, percentage)));
      }
      const maxLabel = max === null ? "Unknown" : formatTokens(max);
      ratio.textContent = `${used === null ? "—" : formatTokens(used)} / ${maxLabel} tokens`;
      maximum.textContent = loading ? "Max context: Loading…"
        : max === null ? "Max context: Unknown" : `Max context: ${maxLabel} tokens (input + output)`;
      breakdown.hidden = !tokenCount(inputTokens, 0) || !tokenCount(outputTokens, 0);
      breakdown.textContent = breakdown.hidden ? "" : `Input: ${formatTokens(inputTokens!)} · Output: ${formatTokens(outputTokens!)}`;
      help.textContent = used === null ? "No token usage reported for this model yet."
        : "Last model call. Input includes system instructions, history, and tools.";
      button.setAttribute("aria-label", `Context usage ${percentage === null ? "unavailable" : label}. ${maximum.textContent}`);
    },
    destroy() {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
      element.remove();
    },
  };
}
