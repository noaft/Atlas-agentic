import type { SkillsApi, SkillDocument, SkillInfo } from "./skills-types";

/** Empty skill catalog for tests of unrelated chat/model behavior. */
export class EmptySkillsApi implements SkillsApi {
  listSkills = async () => ({skills: [], issues: []});
  previewSkillImport = async () => [];
  readSkill = async (): Promise<SkillDocument> => { throw new Error("No skill configured in this test"); };
  installSkill = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
  setSkillEnabled = async (): Promise<SkillInfo> => { throw new Error("No skill configured in this test"); };
}

export const emptySkillsApi: SkillsApi = new EmptySkillsApi();
