import type { ProcessContext } from "./process-types";

export type AgentRunStatus = "queued" | "running" | "waiting_approval" | "cancelling" | "completed" | "failed" | "cancelled" | "timed_out";
export interface AgentRunView {
  run_id: string;
  agent: string;
  task: string;
  context_mode: "fork" | "task_only";
  execution_mode: "wait" | "parallel";
  status: AgentRunStatus;
  tools: string[];
  activity?: {tool: string; success: boolean | null; summary: string}[];
  progress: string | null;
  current_tool: string | null;
  started_at: string;
  duration_ms: number;
  tool_calls: number;
  pending_approval?: {id: string; tool: string; arguments: string; reason: string} | null;
  model?: string | null;
  result: {
    outcome: "done" | "partial" | "blocked";
    summary: string;
    work_done: string[];
    findings: unknown[];
    artifacts: string[];
    verification: unknown[];
    remaining: string[];
  } | null;
  error: string | null;
}
export interface AgentRunsApi {
  listAgentRuns?(context: ProcessContext): Promise<AgentRunView[]>;
  resolveAgentRunApproval?(context: ProcessContext, runId: string, approvalId: string, allow: boolean): Promise<void>;
  cancelAgentRun?(context: ProcessContext, runId: string): Promise<AgentRunView>;
}
