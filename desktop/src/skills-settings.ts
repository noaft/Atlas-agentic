import type { SkillsApi, LocalSkillPreview, SkillCatalog, SkillDocument, SkillInfo, SkillPreview, SkillScope, SkillWorkspaceContext } from "./skills-types";

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

/** Owns local drafts and async results independently of the model Settings form. */
export function createSkillsSettingsPanel(api: SkillsApi, initialContext: SkillWorkspaceContext, initialOwner: string): SkillsSettingsPanel {
  const element = node("section", undefined, "skills-settings"); element.setAttribute("aria-label", "Skills");
  let context = {...initialContext}; let owner = initialOwner; let active = false; let loaded = false;
  let generation = 0; let listRequest = 0; let previewRequest = 0;
  let scope: SkillScope = "personal";
  let catalog: SkillCatalog = {skills: [], issues: []}; let loading = false; let listError = "";
  let previews: SkillPreview[] = []; let previewBusy = false; let previewError = "";
  let updateTarget: SkillInfo | null = null;
  let installBusy = false; let installError = ""; let notice = "";
  let localBusy = false; let localError = ""; let localNotice = ""; let localRequest = 0;
  let localPreviews: LocalSkillPreview[] = [];
  const editors = new Map<string, HTMLTextAreaElement>();
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
  const localCard = node("div", undefined, "settings-card skills-local");
  const chooseFile = button("Choose skill file", () => void chooseLocal("file"));
  const chooseFolder = button("Choose skill folder", () => void chooseLocal("folder"));
  const localArea = node("div", undefined, "skills-local-preview");
  localCard.append(node("h2", "Import local"), node("p", "Choose a SKILL.md file or a skill folder with supporting files. Review before installing. Scripts are never run.", "settings-hint"), chooseFile, chooseFolder, localArea);
  const otherSources = node("details", undefined, "skills-other-sources");
  otherSources.append(node("summary", "Import from source"), node("p", "Use another installer for GitLab, Git URLs, or local paths. Atlas reads standard skill folders.", "settings-hint"), node("code", "npx skills add <source> --agent universal --copy"), node("p", "Add --global for Personal skills.", "settings-hint"), localCard, importCard);
  element.append(node("h2", "Installed skills"), scopeInput, hint, list, otherSources);

  const clearPreview = () => { previewRequest++; previews = []; previewBusy = false; previewError = ""; installBusy = false; installError = ""; notice = ""; updateTarget = null; };
  const reset = () => {
    generation++; loaded = false; loading = false; catalog = {skills: [], issues: []}; listError = "";
    busy.clear(); errors.clear(); documents.clear(); editors.clear(); clearPreview(); localRequest++; localBusy = false; localError = ""; localNotice = ""; localPreviews = [];
  };
  const drawPreview = () => {
    url.disabled = installBusy;
    scopeInput.disabled = installBusy || localBusy;
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
      if (skill.source) actions.append(button("Update", () => { otherSources.open = true; url.value = sourceUrl(skill); void previewImport(skill); }, !!operation || previewBusy || installBusy));
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
  const drawLocal = () => {
    chooseFile.disabled = localBusy; chooseFolder.disabled = localBusy;
    localArea.replaceChildren();
    if (localBusy) { const status = node("p", "Working…"); status.setAttribute("role", "status"); localArea.append(status); }
    if (localError) localArea.append(alert(localError));
    if (localNotice) { const status = node("p", localNotice); status.setAttribute("role", "status"); localArea.append(status); }
    for (const preview of localPreviews) {
      const card = node("article", undefined, "settings-card");
      card.append(node("h3", preview.title), node("p", preview.description), instructions(preview.content, preview.files), button("Install local skill", () => void installLocal(preview), localBusy));
      localArea.append(card);
    }
  };
  const draw = () => { drawList(); drawPreview(); drawLocal(); };
  const chooseLocal = async (kind: "file" | "folder") => {
    if (localBusy) return;
    const version = generation; const request = ++localRequest;
    localBusy = true; localError = ""; localNotice = ""; localPreviews = []; draw();
    try {
      const path = await api.chooseSkillImport(kind);
      if (!path || version !== generation || request !== localRequest) return;
      const result = await api.previewLocalSkills(path);
      if (version !== generation || request !== localRequest) return;
      localPreviews = result;
      if (!result.length) localError = "No skills found in this source.";
    } catch (error) { if (version === generation && request === localRequest) localError = errorText(error); }
    finally { if (version === generation && request === localRequest) { localBusy = false; draw(); } }
  };
  const installLocal = async (preview: LocalSkillPreview) => {
    if (localBusy) return;
    const version = generation; const request = localRequest; const captured = {...context}; const selectedScope = scope;
    localBusy = true; localError = ""; draw();
    try {
      const skill = await api.installLocalSkill(preview.path, preview.fingerprint, selectedScope, captured);
      if (version !== generation || request !== localRequest) return;
      localPreviews = []; localNotice = `${skill.title} installed.`; await refresh();
    } catch (error) { if (version === generation && request === localRequest) localError = errorText(error); }
    finally { if (version === generation && request === localRequest) { localBusy = false; draw(); } }
  };
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
      const installed = await api.installSkill(preview.source, selectedScope, replace, captured);
      if (version !== generation || request !== previewRequest) return;
      // Open drafts retain the fingerprint they were read with, including when
      // this installation replaces their package. A later save must detect that conflict.
      if (!editors.has(installed.id)) documents.delete(installed.id);
      notice = `${preview.title} ${replace ? "updated" : "installed"}.`; previews = []; await refresh();
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
