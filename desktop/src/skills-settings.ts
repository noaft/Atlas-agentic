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
function instructions(content: string, files: string[], collapsible = false): HTMLElement {
  const section = node(collapsible ? "details" : "div", undefined, "skill-instructions");
  if (collapsible) section.append(node("summary", "Instructions"));
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
  let batchBusy = false;
  const previewErrors = new Map<string, string>();
  let searchResults: SkillInfo[] | null = null; let searchBusy = false; let searchError = ""; let searchRequest = 0;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let installingSource: string | null = null;
  let installBusy = false; let installError = ""; let notice = "";
  let localBusy = false; let localError = ""; let localNotice = ""; let localRequest = 0;
  let localPreviews: LocalSkillPreview[] = [];
  let localSource: "file" | "folder" | null = null;
  let localInstalling: string | null = null;
  const openDocuments = new Set<string>();
  const openDetails = new Set<string>();
  const editors = new Map<string, HTMLTextAreaElement>();
  const busy = new Map<string, string>(); const errors = new Map<string, string>(); const documents = new Map<string, SkillDocument>();

  const scopeInput = node("select"); scopeInput.setAttribute("aria-label", "Skill scope");
  for (const [value, text] of [["personal", "All projects"], ["project", "This project"]]) {
    const option = node("option", text); option.value = value; scopeInput.append(option);
  }
  const hint = node("p", undefined, "settings-hint");
  const list = node("div", undefined, "skills-installed");
  const searchInput = node("input"); searchInput.type = "search"; searchInput.maxLength = 512;
  searchInput.placeholder = "Search installed skills"; searchInput.setAttribute("aria-label", "Search skills");
  const importCard = node("div", undefined, "settings-card skills-github-source");
  const label = node("label", undefined, "settings-field");
  const url = node("input"); url.type = "text"; url.setAttribute("aria-label", "GitHub skill URL"); url.placeholder = "https://github.com/owner/repository/tree/main/skill";
  label.append(node("span", "GitHub skill URL", "settings-label"), url);
  const previewButton = button("Preview", () => void previewImport());
  const previewArea = node("div", undefined, "skills-preview");
  importCard.append(node("h2", "Import from GitHub"), node("p", "Paste a GitHub link to see which skills you can add.", "settings-hint"), label, previewButton, previewArea);
  const localCard = node("div", undefined, "settings-card skills-local skills-local-source");
  const chooseFile = button("Choose skill file", () => void chooseLocal("file"));
  const chooseFolder = button("Choose skill folder", () => void chooseLocal("folder"));
  const localArea = node("div", undefined, "skills-local-preview");
  localCard.append(node("h2", "Import local"), node("p", "Choose a skill from your computer, then review it before adding.", "settings-hint"), chooseFile, chooseFolder, localArea);
  const otherSources = node("details", undefined, "skills-other-sources");
  otherSources.append(node("summary", "Other installation methods"), node("p", "Atlas also reads standard skill folders installed by other tools.", "settings-hint"), node("code", "npx skills add <source> --agent universal --copy"), node("p", "Add --global for Personal skills.", "settings-hint"));
  const sourceInput = node("select"); sourceInput.setAttribute("aria-label", "Skill source");
  for (const [value, text] of [["github", "GitHub link"], ["folder", "Folder on my computer"], ["file", "SKILL.md file"]]) {
    const option = node("option", text); option.value = value; sourceInput.append(option);
  }
  let sourceMode = "github";
  const sourceField = node("label", undefined, "settings-field"); sourceField.append(node("span", "Add from", "settings-label"), sourceInput);
  const add = node("section", undefined, "skills-add"); add.setAttribute("aria-label", "Add skills"); add.hidden = true;
  add.append(node("h2", "Add skills"), sourceField, importCard, localCard);
  const addButton = button("Add skill", () => { setAddOpen(add.hidden === true); if (!add.hidden) sourceInput.focus(); });
  addButton.className = "primary-button"; addButton.dataset.addSkill = ""; addButton.setAttribute("aria-expanded", "false");
  const setAddOpen = (open: boolean) => {
    add.hidden = !open; addButton.textContent = open ? "Close" : "Add skill"; addButton.setAttribute("aria-expanded", String(open));
  };
  const drawSource = () => {
    importCard.hidden = sourceMode !== "github"; localCard.hidden = sourceMode === "github";
    chooseFile.hidden = sourceMode !== "file"; chooseFolder.hidden = sourceMode !== "folder";
    localArea.hidden = localSource !== null && localSource !== sourceMode;
    sourceInput.disabled = previewBusy || installBusy || localBusy;
  };
  const toolbar = node("div", undefined, "skills-toolbar");
  const scopeField = node("label", undefined, "skills-scope settings-field");
  scopeField.append(node("span", "Skills for", "settings-label"), scopeInput);
  toolbar.append(scopeField, addButton);
  const library = node("section", undefined, "skills-library"); library.setAttribute("aria-label", "Installed skills");
  const counts = node("p", undefined, "settings-hint"); counts.setAttribute("role", "status");
  const libraryHeading = node("div", undefined, "skills-library-heading"); libraryHeading.append(node("h2", "Installed skills"), counts);
  const refreshButton = button("Refresh skills", () => void refresh());
  const options = node("details", undefined, "skills-library-options"); options.append(node("summary", "Library options"), refreshButton, otherSources);
  const issues = node("details", undefined, "skills-issues"); const issueSummary = node("summary"); const issueBody = node("div"); issues.append(issueSummary, issueBody);
  library.append(libraryHeading, searchInput, issues, list, options, node("p", "Enabled skills are ready for your next request. Changes save automatically.", "settings-hint"));
  element.append(toolbar, hint, add, library);
  sourceInput.addEventListener("change", () => {
    if (previewBusy || installBusy || localBusy) { sourceInput.value = sourceMode; return; }
    sourceMode = sourceInput.value; drawSource();
  });

  const clearPreview = () => { previewRequest++; previews = []; previewBusy = false; previewError = ""; installBusy = false; installingSource = null; installError = ""; notice = ""; updateTarget = null; batchBusy = false; previewErrors.clear(); };
  const reset = () => {
    searchRequest++; clearTimeout(searchTimer); searchResults = null; searchBusy = false; searchError = "";
    generation++; loaded = false; loading = false; catalog = {skills: [], issues: []}; listError = "";
    busy.clear(); errors.clear(); documents.clear(); editors.clear(); openDocuments.clear(); openDetails.clear(); clearPreview(); localRequest++; localBusy = false; localError = ""; localNotice = ""; localPreviews = []; localInstalling = null; localSource = null;
  };
  const drawPreview = () => {
    url.disabled = installBusy;
    scopeInput.disabled = installBusy || localBusy;
    previewButton.disabled = previewBusy || installBusy || localBusy || !url.value.trim();
    previewButton.textContent = previewBusy ? "Loading preview…" : "Preview";
    previewArea.replaceChildren();
    if (previewError) previewArea.append(alert(previewError));
    if (installError) previewArea.append(alert(installError));
    if (notice) { const message = node("p", notice); message.setAttribute("role", "status"); previewArea.append(message); }
    if (previews.length > 1 && !updateTarget) previewArea.append(button(batchBusy ? "Installing all…" : "Add all", () => void installAll(), installBusy || previewBusy || localBusy));
    for (const preview of previews) {
      const card = node("article", undefined, "settings-card skill-preview");
      const provenance = node("details", undefined, "skill-source-details");
      provenance.append(node("summary", "Source details"), node("p", sourceText(preview), "settings-hint"));
      card.append(node("h3", preview.title), node("p", preview.description), instructions(preview.content, preview.files, true), provenance);
      card.append(button(installingSource === JSON.stringify(preview.source) ? "Installing…" : updateTarget ? "Update installed skill" : `Add to ${scope === "personal" ? "all projects" : "this project"}`, () => void install(preview), installBusy || previewBusy || localBusy));
      const error = previewErrors.get(JSON.stringify(preview.source)); if (error) card.append(alert(error));
      previewArea.append(card);
    }
  };
  const drawList = () => {
    const focused = document.activeElement;
    const editing = focused instanceof HTMLTextAreaElement && list.contains(focused) ? focused : null;
    const selection = editing ? [editing.selectionStart, editing.selectionEnd] : null;
    const scrollTop = editing?.scrollTop;
    const focusedSwitchId = focused instanceof HTMLInputElement && list.contains(focused) && focused.getAttribute("role") === "switch" ? focused.dataset.skillId : undefined;
    const switchControls = new Map<string, HTMLInputElement>();
    hint.textContent = scope === "personal" ? "Available across all your projects." : `Only for this project: ${context.workspacePath ?? "Atlas default folder"}.`;
    const scoped = catalog.skills.filter(skill => skill.scope === scope);
    const enabled = scoped.filter(skill => skill.enabled).length;
    counts.textContent = `${scoped.length} installed · ${enabled} enabled`;
    refreshButton.textContent = loading ? "Loading skills…" : "Refresh skills"; refreshButton.disabled = loading;
    issues.hidden = !catalog.issues.length;
    issueSummary.textContent = `${catalog.issues.length} ${catalog.issues.length === 1 ? "skill needs" : "skills need"} attention`;
    issueBody.replaceChildren(node("p", "These skills could not be loaded. Your other skills are still available.", "settings-hint"));
    if (catalog.issues.some(issue => issue.includes("symlink"))) issueBody.append(node("p", "Linked folders are not supported. Import the actual skill folder to create a local copy.", "settings-hint"));
    for (const issue of catalog.issues) issueBody.append(alert(issue));
    list.replaceChildren();
    if (!loading && !listError && scoped.length && !enabled) list.append(node("p", "Enable a skill to make it available to Atlas on the next request.", "settings-hint"));
    if (listError) list.append(alert(listError), button("Retry", () => void refresh()));
    if (loading && !scoped.length) { const status = node("p", "Loading skills…", "settings-hint"); status.setAttribute("role", "status"); list.append(status); }
    if (searchBusy) { const status = node("p", "Searching skills…"); status.setAttribute("role", "status"); list.append(status); }
    if (searchError) list.append(alert(searchError));
    const skills = (searchResults ?? scoped).map(skill => catalog.skills.find(item => item.id === skill.id) ?? skill);
    if (!loading && !searchBusy && !listError && !skills.length) list.append(node("p", searchInput.value.trim() ? "No matching skills." : "No skills yet. Choose Add skill to get started.", "settings-hint"));
    for (const skill of skills) {
      const card = node("article", undefined, "settings-card skill-installed");
      const operation = busy.get(skill.id);
      const header = node("div", undefined, "skill-card-heading");
      const toggleLabel = node("label", undefined, "skill-toggle");
      const toggleInput = node("input"); toggleInput.type = "checkbox"; toggleInput.setAttribute("role", "switch"); toggleInput.setAttribute("aria-label", `Use ${skill.title}`);
      toggleInput.dataset.skillId = skill.id;
      toggleInput.checked = operation === "toggle" ? !skill.enabled : skill.enabled;
      // Keep the switch focusable while saving; suppress repeated activation.
      toggleInput.setAttribute("aria-disabled", String(!!operation));
      toggleInput.addEventListener("click", event => { if (busy.has(skill.id)) event.preventDefault(); });
      toggleInput.addEventListener("keydown", event => {
        if (event.key === "Enter") { event.preventDefault(); if (!event.repeat) toggleInput.click(); }
      });
      toggleInput.addEventListener("change", () => { if (!busy.has(skill.id)) void toggle(skill); });
      switchControls.set(skill.id, toggleInput);
      toggleLabel.append(toggleInput);
      header.append(node("h3", skill.title), toggleLabel);
      card.append(header, node("p", skill.description, "skill-description"));
      if (operation === "toggle") { const status = node("span", "Saving…", "settings-hint"); status.setAttribute("role", "status"); card.append(status); }
      if (errors.has(skill.id)) card.append(alert(errors.get(skill.id)!));
      const details = node("details", undefined, "skill-details"); details.open = openDetails.has(skill.id);
      const summary = node("summary", "Details");
      summary.addEventListener("click", event => {
        event.preventDefault(); details.open = !details.open;
        if (details.open) openDetails.add(skill.id); else openDetails.delete(skill.id);
      });
      details.append(summary, node("p", sourceText(skill), "settings-hint"));
      const actions = node("div", undefined, "skill-actions");
      actions.append(button(operation === "read" ? "Loading instructions…" : openDocuments.has(skill.id) && !editors.has(skill.id) ? "Close instructions" : "View instructions", () => {
        if (openDocuments.has(skill.id)) openDocuments.delete(skill.id);
        else if (documents.has(skill.id)) openDocuments.add(skill.id);
        else { void read(skill); return; }
        drawList();
      }, !!operation), button("Edit skill", () => void read(skill, true), !!operation));
      if (skill.source) actions.append(button("Update", () => {
        setAddOpen(true); sourceMode = "github"; sourceInput.value = sourceMode; drawSource();
        url.value = sourceUrl(skill); void previewImport(skill); add.scrollIntoView?.({block: "nearest"});
      }, !!operation || previewBusy || installBusy || localBusy));
      details.append(actions);
      const doc = documents.get(skill.id);
      const editor = editors.get(skill.id);
      if (doc && editor) {
        editor.disabled = !!operation;
        details.append(editor, node("p", `Files: ${doc.files.join(", ")}`, "settings-hint"), button(operation === "save" ? "Saving skill…" : "Save skill", () => void save(skill), !!operation), button("Reload skill from disk", () => void read(skill, true, true), !!operation));
      } else if (doc && openDocuments.has(skill.id)) details.append(instructions(doc.content, doc.files));
      card.append(details); list.append(card);
    }
    if (focusedSwitchId) switchControls.get(focusedSwitchId)?.focus({preventScroll: true});
    if (editing && list.contains(editing) && !editing.disabled) {
      editing.focus({preventScroll: true});
      editing.setSelectionRange(selection![0], selection![1]); editing.scrollTop = scrollTop!;
    }
  };
  const drawLocal = () => {
    chooseFile.disabled = localBusy || installBusy; chooseFolder.disabled = localBusy || installBusy;
    localArea.replaceChildren();
    if (localBusy) { const status = node("p", "Working…"); status.setAttribute("role", "status"); localArea.append(status); }
    if (localError) localArea.append(alert(localError));
    if (localNotice) { const status = node("p", localNotice); status.setAttribute("role", "status"); localArea.append(status); }
    for (const preview of localPreviews) {
      const card = node("article", undefined, "settings-card");
      card.append(node("h3", preview.title), node("p", preview.description), instructions(preview.content, preview.files, true), button(localInstalling === preview.path ? "Installing…" : "Add this skill", () => void installLocal(preview), localBusy));
      localArea.append(card);
    }
  };
  const acceptInstalled = (skill: SkillInfo) => {
    catalog.skills = [...catalog.skills.filter(item => item.id !== skill.id), skill];
    if (!editors.has(skill.id)) { documents.delete(skill.id); openDocuments.delete(skill.id); }
  };
  const draw = () => { drawList(); drawPreview(); drawLocal(); drawSource(); };
  const chooseLocal = async (kind: "file" | "folder") => {
    if (localBusy || installBusy) return;
    const version = generation; const request = ++localRequest;
    localBusy = true; localSource = kind; localError = ""; localNotice = ""; localPreviews = []; draw();
    try {
      const path = await api.chooseSkillImport(kind);
      if (!path || version !== generation || request !== localRequest) return;
      const result = await api.previewLocalSkills(path);
      if (version !== generation || request !== localRequest) return;
      localPreviews = result;
      if (!result.length) localError = "No skills found in this source.";
    } catch (error) { if (version === generation && request === localRequest) localError = errorText(error); }
    finally { if (version === generation && request === localRequest) { localBusy = false; localInstalling = null; draw(); } }
  };
  const installLocal = async (preview: LocalSkillPreview) => {
    if (localBusy || installBusy) return;
    const version = generation; const request = localRequest; const captured = {...context}; const selectedScope = scope;
    localBusy = true; localInstalling = preview.path; localError = ""; draw();
    try {
      const skill = await api.installLocalSkill(preview.path, preview.fingerprint, selectedScope, captured);
      if (version !== generation || request !== localRequest) return;
      acceptInstalled(skill);
      localPreviews = localPreviews.filter(item => item.path !== preview.path); localNotice = `${skill.title} installed.`; await refresh();
    } catch (error) { if (version === generation && request === localRequest) localError = errorText(error); }
    finally { if (version === generation && request === localRequest) { localBusy = false; localInstalling = null; draw(); } }
  };
  const search = async () => {
    const query = searchInput.value.trim(); const request = ++searchRequest; const version = generation;
    searchError = "";
    if (!query) { searchResults = null; searchBusy = false; drawList(); return; }
    const captured = {...context}; const selectedScope = scope;
    searchBusy = true; drawList();
    try {
      const result = await api.searchSkills(query, selectedScope, captured);
      if (version === generation && request === searchRequest) searchResults = result.map(item => item.skill);
    } catch (error) { if (version === generation && request === searchRequest) searchError = errorText(error); }
    finally { if (version === generation && request === searchRequest) { searchBusy = false; drawList(); } }
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
    finally { if (version === generation && request === listRequest) { loading = false; drawList(); if (searchInput.value.trim()) void search(); } }
  };
  const read = async (skill: SkillInfo, edit = false, reload = false) => {
    if (busy.has(skill.id) || (editors.has(skill.id) && !reload)) return;
    const version = generation; const captured = {...context}; busy.set(skill.id, "read"); errors.delete(skill.id); drawList();
    try {
      const doc = await api.readSkill(skill.id, captured);
      if (version === generation) {
        documents.set(skill.id, doc); openDocuments.add(skill.id);
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
    if (previewBusy || installBusy || localBusy || !url.value.trim()) return;
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
    if (installBusy || previewBusy || localBusy) return;
    const version = generation; const request = previewRequest; const captured = {...context}; const selectedScope = scope; const replace = !!updateTarget;
    installBusy = true; installingSource = JSON.stringify(preview.source); installError = ""; notice = ""; draw();
    try {
      const installed = await api.installSkill(preview.source, selectedScope, replace, captured);
      if (version !== generation || request !== previewRequest) return;
      // Open drafts retain the fingerprint they were read with, including when
      // this installation replaces their package. A later save must detect that conflict.
      acceptInstalled(installed);
      notice = `${preview.title} ${replace ? "updated" : "installed"}.`; previews = previews.filter(item => JSON.stringify(item.source) !== JSON.stringify(preview.source)); await refresh();
    } catch (error) { if (version === generation && request === previewRequest) installError = errorText(error); }
    finally { if (version === generation && request === previewRequest) { installBusy = false; installingSource = null; draw(); } }
  };
  const installAll = async () => {
    if (installBusy || previewBusy || localBusy || updateTarget || !previews.length) return;
    const version = generation; const request = previewRequest; const captured = {...context}; const selectedScope = scope;
    const sources = previews.map(item => item.source);
    installBusy = true; batchBusy = true; installError = ""; notice = ""; previewErrors.clear(); draw();
    try {
      const results = await api.installSkills(sources, selectedScope, captured);
      if (version !== generation || request !== previewRequest) return;
      const installed = new Set<string>();
      for (const result of results) {
        const key = JSON.stringify(result.source);
        if (result.skill && !result.error) { acceptInstalled(result.skill); installed.add(key); }
        else previewErrors.set(key, result.error ?? "Installation returned no verified skill.");
      }
      previews = previews.filter(item => !installed.has(JSON.stringify(item.source)));
      notice = `${installed.size} installed; ${sources.length - installed.size} failed.`;
      await refresh();
    } catch (error) { if (version === generation && request === previewRequest) installError = errorText(error); }
    finally { if (version === generation && request === previewRequest) { installBusy = false; batchBusy = false; draw(); } }
  };
  searchInput.addEventListener("input", () => {
    clearTimeout(searchTimer); searchRequest++; searchResults = null; searchError = "";
    if (!searchInput.value.trim()) { searchBusy = false; drawList(); return; }
    searchBusy = true; drawList(); searchTimer = setTimeout(() => void search(), 200);
  });
  searchInput.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); clearTimeout(searchTimer); void search(); } });
  scopeInput.addEventListener("change", () => {
    scope = scopeInput.value as SkillScope; reset(); draw(); if (active) void refresh();
  });
  url.addEventListener("input", () => { clearPreview(); drawPreview(); drawSource(); });
  url.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); void previewImport(); } });
  draw();
  return {element, updateContext(next, nextOwner, enabled = true) {
    const changed = owner !== nextOwner || context.conversationId !== next.conversationId || context.workspacePath !== next.workspacePath;
    if (changed) { context = {...next}; owner = nextOwner; reset(); draw(); }
    active = enabled; if (active && !loaded) void refresh();
  }};
}
