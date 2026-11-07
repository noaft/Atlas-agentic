/** Measure an inert mirror so native editing never sees a collapsed input box. */
export function createTextareaLayout(textarea: HTMLTextAreaElement) {
  let mirror: HTMLTextAreaElement | null = null;
  const metrics = [
    "font-family", "font-size", "font-style", "font-weight", "font-variant",
    "letter-spacing", "word-spacing", "line-height", "text-indent", "text-transform",
    "tab-size", "white-space", "overflow-wrap", "word-break",
    "padding-top", "padding-right", "padding-bottom", "padding-left",
  ];
  const pixels = (value: string) => Number.parseFloat(value) || 0;
  return {
    refresh() {
      if (!textarea.isConnected || textarea.closest("[hidden]")) return;
      const view = textarea.ownerDocument.defaultView!;
      const style = view.getComputedStyle(textarea);
      if (!mirror) {
        mirror = textarea.ownerDocument.createElement("textarea");
        mirror.rows = 1; mirror.tabIndex = -1; mirror.readOnly = true;
        mirror.setAttribute("aria-hidden", "true"); mirror.inert = true;
        mirror.style.cssText = "position:fixed;left:-10000px;top:0;visibility:hidden;pointer-events:none;box-sizing:border-box;height:0;min-height:0;max-height:none;overflow:hidden;resize:none;border:0";
        textarea.ownerDocument.body.append(mirror);
      }
      for (const property of metrics) mirror.style.setProperty(property, style.getPropertyValue(property));
      mirror.style.width = `${textarea.clientWidth}px`;
      mirror.wrap = textarea.wrap;
      mirror.value = textarea.value || " ";
      const padding = pixels(style.paddingTop) + pixels(style.paddingBottom);
      const border = pixels(style.borderTopWidth) + pixels(style.borderBottomWidth);
      const height = Math.ceil(mirror.scrollHeight + (style.boxSizing === "border-box" ? border : -padding));
      // Hidden/not-yet-laid-out controls can report zero; never collapse the editor.
      if (height <= 0) return;
      const nextHeight = `${height}px`;
      if (textarea.style.height !== nextHeight) textarea.style.height = nextHeight;
    },
    destroy() { mirror?.remove(); mirror = null; },
  };
}
