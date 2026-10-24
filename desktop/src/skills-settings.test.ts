import { expect, test, vi } from "vitest";
import { createSkillsSettingsPanel } from "./skills-settings";
import type { SkillsApi, SkillInfo, SkillPreview } from "./skills-types";

const context = { conversationId: null, workspacePath: "/project/a" };
const source = { repository: "team/repo", reference: "main", revision: "a".repeat(40), path: "skills/review" };
const skill: SkillInfo = { id: "personal:review", name: "review", title: "Review", description: "Inspect changes", scope: "personal", enabled: true, source };
const preview: SkillPreview = { ...skill, source, content: "# Review\n<script>bad()</script>", files: ["SKILL.md", "references/guide.md"] };
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function fixture(overrides: Partial<SkillsApi> = {}) {
  const api: SkillsApi = {
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
  const click = (text: string) => {
    const button = [...root.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === text);
    expect(button, text).toBeDefined(); button!.click();
  };
  const url = () => root.querySelector<HTMLInputElement>('[aria-label="GitHub skill URL"]')!;
  const setUrl = (value = "https://github.com/team/repo") => { url().value = value; url().dispatchEvent(new Event("input")); };
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

test("previews before explicit pinned installation and handles Enter without submitting settings", async () => {
  const {api, panel, root, click, setUrl, url} = fixture();
  const form = document.createElement("form"); form.append(root);
  const submit = vi.fn(e => e.preventDefault()); form.addEventListener("submit", submit);
  panel.updateContext(context, "a", true); await flush(); setUrl();
  const enter = new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}); url().dispatchEvent(enter); await flush();
  expect(enter.defaultPrevented).toBe(true); expect(submit).not.toHaveBeenCalled();
  expect(root.textContent).toContain(source.revision);
  expect(root.textContent).toContain("references/guide.md");
  expect(api.installSkill).not.toHaveBeenCalled(); click("Install in Personal"); await flush();
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

test("toggle captures authority, disables the operation, and cannot publish into another context", async () => {
  let finish!: (value: SkillInfo) => void;
  const {api, panel, root, click} = fixture({setSkillEnabled: vi.fn(() => new Promise<SkillInfo>(resolve => {finish = resolve;}))});
  panel.updateContext(context, "a", true); await flush(); click("Disable");
  expect([...root.querySelectorAll("button")].find(b => b.textContent === "Saving…")?.disabled).toBe(true);
  expect(api.setSkillEnabled).toHaveBeenCalledWith(skill.id, false, context);
  panel.updateContext({...context, workspacePath: "/project/b"}, "b", true); await flush(); finish({...skill, enabled: false}); await flush();
  expect(root.textContent).toContain("Disable");
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
  setUrl(); click("Preview"); await flush(); click("Install in Project");
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
  click("Install in Personal"); await flush();
  expect(root.textContent).toContain("Permission denied");
  expect(root.querySelector(".skills-preview")?.textContent).toContain(preview.content);
  click("Install in Personal"); await flush();
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
  panel.updateContext(context, "a", true); await flush(); setUrl(); click("Preview"); await flush(); click("Install in Personal");
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  expect(scope.disabled).toBe(true);
  finish(skill); await flush(); expect(scope.disabled).toBe(false);
});

function fill(root: HTMLElement, label: string, value: string) {
  const field = root.querySelector<HTMLInputElement | HTMLTextAreaElement>(`[aria-label="${label}"]`)!;
  expect(field, label).not.toBeNull(); field.value = value; field.dispatchEvent(new Event("input"));
}

test("creates a local skill in the selected scope and preserves failed drafts across navigation", async () => {
  const {api, panel, root, click} = fixture({createSkill: vi.fn().mockRejectedValueOnce(new Error("Name exists")).mockResolvedValue(skill)});
  panel.updateContext(context, "a", true); await flush();
  const select = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  select.value = "project"; select.dispatchEvent(new Event("change")); await flush();
  fill(root, "Skill name", "review"); fill(root, "Skill title (optional)", "Review");
  fill(root, "Skill summary", "Inspect changes"); fill(root, "Skill instructions", "Read diffs.");
  const form = document.createElement("form"); form.append(root);
  const submit = vi.fn(e => e.preventDefault()); form.addEventListener("submit", submit);
  click("Create skill"); await flush();
  expect(api.createSkill).toHaveBeenCalledWith({name: "review", title: "Review", description: "Inspect changes", instructions: "Read diffs."}, "project", context);
  expect(root.textContent).toContain("Name exists");
  panel.updateContext(context, "a", false); panel.updateContext(context, "a", true);
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill instructions"]')!.value).toBe("Read diffs.");
  expect(submit).not.toHaveBeenCalled(); click("Create skill"); await flush();
  expect(root.textContent).toContain("Review created.");
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
  expect(root.querySelector("script")).toBeNull(); click("Install local skill"); await flush();
  expect(api.installLocalSkill).toHaveBeenCalledWith("/chosen/review", "package", "personal", context);
  expect(root.textContent).toContain("Skill changed since preview");
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

test("open edit and local previews survive category navigation and edit reopens retain their original fingerprint", async () => {
  const {api, panel, root, click} = fixture();
  panel.updateContext(context, "a", true); await flush(); click("Edit skill"); await flush();
  fill(root, "Skill document review", "unsaved edit");
  click("Choose skill file"); await flush();
  panel.updateContext(context, "a", false); panel.updateContext(context, "a", true);
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill document review"]')!.value).toBe("unsaved edit");
  expect(root.querySelector(".skills-local-preview")!.textContent).toContain(preview.content);
  click("Edit skill"); await flush();
  expect(api.readSkill).toHaveBeenCalledTimes(1);
  click("Save skill"); await flush();
  expect(api.updateSkillDocument).toHaveBeenCalledWith(skill.id, "unsaved edit", "original", context);
});

test("scope changes invalidate local previews while retaining creation fields and optional sources are collapsed", async () => {
  const {panel, root, click} = fixture(); panel.updateContext(context, "a", true); await flush();
  expect(root.querySelector<HTMLDetailsElement>(".skills-other-sources")!.open).toBe(false);
  fill(root, "Skill name", "draft"); click("Choose skill file"); await flush();
  const select = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  select.value = "project"; select.dispatchEvent(new Event("change")); await flush();
  expect(root.querySelector(".skills-local-preview")!.textContent).not.toContain(preview.content);
  expect(root.querySelector<HTMLInputElement>('[aria-label="Skill name"]')!.value).toBe("draft");
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

test.each([false, true])("GitHub installation preserves open edits and their original fingerprint (same skill: %s)", async sameSkill => {
  const installed = sameSkill ? skill : {...skill, id: "personal:another", name: "another", title: "Another"};
  const {api, panel, root, click, setUrl} = fixture({
    installSkill: vi.fn(async () => installed),
    updateSkillDocument: vi.fn().mockRejectedValue(new Error("Skill changed since read")),
  });
  panel.updateContext(context, "a", true); await flush(); click("Edit skill"); await flush();
  fill(root, "Skill document review", "unsaved review draft");
  setUrl(); click("Preview"); await flush(); click("Install in Personal"); await flush();
  expect(root.querySelector<HTMLTextAreaElement>('[aria-label="Skill document review"]')?.value).toBe("unsaved review draft");
  click("Save skill"); await flush();
  expect(api.updateSkillDocument).toHaveBeenCalledWith(skill.id, "unsaved review draft", "original", context);
  expect(root.textContent).toContain("Skill changed since read");
});

test("GitHub installation invalidates only the installed skill's read-only cached document", async () => {
  const {panel, root, click, setUrl} = fixture();
  panel.updateContext(context, "a", true); await flush(); click("View instructions"); await flush();
  expect(root.querySelector(".skill-installed .skill-instructions")).not.toBeNull();
  setUrl(); click("Preview"); await flush(); click("Install in Personal"); await flush();
  expect(root.querySelector(".skill-installed .skill-instructions")).toBeNull();
});
