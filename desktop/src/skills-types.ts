export type SkillScope = "personal" | "project";

export interface SkillSource {
  repository: string;
  reference: string;
  revision: string;
  path: string;
}

export interface SkillInfo {
  id: string;
  name: string;
  title: string;
  description: string;
  scope: SkillScope;
  enabled: boolean;
  source: SkillSource | null;
}

export interface SkillCatalog {
  skills: SkillInfo[];
  issues: string[];
}

export interface SkillDocument {
  skill: SkillInfo;
  content: string;
  files: string[];
}

export interface SkillPreview {
  source: SkillSource;
  name: string;
  title: string;
  description: string;
  content: string;
  files: string[];
}

export interface SkillWorkspaceContext {
  conversationId: string | null;
  workspacePath: string | null;
}

export interface SkillsApi {
  listSkills(context: SkillWorkspaceContext): Promise<SkillCatalog>;
  readSkill(id: string, context: SkillWorkspaceContext): Promise<SkillDocument>;
  previewSkillImport(url: string): Promise<SkillPreview[]>;
  installSkill(source: SkillSource, scope: SkillScope, replace: boolean, context: SkillWorkspaceContext): Promise<SkillInfo>;
  setSkillEnabled(id: string, enabled: boolean, context: SkillWorkspaceContext): Promise<SkillInfo>;
}
