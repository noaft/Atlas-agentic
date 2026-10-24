import type { SkillsApi, SkillCatalog, SkillDocument, SkillInfo, SkillPreview, SkillScope, SkillWorkspaceContext } from "./skills-types";

export interface SkillsSettingsPanel {
  element: HTMLElement;
  updateContext(context: SkillWorkspaceContext, owner: string, active?: boolean): void;
}

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string, className?: string): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  if (text) element.textContent = text;
  if (className) element.className = className;
  return element;
}
function button(text: string, action: () => void, disabled = false): HTMLButtonElement {
  const element = node("button", text, "secondary-button"); element.type = "button"; element.disabled = disabled;
  element.addEventListener("click", action); return element;
}
function alert(text: string): HTMLElement {
  const element = node("p", text, "settings-error"); element.setAttribute("role", "alert"); return element;
}
function instructions(content: string, files: string[]): HTMLElement {
  const section = node("div", undefined, "skill-instructions");
  section.append(node("pre", content), node("p", `Files: ${files.join(", ")}`, "settings-hint")); return section;
}
function sourceText(skill: Pick<SkillInfo, "source">): string {
  const source = skill.source;
  return source ? `${source.repository} · ${source.path || "/"} · ${source.reference} · ${source.revision}` : "Local skill";
}
function sourceUrl(skill: SkillInfo): string {
  const source = skill.source!;
  return `https://github.com/${source.repository}/tree/${source.reference}${source.path ? "/" + source.path : ""}`;
}

/** Retains the URL input and owns async results independently of the model Settings form. */
export function createSkillsSettingsPanel(api: SkillsApi, initialContext: SkillWorkspaceContext, initialOwner: string): SkillsSettingsPanel {
  const element = node("section", undefined, "skills-settings"); element.setAttribute("aria-label", "Skills");
  let context = {...initialContext}; let owner = initialOwner; let active = false; let loaded = false;
  let generation = 0; let listRequest = 0; let previewRequest = 0;
  let scope: SkillScope = "personal";
  let catalog: SkillCatalog = {skills: [], issues: []}; let loading = false; let listError = "";
  let previews: SkillPreview[] = []; let previewBusy = false; let previewError = "";
  let updateTarget: SkillInfo | null = null;
  let installBusy = false; let installError = ""; let notice = "";
  const busy = new Map<string, string>(); const errors = new Map<string, string>(); const documents = new Map<string, SkillDocument>();

  const scopeInput = node("select"); scopeInput.setAttribute("aria-label", "Skill scope");
  for (const [value, text] of [["personal", "Personal"], ["project", "Project"]]) {
    const option = node("option", text); option.value = value; scopeInput.append(option);
  }
  const hint = node("p", undefined, "settings-hint");
  const list = node("div", undefined, "skills-installed");
  const importCard = node("div", undefined, "settings-card");
  const label = node("label", undefined, "settings-field");
  const url = node("input"); url.type = "text"; url.setAttribute("aria-label", "GitHub skill URL"); url.placeholder = "https://github.com/owner/repository/tree/main/skill";
  label.append(node("span", "GitHub skill URL", "settings-label"), url);
  const previewButton = button("Preview", () => void previewImport());
  const previewArea = node("div", undefined, "skills-preview");
  importCard.append(node("h2", "Import from GitHub"), node("p", "Review the instructions and pinned source before installing. Changes apply immediately; Save and Discard do not affect skills.", "settings-hint"), label, previewButton, previewArea);
  element.append(scopeInput, hint, list, importCard);

  const clearPreview = () => { previewRequest++; previews = []; previewBusy = false; previewError = ""; installBusy = false; installError = ""; notice = ""; updateTarget = null; };
  const reset = () => {
    generation++; loaded = false; loading = false; catalog = {skills: [], issues: []}; listError = "";
    busy.clear(); errors.clear(); documents.clear(); clearPreview();
  };
  const drawPreview = () => {
    url.disabled = installBusy;
    scopeInput.disabled = installBusy;
    previewButton.disabled = previewBusy || installBusy || !url.value.trim();
    previewButton.textContent = previewBusy ? "Loading preview…" : "Preview";
    previewArea.replaceChildren();
    if (previewError) previewArea.append(alert(previewError));
    if (installError) previewArea.append(alert(installError));
    if (notice) { const message = node("p", notice); message.setAttribute("role", "status"); previewArea.append(message); }
    for (const preview of previews) {
      const card = node("article", undefined, "settings-card skill-preview");
      card.append(node("h3", preview.title), node("p", preview.description), node("p", sourceText(preview), "settings-hint"), instructions(preview.content, preview.files));
      card.append(button(installBusy ? "Installing…" : updateTarget ? "Update installed skill" : `Install in ${scope === "personal" ? "Personal" : "Project"}`, () => void install(preview), installBusy || previewBusy));
      previewArea.append(card);
    }
  };
  const drawList = () => {
    hint.textContent = scope === "personal" ? "Personal skills are shared across all projects." : `Project skills apply to the current starting folder: ${context.workspacePath ?? "Atlas default folder"}.`;
    list.replaceChildren(button(loading ? "Loading skills…" : "Refresh skills", () => void refresh(), loading));
    if (listError) list.append(alert(listError));
    for (const issue of catalog.issues) list.append(alert(issue));
    const skills = catalog.skills.filter(skill => skill.scope === scope);
    if (!loading && !listError && !skills.length) list.append(node("p", `No ${scope} skills installed.`));
    for (const skill of skills) {
      const card = node("article", undefined, "settings-card skill-installed");
      const operation = busy.get(skill.id);
      card.append(node("h2", skill.title), node("p", skill.description), node("p", sourceText(skill), "settings-hint"), node("span", skill.enabled ? "Enabled" : "Disabled", "settings-hint"));
      const actions = node("div", undefined, "skill-actions");
      actions.append(button(operation === "read" ? "Loading instructions…" : "View instructions", () => void read(skill), !!operation), button(operation === "toggle" ? "Saving…" : skill.enabled ? "Disable" : "Enable", () => void toggle(skill), !!operation));
      if (skill.source) actions.append(button("Update", () => { url.value = sourceUrl(skill); void previewImport(skill); }, !!operation || previewBusy || installBusy));
      card.append(actions);
      if (errors.has(skill.id)) card.append(alert(errors.get(skill.id)!));
      const doc = documents.get(skill.id); if (doc) card.append(instructions(doc.content, doc.files));
      list.append(card);
    }
  };
  const draw = () => { drawList(); drawPreview(); };
  const refresh = async () => {
    const version = generation; const request = ++listRequest; const captured = {...context};
    loaded = true; loading = true; listError = ""; drawList();
    try { const result = await api.listSkills(captured); if (version === generation && request === listRequest) catalog = result; }
    catch (error) { if (version === generation && request === listRequest) listError = errorText(error); }
    finally { if (version === generation && request === listRequest) { loading = false; drawList(); } }
  };
  const read = async (skill: SkillInfo) => {
    if (busy.has(skill.id)) return;
    const version = generation; const captured = {...context}; busy.set(skill.id, "read"); errors.delete(skill.id); drawList();
    try { const doc = await api.readSkill(skill.id, captured); if (version === generation) documents.set(skill.id, doc); }
    catch (error) { if (version === generation) errors.set(skill.id, errorText(error)); }
    finally { if (version === generation) { busy.delete(skill.id); drawList(); } }
  };
  const toggle = async (skill: SkillInfo) => {
    if (busy.has(skill.id)) return;
    const version = generation; const captured = {...context}; busy.set(skill.id, "toggle"); errors.delete(skill.id); drawList();
    try {
      const updated = await api.setSkillEnabled(skill.id, !skill.enabled, captured);
      if (version === generation) catalog.skills = catalog.skills.map(item => item.id === skill.id ? updated : item);
    } catch (error) { if (version === generation) errors.set(skill.id, errorText(error)); }
    finally { if (version === generation) { busy.delete(skill.id); drawList(); } }
  };
  const previewImport = async (target: SkillInfo | null = null) => {
    if (previewBusy || installBusy || !url.value.trim()) return;
    clearPreview(); updateTarget = target; previewBusy = true;
    const request = ++previewRequest; const version = generation; const requestedUrl = url.value.trim(); draw();
    try {
      const result = await api.previewSkillImport(requestedUrl);
      if (version !== generation || request !== previewRequest) return;
      previews = target ? result.filter(item => item.name === target.name && item.source.repository === target.source?.repository && item.source.path === target.source.path) : result;
      if (!previews.length) previewError = target ? "The installed skill was not found in this source. Nothing was updated." : "No skills found in this source.";
    } catch (error) { if (version === generation && request === previewRequest) previewError = errorText(error); }
    finally { if (version === generation && request === previewRequest) { previewBusy = false; draw(); } }
  };
  const install = async (preview: SkillPreview) => {
    if (installBusy || previewBusy) return;
    const version = generation; const request = previewRequest; const captured = {...context}; const selectedScope = scope; const replace = !!updateTarget;
    installBusy = true; installError = ""; notice = ""; draw();
    try {
      await api.installSkill(preview.source, selectedScope, replace, captured);
      if (version !== generation || request !== previewRequest) return;
      notice = `${preview.title} ${replace ? "updated" : "installed"}.`; previews = []; documents.clear(); await refresh();
    } catch (error) { if (version === generation && request === previewRequest) installError = errorText(error); }
    finally { if (version === generation && request === previewRequest) { installBusy = false; draw(); } }
  };
  scopeInput.addEventListener("change", () => {
    scope = scopeInput.value as SkillScope; reset(); draw(); if (active) void refresh();
  });
  url.addEventListener("input", () => { clearPreview(); drawPreview(); });
  url.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); void previewImport(); } });
  draw();
  return {element, updateContext(next, nextOwner, enabled = true) {
    const changed = owner !== nextOwner || context.conversationId !== next.conversationId || context.workspacePath !== next.workspacePath;
    if (changed) { context = {...next}; owner = nextOwner; reset(); draw(); }
    active = enabled; if (active && !loaded) void refresh();
  }};
}
