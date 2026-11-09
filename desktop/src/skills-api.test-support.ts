import type { ProcessesApi } from "./process-types";
import type { McpOnboardingApi, McpSettingsView } from "./mcp-types";
import type { SkillsApi, SkillDocument, SkillInfo } from "./skills-types";

/** Empty skill catalog for tests of unrelated chat/model behavior. */
export class EmptySkillsApi implements SkillsApi, McpOnboardingApi, ProcessesApi {
  openProcessScope: ProcessesApi["openProcessScope"] = async context => ({scopeId: context.scopeId ?? "test-scope"});
  releaseProcessScope: ProcessesApi["releaseProcessScope"] = async () => {};
  listProcesses: ProcessesApi["listProcesses"] = async () => [];
  readProcessOutput: ProcessesApi["readProcessOutput"] = async () => {throw new Error("No process in fixture");};
  stopProcess: ProcessesApi["stopProcess"] = async () => {throw new Error("No process in fixture");};
  loadMcpSettings = async (): Promise<McpSettingsView> => ({path: "/config/mcp.toml", fingerprint: "fixture", connections: []});
  addMcpServer: McpOnboardingApi["addMcpServer"] = async () => {throw new Error("unused");};
  inspectMcpServer: McpOnboardingApi["inspectMcpServer"] = async () => {throw new Error("unused");};
  resolveMcpInspectionApproval = async () => {};
  cancelMcpInspection = async () => {};
  installSkills = async () => [];
  searchSkills = async () => [];
  createSkill = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
  updateSkillDocument = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
  chooseSkillImport = async () => null;
  previewLocalSkills = async () => [];
  installLocalSkill = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
  listSkills = async () => ({skills: [], issues: []});
  previewSkillImport = async () => [];
  readSkill = async (): Promise<SkillDocument> => { throw new Error("No skill configured in this test"); };
  installSkill = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
  setSkillEnabled = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
}

export const emptySkillsApi = new EmptySkillsApi();
