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
  fingerprint: string;
}

export interface SkillDraftInput {
  name: string;
  title?: string;
  description: string;
  instructions: string;
  files?: Record<string, string>;
}

export interface LocalSkillPreview {
  name: string;
  title: string;
  description: string;
  content: string;
  files: string[];
  path: string;
  fingerprint: string;
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
  installSkills(sources: SkillSource[], scope: SkillScope, context: SkillWorkspaceContext): Promise<SkillInstallResult[]>;
  searchSkills(query: string, scope: SkillScope, context: SkillWorkspaceContext): Promise<SkillSearchResult[]>;
  createSkill(input: SkillDraftInput, scope: SkillScope, context: SkillWorkspaceContext): Promise<SkillInfo>;
  updateSkillDocument(id: string, content: string, fingerprint: string, context: SkillWorkspaceContext): Promise<SkillInfo>;
  chooseSkillImport(kind: "file" | "folder"): Promise<string | null>;
  previewLocalSkills(path: string): Promise<LocalSkillPreview[]>;
  installLocalSkill(path: string, fingerprint: string, scope: SkillScope, context: SkillWorkspaceContext): Promise<SkillInfo>;
  listSkills(context: SkillWorkspaceContext): Promise<SkillCatalog>;
  readSkill(id: string, context: SkillWorkspaceContext): Promise<SkillDocument>;
  previewSkillImport(url: string): Promise<SkillPreview[]>;
  installSkill(source: SkillSource, scope: SkillScope, replace: boolean, context: SkillWorkspaceContext): Promise<SkillInfo>;
  setSkillEnabled(id: string, enabled: boolean, context: SkillWorkspaceContext): Promise<SkillInfo>;
}

export interface SkillInstallResult {
  source: SkillSource;
  skill: SkillInfo | null;
  error: string | null;
}
export interface SkillSearchResult {
  skill: SkillInfo;
  score: number;
}
