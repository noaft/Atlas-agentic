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
    listSkills: vi.fn(async () => ({skills: [skill], issues: []})),
    readSkill: vi.fn(async () => ({skill, content: preview.content, files: preview.files})),
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
  let finishRead!: (value: {skill: SkillInfo; content: string; files: string[]}) => void;
  const project = {...skill, scope: "project" as const, id: "project:review"};
  const {api, panel, root, click} = fixture({
    listSkills: vi.fn().mockResolvedValueOnce({skills: [project], issues: []}).mockResolvedValueOnce({skills: [project], issues: []}).mockImplementationOnce(() => new Promise(resolve => {finishList = resolve;})).mockResolvedValue({skills: [], issues: []}),
    readSkill: vi.fn(() => new Promise<{skill: SkillInfo; content: string; files: string[]}>(resolve => {finishRead = resolve;})),
  });
  panel.updateContext(context, "a", true); await flush();
  const scope = root.querySelector<HTMLSelectElement>('[aria-label="Skill scope"]')!;
  scope.value = "project"; scope.dispatchEvent(new Event("change"));
  await flush();
  click("View instructions"); click("Refresh skills");
  panel.updateContext({...context, workspacePath: "/private/b"}, "b", true); await flush();
  finishList({skills: [project], issues: []});
  finishRead({skill: project, content: "PRIVATE PROJECT A", files: []}); await flush();
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
