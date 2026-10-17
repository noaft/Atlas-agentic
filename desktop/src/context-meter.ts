export interface ContextReading {
  maxTokens: number | null;
  usedTokens: number | null;
  loading?: boolean;
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
      <span class="context-token-ratio" data-context-ratio></span>
      <span data-model-context></span>
      <span class="context-help" data-context-help></span>
    </div>`;
  const button = element.querySelector<HTMLButtonElement>("button")!;
  const ring = element.querySelector<HTMLElement>("[data-context-ring]")!;
  const percent = element.querySelector<HTMLElement>("[data-context-percent]")!;
  const tooltip = element.querySelector<HTMLElement>("[data-context-tooltip]")!;
  const ratio = element.querySelector<HTMLElement>("[data-context-ratio]")!;
  const maximum = element.querySelector<HTMLElement>("[data-model-context]")!;
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
    update({ maxTokens, usedTokens, loading = false }) {
      const max = !loading && tokenCount(maxTokens, 1) ? maxTokens : null;
      const used = tokenCount(usedTokens, 0) ? usedTokens : null;
      const percentage = max !== null && used !== null ? used / max * 100 : null;
      const label = percentage === null ? "—" : percentage > 100 ? ">100%" : `${Math.round(percentage)}%`;
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
        ring.setAttribute("aria-valuenow", String(Math.min(100, Math.round(percentage))));
      }
      const maxLabel = max === null ? "Unknown" : formatTokens(max);
      ratio.textContent = `${used === null ? "—" : formatTokens(used)} / ${maxLabel} tokens`;
      maximum.textContent = loading ? "Max context: Loading…"
        : max === null ? "Max context: Unknown" : `Max context: ${maxLabel} tokens (input + output)`;
      help.textContent = used === null ? "Token usage is not available yet." : "Input and output share this context window.";
      button.setAttribute("aria-label", `Context usage ${percentage === null ? "unavailable" : label}. ${maximum.textContent}`);
    },
    destroy() {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", dismissEscape);
      element.remove();
    },
  };
}
