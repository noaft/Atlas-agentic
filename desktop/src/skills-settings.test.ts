import { afterEach, expect, test, vi } from "vitest";
import { createSkillsSettingsPanel } from "./skills-settings";
import type { SkillsApi, SkillDocument, SkillInfo, SkillPreview } from "./skills-types";

afterEach(() => document.body.replaceChildren());

const context = { conversationId: null, workspacePath: "/project/a" };
const source = { repository: "team/repo", reference: "main", revision: "a".repeat(40), path: "skills/review" };
const skill: SkillInfo = { id: "personal:review", name: "review", title: "Review", description: "Inspect changes", scope: "personal", enabled: true, source };
const preview: SkillPreview = { ...skill, source, content: "# Review\n<script>bad()</script>", files: ["SKILL.md", "references/guide.md"] };
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function fixture(overrides: Partial<SkillsApi> = {}) {
  const api: SkillsApi = {
    installSkills: vi.fn(async () => [{source, skill, error: null}]),
    searchSkills: vi.fn(async () => [{skill, score: 1}]),
    createSkill: vi.fn(async () => skill),
    updateSkillDocument: vi.fn(async () => ({...skill, source: null})),
    chooseSkillImport: vi.fn(async () => "/chosen/review"),
    previewLocalSkills: vi.fn(async () => [{...preview, path: "/chosen/review", fingerprint: "package"}]),
    installLocalSkill: vi.fn(async () => skill),
    listSkills: vi.fn(async () => ({skills: [skill], issues: []})),
    readSkill: vi.fn(async () => ({skill, content: preview.content, files: preview.files, fingerprint: "original"})),
    previewSkillImport: vi.fn(async () => [preview]),
    installSkill: vi.fn(async () => skill),
    setSkillEnabled: vi.fn(async (_id, enabled) => ({...skill, enabled})), ...overrides,
  };
  const panel = createSkillsSettingsPanel(api, context, "a");
  const root = panel.element;
  document.body.append(root);
  const click = (text: string) => {
    const button = [...root.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === text);
    expect(button, text).toBeDefined();
    const add = button!.closest<HTMLElement>(".skills-add");
    if (add?.hidden) root.querySelector<HTMLButtonElement>("[data-add-skill]")!.click();
    if (text === "Choose skill file" || text === "Choose skill folder") {
      const source = root.querySelector<HTMLSelectElement>('[aria-label="Skill source"]')!;
      source.value = text === "Choose skill file" ? "file" : "folder"; source.dispatchEvent(new Event("change"));
    }
    const parents: HTMLDetailsElement[] = [];
    for (let parent = button!.parentElement; parent; parent = parent.parentElement) if (parent instanceof HTMLDetailsElement) parents.unshift(parent);
    for (const parent of parents) if (!parent.open) parent.querySelector<HTMLElement>(":scope > summary")!.click();
    button!.click();
  };
  const url = () => root.querySelector<HTMLInputElement>('[aria-label="GitHub skill URL"]')!;
  const setUrl = (value = "https://github.com/team/repo") => {
    if (root.querySelector<HTMLElement>(".skills-add")!.hidden) root.querySelector<HTMLButtonElement>("[data-add-skill]")!.click();
    const source = root.querySelector<HTMLSelectElement>('[aria-label="Skill source"]')!;
    source.value = "github"; source.dispatchEvent(new Event("change"));
    url().value = value; url().dispatchEvent(new Event("input"));
  };
  return {api, panel, root, click, url, setUrl};
}

test("loads on activation, filters scopes and views untrusted instructions as text", async () => {
  const {api, panel, root, click} = fixture();
  expect(api.listSkills).not.toHaveBeenCalled();
  panel.updateContext(context, "a", true); await flush();
  expect(root.textContent).toContain("Review");
  click("View instructions"); await flush();
  expect(root.textContent).toContain("<script>bad()</script>");
  expect(root.querySelector("script")).toBeNull();
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  scope.value = "project"; scope.dispatchEvent(new Event("change")); await flush();
  expect(root.textContent).not.toContain("Inspect changes");
  expect(root.textContent).toContain("/project/a");
});

test("toggle captures authority, disables the operation, and cannot publish into another context", async () => {
  let finish!: (value: SkillInfo) => void;
  const {api, panel, root, click} = fixture({setSkillEnabled: vi.fn(() => new Promise<SkillInfo>(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush(); root.querySelector<HTMLInputElement>('[role="switch"]')!.click();
  expect(root.querySelector<HTMLInputElement>('[role="switch"]')!.getAttribute("aria-disabled")).toBe("true");
  expect(api.setSkillEnabled).toHaveBeenCalledWith(skill.id, false, context);
  panel.updateContext({...context, workspacePath: "/project/b"}, "b", true); await flush(); finish({...skill, enabled: false}); await flush();
  expect(root.querySelector<HTMLInputElement>('[role="switch"]')!.checked).toBe(true);
});

test("late list and instruction results cannot disclose an old project's skills", async () => {
  let finishList!: (value: {skills: SkillInfo[]; issues: string[]}) => void;
  let finishRead!: (value: {skill: SkillInfo; content: string; files: string[]; fingerprint: string}) => void;
  const project = {...skill, scope: "project" as const, id: "project:review"};
  const {api, panel, root, click} = fixture({
    listSkills: vi.fn().mockResolvedValueOnce({skills: [project], issues: []}).mockResolvedValueOnce({skills: [project], issues: []}).mockImplementationOnce(() => new Promise(resolve => {finishList = resolve;})).mockResolvedValue({skills: [], issues: []}),
    readSkill: vi.fn(() => new Promise<{skill: SkillInfo; content: string; files: string[]; fingerprint: string}>(resolve => {finishRead = resolve;})),
  });
  panel.updateContext(context, "a", true); await flush();
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  scope.value = "project"; scope.dispatchEvent(new Event("change"));
  await flush();
  click("View instructions"); click("Refresh skills");
  panel.updateContext({...context, workspacePath: "/private/b"}, "b", true); await flush();
  finishList({skills: [project], issues: []});
  finishRead({skill: project, content: "PRIVATE PROJECT A", files: [], fingerprint: "original"}); await flush();
  expect(root.textContent).not.toContain("PRIVATE PROJECT A");
  expect(root.textContent).not.toContain("Inspect changes");
  expect(api.readSkill).toHaveBeenCalledWith(project.id, context);
});

test("editing saves the original fingerprint, retains conflict drafts and reloads the document after success", async () => {
  const {api, panel, root, click} = fixture({updateSkillDocument: vi.fn().mockRejectedValueOnce(new Error("Skill changed since read")).mockResolvedValue({...skill, source: null})});
  panel.updateContext(context, "a", true); await flush(); click("Edit skill"); await flush();
  fill(root, "Skill document review", "edited full SKILL.md"); click("Save skill"); await flush();
  expect(api.updateSkillDocument).toHaveBeenCalledWith(skill.id, "edited full SKILL.md", "original", context);
  expect(root.textContent).toContain("Skill changed since read");
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill document review"]')!.value).toBe("edited full SKILL.md");
  expect(root.textContent).toContain("references/guide.md"); click("Save skill"); await flush();
  expect(api.readSkill).toHaveBeenCalledTimes(2);
});

test("refreshing installed skills preserves an editor's focus, selection and draft", async () => {
  const {panel, root, click} = fixture(); document.body.append(root);
  try {
    panel.updateContext(context, "a", true); await flush(); click("Edit skill"); await flush();
    const editor = root.querySelector<HTMLTextAreaElement>('[aria-label="Skill document review"]')!;
    editor.value = "unfinished instructions"; editor.focus(); editor.setSelectionRange(2, 8);
    click("Refresh skills"); await flush();
    expect(document.activeElement).toBe(editor);
    expect(editor.selectionStart).toBe(2); expect(editor.selectionEnd).toBe(8);
    expect(editor.value).toBe("unfinished instructions");
  } finally { root.remove(); }
});

function fill(root: HTMLElement, label: string, value: string) {
  const field = root.querySelector<HTMLTextAreaElement>(`[aria-label="${label}"]`)!;
  expect(field, label).not.toBeNull(); field.value = value;
}

test("offers installation first and keeps installed skills in a separate section without creation controls", async () => {
  const {api, panel, root} = fixture();
  panel.updateContext(context, "a", true); await flush();
  expect(root.querySelector(".skills-add h2")?.textContent).toBe("Add skills");
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
  const importer = root.querySelector<HTMLElement>(".skills-add");
  const installed = root.querySelector<HTMLElement>(".skills-library");
  expect(importer?.compareDocumentPosition(installed!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  expect(importer?.querySelector('[aria-label="GitHub skill URL"]')).not.toBeNull();
  expect(importer?.textContent).toContain("Choose skill folder");
  expect(installed?.querySelector('[aria-label="Search skills"]')).not.toBeNull();
  expect(installed?.textContent).toContain("1 installed · 1 enabled");
  expect(root.textContent).not.toMatch(/Create skill|Create a skill/);
  expect(api.createSkill).not.toHaveBeenCalled();
  expect(api.chooseSkillImport).not.toHaveBeenCalled();
  expect(api.previewSkillImport).not.toHaveBeenCalled();
});

test("list failures can be retried without affecting installed skill management", async () => {
  const {panel, root, click} = fixture({listSkills: vi.fn().mockRejectedValueOnce(new Error("List failed")).mockResolvedValue({skills: [skill], issues: []})});
  panel.updateContext(context, "a", true); await flush();
  expect(root.textContent).toContain("List failed");
  click("Refresh skills"); await flush();
  expect(root.textContent).not.toContain("List failed");
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
});

test("previews before explicit pinned installation and handles Enter without submitting settings", async () => {
  const {api, panel, root, click, setUrl, url} = fixture();
  const form = document.createElement("form"); form.append(root);
  const submit = vi.fn(e => e.preventDefault()); form.addEventListener("submit", submit);
  panel.updateContext(context, "a", true); await flush(); setUrl();
  const enter = new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}); url().dispatchEvent(enter); await flush();
  expect(enter.defaultPrevented).toBe(true); expect(submit).not.toHaveBeenCalled();
  expect(root.textContent).toContain(source.revision);
  expect(root.textContent).toContain("references/guide.md");
  expect(api.installSkill).not.toHaveBeenCalled(); click("Add to all projects"); await flush();
  expect(api.installSkill).toHaveBeenCalledWith(source, "personal", false, context);
});

test("explicit update previews only the installed source then replaces on confirmation", async () => {
  const {api, panel, click} = fixture(); panel.updateContext(context, "a", true); await flush();
  click("Update"); await flush();
  expect(api.previewSkillImport).toHaveBeenCalledWith("https://github.com/team/repo/tree/main/skills/review");
  expect(api.installSkill).not.toHaveBeenCalled(); click("Update installed skill"); await flush();
  expect(api.installSkill).toHaveBeenCalledWith(source, "personal", true, context);
});

test("context changes clear old project data and ignore late previews while preserving URL", async () => {
  let finish!: (value: SkillPreview[]) => void;
  const {api, panel, root, click, setUrl, url} = fixture({previewSkillImport: () => new Promise(resolve => {finish = resolve;})});
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview");
  const next = {conversationId: "saved-b", workspacePath: "/project/b"}; panel.updateContext(next, "b", true);
  finish([preview]); await flush();
  expect(url().value).toBe("https://github.com/team/repo");
  expect(root.querySelector(".skills-preview")?.textContent).not.toContain(source.revision);
  expect(api.listSkills).toHaveBeenLastCalledWith(next);
});

test("list and preview errors remain retryable and editing URL invalidates an old preview", async () => {
  const {api, panel, root, click, setUrl} = fixture({listSkills: vi.fn().mockRejectedValueOnce(new Error("List failed")).mockResolvedValue({skills: [], issues: []}), previewSkillImport: vi.fn().mockRejectedValueOnce(new Error("Preview failed")).mockResolvedValue([preview])});
  panel.updateContext(context, "a", true); await flush(); expect(root.textContent).toContain("List failed");
  click("Refresh skills"); await flush(); expect(root.textContent).not.toContain("List failed");
  setUrl(); click("Preview"); await flush(); expect(root.textContent).toContain("Preview failed");
  click("Preview"); await flush(); expect(root.textContent).toContain(source.revision);
  setUrl("https://github.com/team/other"); expect(root.textContent).not.toContain(source.revision);
  expect(api.installSkill).not.toHaveBeenCalled();
});

test("installation keeps its captured Project scope and starting folder while another chat opens", async () => {
  let finish!: (value: SkillInfo) => void;
  const {api, panel, root, click, setUrl, url} = fixture({installSkill: vi.fn(() => new Promise<SkillInfo>(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush();
  const select = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  select.value = "project"; select.dispatchEvent(new Event("change")); await flush();
  setUrl(); click("Preview"); await flush(); click("Add to this project");
  expect(url().disabled).toBe(true);
  expect([...root.querySelectorAll("button")].find(b => b.textContent === "Installing…")?.disabled).toBe(true);
  expect(api.installSkill).toHaveBeenCalledWith(source, "project", false, context);
  panel.updateContext({...context, workspacePath: "/private/b"}, "b", true); finish({...skill, scope: "project"}); await flush();
  expect(root.textContent).toContain("/private/b");
  expect(root.textContent).not.toContain("Review installed.");
  expect(url().disabled).toBe(false);
});

test("install failure retains preview and permits explicit retry", async () => {
  const {api, panel, root, click, setUrl} = fixture({installSkill: vi.fn().mockRejectedValueOnce(new Error("Permission denied")).mockResolvedValue(skill)});
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush();
  click("Add to all projects"); await flush();
  expect(root.textContent).toContain("Permission denied");
  expect(root.querySelector(".skills-preview")?.textContent).toContain(preview.content);
  click("Add to all projects"); await flush();
  expect(api.installSkill).toHaveBeenCalledTimes(2);
  expect(root.textContent).toContain("Review installed.");
});

test("an explicit update refuses a different skill from the same repository", async () => {
  const {api, panel, root, click} = fixture({previewSkillImport: async () => [{...preview, name: "unrelated"}]});
  panel.updateContext(context, "a", true); await flush(); click("Update"); await flush();
  expect(root.textContent).toContain("The installed skill was not found");
  expect(root.textContent).not.toContain("Update installed skill");
  expect(api.installSkill).not.toHaveBeenCalled();
});

test("updates preserve slash-containing GitHub references in the preview URL", async () => {
  const branchSource = {...source, reference: "feature/review"};
  const branchSkill = {...skill, source: branchSource};
  const {api, panel, click} = fixture({
    listSkills: async () => ({skills: [branchSkill], issues: []}),
    previewSkillImport: vi.fn(async () => [{...preview, source: branchSource}]),
  });
  panel.updateContext(context, "a", true); await flush(); click("Update"); await flush();
  expect(api.previewSkillImport).toHaveBeenCalledWith("https://github.com/team/repo/tree/feature/review/skills/review");
});

test("scope selection stays locked while installation is pending", async () => {
  let finish!: (value: SkillInfo) => void;
  const {panel, root, click, setUrl} = fixture({installSkill: () => new Promise(resolve => {finish = resolve;})});
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush(); click("Add to all projects");
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  expect(scope.disabled).toBe(true);
  finish(skill); await flush(); expect(scope.disabled).toBe(false);
});

test("canceled local dialogs do not preview and stale dialogs cannot import into another project", async () => {
  let finish!: (path: string | null) => void;
  const {api, panel, root, click} = fixture({chooseSkillImport: vi.fn().mockResolvedValueOnce(null).mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush();
  click("Choose skill file"); await flush(); expect(api.previewLocalSkills).not.toHaveBeenCalled();
  click("Choose skill folder"); panel.updateContext({...context, workspacePath: "/other"}, "b", true);
  finish("/chosen/review"); await flush();
  expect(api.previewLocalSkills).not.toHaveBeenCalled(); expect(root.textContent).not.toContain("references/guide.md");
});

test("local imports show text and retain changed-package errors with captured fingerprint", async () => {
  const {api, panel, root, click} = fixture({installLocalSkill: vi.fn().mockRejectedValue(new Error("Skill changed since preview"))});
  panel.updateContext(context, "a", true); await flush(); click("Choose skill folder"); await flush();
  expect(root.querySelector(".skills-local-preview")!.textContent).toContain(preview.content);
  expect(root.querySelector("script")).toBeNull(); click("Add this skill"); await flush();
  expect(api.installLocalSkill).toHaveBeenCalledWith("/chosen/review", "package", "personal", context);
  expect(root.textContent).toContain("Skill changed since preview");
});

test.each([false, true])("GitHub installation preserves open edits and their original fingerprint (same skill: %s)", async sameSkill => {
  const installed = sameSkill ? skill : {...skill, id: "personal:another", name: "another", title: "Another"};
  const {api, panel, root, click, setUrl} = fixture({
    installSkill: vi.fn(async () => installed),
    updateSkillDocument: vi.fn().mockRejectedValue(new Error("Skill changed since read")),
  });
  panel.updateContext(context, "a", true); await flush(); click("Edit skill"); await flush();
  fill(root, "Skill document review", "unsaved review draft");
  setUrl(); click("Preview"); await flush(); click("Add to all projects"); await flush();
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill document review"]')?.value).toBe("unsaved review draft");
  click("Save skill"); await flush();
  expect(api.updateSkillDocument).toHaveBeenCalledWith(skill.id, "unsaved review draft", "original", context);
  expect(root.textContent).toContain("Skill changed since read");
});

test("GitHub installation invalidates only the installed skill's read-only cached document", async () => {
  const {panel, root, click, setUrl} = fixture();
  panel.updateContext(context, "a", true); await flush(); click("View instructions"); await flush();
  expect(root.querySelector(".skill-installed .skill-instructions")).not.toBeNull();
  setUrl(); click("Preview"); await flush(); click("Add to all projects"); await flush();
  expect(root.querySelector(".skill-installed .skill-instructions")).toBeNull();
});

test("installed instructions close and reopen without another read", async () => {
  const {api, panel, root, click} = fixture(); panel.updateContext(context, "a", true); await flush();
  click("View instructions"); await flush();
  click("Close instructions");
  expect(root.querySelector(".skill-installed .skill-instructions")).toBeNull();
  click("View instructions"); await flush();
  expect(root.querySelector(".skill-installed .skill-instructions")).not.toBeNull();
  expect(api.readSkill).toHaveBeenCalledTimes(1);
});

test("only the selected preview installs and other candidates remain available", async () => {
  let finish!: (value: SkillInfo) => void;
  const second = {...preview, name: "second", title: "Second", source: {...source, path: "skills/second"}};
  const {api, panel, root, click, setUrl} = fixture({
    previewSkillImport: async () => [preview, second],
    installSkill: vi.fn(() => new Promise<SkillInfo>(resolve => {finish = resolve;})),
    listSkills: vi.fn().mockResolvedValueOnce({skills: [], issues: []}).mockRejectedValue(new Error("Refresh unavailable")),
  });
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush(); click("Add to all projects");
  expect([...root.querySelectorAll(".skill-preview button")].map(b => b.textContent)).toEqual(["Installing…", "Add to all projects"]);
  finish(skill); await flush();
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
  expect(root.querySelector(".skills-preview")?.textContent).toContain("Second");
  expect(api.installSkill).toHaveBeenCalledTimes(1);
  expect(root.textContent).not.toContain("Installing…");
});

test("local collection imports retain unselected candidates and publish verified results", async () => {
  const second = {...preview, name: "second", title: "Second", path: "/chosen/second", fingerprint: "second"};
  const {panel, root, click} = fixture({
    previewLocalSkills: async () => [{...preview, path: "/chosen/review", fingerprint: "package"}, second],
    listSkills: vi.fn().mockResolvedValueOnce({skills: [], issues: []}).mockRejectedValue(new Error("Refresh unavailable")),
  });
  panel.updateContext(context, "a", true); await flush(); click("Choose skill folder"); await flush(); click("Add this skill"); await flush();
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
  expect(root.querySelector(".skills-local-preview")?.textContent).toContain("Second");
});

test("Install all sends one backend batch and retains failed candidates for retry", async () => {
  const secondSource = {...source, path: "skills/second"};
  const {api, panel, root, click, setUrl} = fixture({
    previewSkillImport: async () => [preview, {...preview, name: "second", title: "Second", source: secondSource}],
    installSkills: vi.fn(async () => [{source, skill, error: null}, {source: secondSource, skill: null, error: "Second: file unavailable"}]),
  });
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush(); click("Add all"); await flush();
  expect(api.installSkills).toHaveBeenCalledWith([source, secondSource], "personal", context);
  expect(api.installSkill).not.toHaveBeenCalled();
  expect(root.querySelector(".skills-preview")?.textContent).toContain("Second: file unavailable");
  expect(root.querySelector(".skills-preview")?.textContent).toContain("Second");
  expect(root.querySelectorAll(".skill-preview")).toHaveLength(1);
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
});

test("BM25 search calls the backend and ignores results from an old project", async () => {
  vi.useFakeTimers();
  try {
    let finish!: (value: {skill: SkillInfo; score: number}[]) => void;
    const {api, panel, root} = fixture({searchSkills: vi.fn().mockImplementationOnce(() => new Promise(resolve => {finish = resolve;})).mockResolvedValue([])});
    panel.updateContext(context, "a", true); await flush();
    const search = root.querySelector<HTMLInputElement>('[aria-label="Search skills"]')!;
    search.value = "rust review"; search.dispatchEvent(new Event("input"));
    await vi.advanceTimersByTimeAsync(200); await flush();
    expect(api.searchSkills).toHaveBeenCalledWith("rust review", "personal", context);
    panel.updateContext({...context, workspacePath: "/project/b"}, "b", true); await flush();
    finish([{skill: {...skill, title: "PRIVATE OLD RESULT"}, score: 3}]); await flush();
    expect(root.textContent).not.toContain("PRIVATE OLD RESULT");
    search.value = ""; search.dispatchEvent(new Event("input")); await flush();
    expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
  } finally { vi.useRealTimers(); }
});

test("preview instructions can open and close without starting an installation", async () => {
  const {api, panel, root, click, setUrl} = fixture();
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush();
  const disclosure = root.querySelector<HTMLDetailsElement>(".skill-preview details.skill-instructions")!;
  expect(disclosure.open).toBe(false);
  disclosure.querySelector("summary")!.click(); expect(disclosure.open).toBe(true);
  disclosure.querySelector("summary")!.click(); expect(disclosure.open).toBe(false);
  expect(api.installSkill).not.toHaveBeenCalled();
});


test("disabled skills remain installed but are explained as unavailable to Atlas", async () => {
  const {panel, root} = fixture({listSkills: vi.fn(async () => ({skills: [{...skill, enabled: false}], issues: []}))});
  panel.updateContext(context, "a", true); await flush();
  expect(root.querySelector(".skills-library")?.textContent).toContain("1 installed · 0 enabled");
  expect(root.querySelector(".skills-library")?.textContent).toContain("Enable a skill to make it available to Atlas");
});


test("default library shows only the main add action and compact skill controls", async () => {
  const {panel, root} = fixture();
  panel.updateContext(context, "a", true); await flush();
  const add = root.querySelector<HTMLElement>(".skills-add")!;
  expect(add.hidden).toBe(true);
  const trigger = root.querySelector<HTMLButtonElement>('[data-add-skill]')!;
  expect(trigger.textContent).toBe("Add skill");
  expect(trigger.getAttribute("aria-expanded")).toBe("false");
  const card = root.querySelector<HTMLElement>(".skill-installed")!;
  const toggle = card.querySelector<HTMLInputElement>('[role="switch"]')!;
  expect(toggle.checked).toBe(true);
  expect(toggle.getAttribute("aria-label")).toBe("Use Review");
  const details = card.querySelector<HTMLDetailsElement>(".skill-details")!;
  expect(details.open).toBe(false);
  expect(card.querySelectorAll("button").length).toBe(details.querySelectorAll("button").length);
  trigger.click();
  expect(add.hidden).toBe(false);
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
});

test("add flow offers one source at a time and retains the link when closed", async () => {
  const {panel, root} = fixture(); panel.updateContext(context, "a", true); await flush();
  const trigger = root.querySelector<HTMLButtonElement>('[data-add-skill]')!;
  trigger.click();
  const source = root.querySelector<HTMLSelectElement>('[aria-label="Skill source"]')!;
  const url = root.querySelector<HTMLInputElement>('[aria-label="GitHub skill URL"]')!;
  expect(root.querySelector<HTMLElement>(".skills-github-source")!.hidden).toBe(false);
  expect(root.querySelector<HTMLElement>(".skills-local-source")!.hidden).toBe(true);
  url.value = "https://github.com/team/repo"; url.dispatchEvent(new Event("input"));
  source.value = "folder"; source.dispatchEvent(new Event("change"));
  expect(root.querySelector<HTMLElement>(".skills-github-source")!.hidden).toBe(true);
  expect(root.querySelector<HTMLElement>(".skills-local-source")!.hidden).toBe(false);
  trigger.click(); trigger.click();
  expect(url.value).toBe("https://github.com/team/repo");
  expect(source.value).toBe("folder");
});

test("catalog issues are grouped and collapsed without hiding healthy skills", async () => {
  const issue = "personal:linked: skill folder must be a directory, not a symlink";
  const {panel, root} = fixture({listSkills: async () => ({skills: [skill], issues: [issue, "personal:broken: invalid metadata"]})});
  panel.updateContext(context, "a", true); await flush();
  const disclosure = root.querySelector<HTMLDetailsElement>(".skills-issues")!;
  expect(disclosure.open).toBe(false);
  expect(disclosure.querySelector("summary")?.textContent).toBe("2 skills need attention");
  expect(disclosure.textContent).toContain(issue);
  expect(disclosure.textContent).toContain("Import the actual skill folder");
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
});

test("skill details stay expanded across toggle and background refresh", async () => {
  const {panel, root, click} = fixture(); panel.updateContext(context, "a", true); await flush();
  root.querySelector<HTMLElement>(".skill-details summary")!.click();
  root.querySelector<HTMLInputElement>('[role="switch"]')!.click(); await flush();
  expect(root.querySelector<HTMLDetailsElement>(".skill-details")!.open).toBe(true);
  click("Refresh skills"); await flush();
  expect(root.querySelector<HTMLDetailsElement>(".skill-details")!.open).toBe(true);
});


test("closing the add panel preserves a pending preview and its reviewed result", async () => {
  let finish!: (value: SkillPreview[]) => void;
  const {api, panel, root, click, setUrl} = fixture({previewSkillImport: () => new Promise(resolve => {finish = resolve;})});
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview");
  root.querySelector<HTMLButtonElement>("[data-add-skill]")!.click();
  expect(root.querySelector<HTMLElement>(".skills-add")!.hidden).toBe(true);
  finish([preview]); await flush();
  root.querySelector<HTMLButtonElement>("[data-add-skill]")!.click();
  expect(root.querySelector(".skills-preview")?.textContent).toContain(preview.content);
  expect(api.installSkill).not.toHaveBeenCalled();
});

test("editing the URL during preview unlocks source choice and ignores the stale result", async () => {
  let finish!: (value: SkillPreview[]) => void;
  const {panel, root, click, setUrl} = fixture({previewSkillImport: () => new Promise(resolve => {finish = resolve;})});
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview");
  const source = root.querySelector<HTMLSelectElement>('[aria-label="Skill source"]')!;
  expect(source.disabled).toBe(true);
  setUrl("https://github.com/team/other");
  expect(source.disabled).toBe(false);
  finish([preview]); await flush();
  expect(root.querySelector(".skills-preview")?.textContent).not.toContain(preview.content);
});

test("keyboard focus stays on the same skill switch while saving and after completion", async () => {
  let finish!: (value: SkillInfo) => void;
  const {api, panel, root} = fixture({setSkillEnabled: vi.fn(() => new Promise<SkillInfo>(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush();
  const toggle = root.querySelector<HTMLInputElement>('[role="switch"]')!; toggle.focus(); toggle.click();
  const saving = root.querySelector<HTMLInputElement>('[role="switch"]')!;
  expect(document.activeElement).toBe(saving);
  expect(saving.getAttribute("aria-disabled")).toBe("true");
  saving.click();
  expect(api.setSkillEnabled).toHaveBeenCalledTimes(1);
  finish({...skill, enabled: false}); await flush();
  expect(document.activeElement).toBe(root.querySelector('[role="switch"]'));
  expect(root.querySelector<HTMLInputElement>('[role="switch"]')!.checked).toBe(false);
});

test("a late instruction read respects Details being collapsed by the user", async () => {
  let finish!: (value: {skill: SkillInfo; content: string; files: string[]; fingerprint: string}) => void;
  const {api, panel, root, click} = fixture({readSkill: vi.fn(() => new Promise<SkillDocument>(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush(); click("View instructions");
  root.querySelector<HTMLElement>(".skill-details summary")!.click();
  finish({skill, content: preview.content, files: preview.files, fingerprint: "original"}); await flush();
  expect(root.querySelector<HTMLDetailsElement>(".skill-details")!.open).toBe(false);
  root.querySelector<HTMLElement>(".skill-details summary")!.click();
  expect(root.querySelector(".skill-instructions")?.textContent).toContain(preview.content);
  expect(api.readSkill).toHaveBeenCalledTimes(1);
});

test("folder previews are hidden when selecting the file source and restored when returning", async () => {
  const {panel, root, click} = fixture(); panel.updateContext(context, "a", true); await flush();
  click("Choose skill folder"); await flush();
  const area = root.querySelector<HTMLElement>(".skills-local-preview")!;
  const source = root.querySelector<HTMLSelectElement>('[aria-label="Skill source"]')!;
  expect(area.hidden).toBe(false);
  source.value = "file"; source.dispatchEvent(new Event("change"));
  expect(area.hidden).toBe(true);
  source.value = "folder"; source.dispatchEvent(new Event("change"));
  expect(area.hidden).toBe(false);
  expect(area.textContent).toContain(preview.content);
});

test("Enter toggles the skill without submitting the surrounding Settings form", async () => {
  const {api, panel, root} = fixture(); panel.updateContext(context, "a", true); await flush();
  const form = document.createElement("form"); document.body.append(form); form.append(root);
  const submit = vi.fn(event => event.preventDefault()); form.addEventListener("submit", submit);
  const toggle = root.querySelector<HTMLInputElement>('[role="switch"]')!; toggle.focus();
  const enter = new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}); toggle.dispatchEvent(enter); await flush();
  expect(enter.defaultPrevented).toBe(true);
  expect(submit).not.toHaveBeenCalled();
  expect(api.setSkillEnabled).toHaveBeenCalledWith(skill.id, false, context);
});
