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
  return {api, panel, root, click};
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
  panel.updateContext(context, "a", true); await flush(); click("Disable");
  expect([...root.querySelectorAll("button")].find(b => b.textContent === "Saving…")?.disabled).toBe(true);
  expect(api.setSkillEnabled).toHaveBeenCalledWith(skill.id, false, context);
  panel.updateContext({...context, workspacePath: "/project/b"}, "b", true); await flush(); finish({...skill, enabled: false}); await flush();
  expect(root.textContent).toContain("Disable");
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

test("shows installed skills first without creation or import controls", async () => {
  const {api, panel, root} = fixture();
  panel.updateContext(context, "a", true); await flush();
  expect(root.firstElementChild?.textContent).toBe("Installed skills");
  expect(root.querySelector(".skill-installed")?.textContent).toContain("Review");
  expect(root.querySelector("input")).toBeNull();
  expect(root.textContent).not.toMatch(/Create skill|Create a skill|Import|Other sources|Choose skill|Install in/);
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
