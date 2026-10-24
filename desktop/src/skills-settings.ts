import type { SkillsApi, SkillCatalog, SkillDocument, SkillInfo, SkillScope, SkillWorkspaceContext } from "./skills-types";

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
/** Owns local drafts and async results independently of the model Settings form. */
export function createSkillsSettingsPanel(api: SkillsApi, initialContext: SkillWorkspaceContext, initialOwner: string): SkillsSettingsPanel {
  const element = node("section", undefined, "skills-settings"); element.setAttribute("aria-label", "Skills");
  let context = {...initialContext}; let owner = initialOwner; let active = false; let loaded = false;
  let generation = 0; let listRequest = 0;
  let scope: SkillScope = "personal";
  let catalog: SkillCatalog = {skills: [], issues: []}; let loading = false; let listError = "";
  const editors = new Map<string, HTMLTextAreaElement>();
  const busy = new Map<string, string>(); const errors = new Map<string, string>(); const documents = new Map<string, SkillDocument>();

  const scopeInput = node("select"); scopeInput.setAttribute("aria-label", "Skill scope");
  for (const [value, text] of [["personal", "Personal"], ["project", "Project"]]) {
    const option = node("option", text); option.value = value; scopeInput.append(option);
  }
  const hint = node("p", undefined, "settings-hint");
  const list = node("div", undefined, "skills-installed");
  element.append(node("h2", "Installed skills"), scopeInput, hint, list);

  const reset = () => {
    generation++; loaded = false; loading = false; catalog = {skills: [], issues: []}; listError = "";
    busy.clear(); errors.clear(); documents.clear(); editors.clear();
  };
  const drawList = () => {
    const focused = document.activeElement;
    const editing = focused instanceof HTMLTextAreaElement && list.contains(focused) ? focused : null;
    const selection = editing ? [editing.selectionStart, editing.selectionEnd] : null;
    const scrollTop = editing?.scrollTop;
    hint.textContent = scope === "personal" ? "Personal skills use the shared skills folder and apply across all projects." : `Project skills live in .agents/skills and apply to the current starting folder: ${context.workspacePath ?? "Atlas default folder"}.`;
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
      actions.append(button("Edit skill", () => void read(skill, true), !!operation));
      card.append(actions);
      if (errors.has(skill.id)) card.append(alert(errors.get(skill.id)!));
      const doc = documents.get(skill.id);
      const editor = editors.get(skill.id);
      if (doc && editor) {
        editor.disabled = !!operation;
        card.append(editor, node("p", `Files: ${doc.files.join(", ")}`, "settings-hint"), button(operation === "save" ? "Saving skill…" : "Save skill", () => void save(skill), !!operation), button("Reload skill from disk", () => void read(skill, true, true), !!operation));
      } else if (doc) card.append(instructions(doc.content, doc.files));
      list.append(card);
    }
    if (editing && list.contains(editing) && !editing.disabled) {
      editing.focus({preventScroll: true});
      editing.setSelectionRange(selection![0], selection![1]); editing.scrollTop = scrollTop!;
    }
  };
  const draw = () => drawList();
  const save = async (skill: SkillInfo) => {
    const doc = documents.get(skill.id); const editor = editors.get(skill.id);
    if (!doc || !editor || busy.has(skill.id)) return;
    const version = generation; const captured = {...context};
    busy.set(skill.id, "save"); errors.delete(skill.id); drawList();
    try {
      const updated = await api.updateSkillDocument(skill.id, editor.value, doc.fingerprint, captured);
      if (version !== generation) return;
      catalog.skills = catalog.skills.map(item => item.id === skill.id ? updated : item);
      const fresh = await api.readSkill(skill.id, captured);
      if (version !== generation) return;
      documents.set(skill.id, fresh); editor.value = fresh.content;
    } catch (error) { if (version === generation) errors.set(skill.id, errorText(error)); }
    finally { if (version === generation) { busy.delete(skill.id); drawList(); } }
  };
  const refresh = async () => {
    const version = generation; const request = ++listRequest; const captured = {...context};
    loaded = true; loading = true; listError = ""; drawList();
    try { const result = await api.listSkills(captured); if (version === generation && request === listRequest) catalog = result; }
    catch (error) { if (version === generation && request === listRequest) listError = errorText(error); }
    finally { if (version === generation && request === listRequest) { loading = false; drawList(); } }
  };
  const read = async (skill: SkillInfo, edit = false, reload = false) => {
    if (busy.has(skill.id) || (editors.has(skill.id) && !reload)) return;
    const version = generation; const captured = {...context}; busy.set(skill.id, "read"); errors.delete(skill.id); drawList();
    try {
      const doc = await api.readSkill(skill.id, captured);
      if (version === generation) {
        documents.set(skill.id, doc);
        if (edit && (!editors.has(skill.id) || reload)) {
          const editor = node("textarea", undefined, "skill-document-editor"); editor.rows = 12;
          editor.setAttribute("aria-label", `Skill document ${skill.name}`); editor.value = doc.content; editors.set(skill.id, editor);
        }
      }
    }
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
  scopeInput.addEventListener("change", () => {
    scope = scopeInput.value as SkillScope; reset(); draw(); if (active) void refresh();
  });
  draw();
  return {element, updateContext(next, nextOwner, enabled = true) {
    const changed = owner !== nextOwner || context.conversationId !== next.conversationId || context.workspacePath !== next.workspacePath;
    if (changed) { context = {...next}; owner = nextOwner; reset(); draw(); }
    active = enabled; if (active && !loaded) void refresh();
  }};
}
