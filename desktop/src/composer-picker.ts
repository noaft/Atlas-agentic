let nextPickerId = 0;

/** Custom composer menu backed by a hidden select for value/change semantics. */
export function createComposerPicker(select: HTMLSelectElement) {
  const element = document.createElement("div");
  element.className = "composer-picker";
  select.hidden = true;
  select.tabIndex = -1;
  const button = document.createElement("button");
  button.type = "button";
  button.className = "composer-picker-trigger";
  button.disabled = select.disabled;
  button.dataset.pickerLabel = select.getAttribute("aria-label") ?? "Choose option";
  button.setAttribute("aria-haspopup", "listbox");
  button.setAttribute("aria-expanded", "false");
  const list = document.createElement("div");
  list.className = "composer-picker-menu";
  list.id = `composer-picker-${++nextPickerId}`;
  list.setAttribute("role", "listbox");
  list.setAttribute("aria-label", select.getAttribute("aria-label") ?? "Options");
  list.tabIndex = -1;
  list.hidden = true;
  button.setAttribute("aria-controls", list.id);
  const options = Array.from(select.options);
  let active = Math.max(0, select.selectedIndex);
  let open = false;
  const rows = options.map((option, index) => {
    const row = document.createElement("div");
    row.className = "composer-picker-option";
    row.id = `${list.id}-${index}`;
    row.setAttribute("role", "option");
    const name = document.createElement("span");
    name.textContent = option.textContent;
    const check = document.createElement("span");
    check.className = "composer-picker-check";
    check.setAttribute("aria-hidden", "true");
    check.textContent = "✓";
    row.append(name, check);
    row.addEventListener("click", () => choose(index));
    list.append(row);
    return row;
  });
  function refresh() {
    button.textContent = options[select.selectedIndex]?.textContent ?? "Choose";
    button.setAttribute("aria-label", `${button.dataset.pickerLabel}: ${button.textContent}`);
    rows.forEach((row, index) => {
      row.setAttribute("aria-selected", String(index === select.selectedIndex));
      row.classList.toggle("is-active", index === active);
    });
    if (rows[active]) list.setAttribute("aria-activedescendant", rows[active].id);
  }
  function outside(event: Event) {
    if (event.target instanceof Node && !element.contains(event.target)) close(false);
  }
  function close(restoreFocus: boolean) {
    open = false;
    list.hidden = true;
    button.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", outside, true);
    if (restoreFocus) button.focus();
  }
  function show() {
    if (button.disabled || !options.length) return;
    open = true;
    active = Math.max(0, select.selectedIndex);
    refresh();
    list.hidden = false;
    list.style.left = "0px";
    list.style.left = `${Math.min(0, window.innerWidth - 12 - list.getBoundingClientRect().right)}px`;
    button.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", outside, true);
    list.focus();
    rows[active]?.scrollIntoView?.({ block: "nearest" });
  }
  function choose(index: number) {
    if (options[index].disabled) return;
    const changed = select.selectedIndex !== index;
    select.selectedIndex = index;
    refresh();
    close(true);
    if (changed) select.dispatchEvent(new Event("change", { bubbles: true }));
  }
  button.addEventListener("click", () => open ? close(false) : show());
  button.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      show();
    }
  });
  list.addEventListener("keydown", (event) => {
    if (event.key === "Tab") { close(true); return; }
    if (event.key === "Escape") { event.preventDefault(); close(true); return; }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      choose(active);
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      active = event.key === "Home" ? 0 : event.key === "End" ? options.length - 1
        : (active + (event.key === "ArrowDown" ? 1 : -1) + options.length) % options.length;
      refresh();
      rows[active]?.scrollIntoView?.({ block: "nearest" });
    }
  });
  element.addEventListener("focusout", (event) => {
    if (!(event.relatedTarget instanceof Node) || !element.contains(event.relatedTarget)) close(false);
  });
  refresh();
  element.append(select, button, list);
  return { element, destroy: () => close(false) };
}
