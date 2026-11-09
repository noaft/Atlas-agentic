export interface ProcessContext { conversationId: string | null; scopeId: string | null }
export interface ProcessCursor { stdout: number; stderr: number }
export interface ProcessInfo {
  sessionId: string; kind: "process"; lifetime: "job" | "background";
  status: "starting" | "running" | "stopping" | "exited" | "stopped" | "timedOut" | "failed";
  running: boolean; workspacePath: string; startedAt: string; timeoutMs: number; exitCode: number | null;
}
export interface ProcessOutput {
  session_id: string; running: boolean; stdout: string; stderr: string;
  stdout_truncated: boolean; stderr_truncated: boolean;
  next_cursor: ProcessCursor; oldest_cursor: ProcessCursor;
  exit_code: number | null; timed_out: boolean; stopped: boolean; stop_requested: boolean; error: string | null;
}
export interface ProcessesApi {
  openProcessScope(context: ProcessContext): Promise<{scopeId: string}>;
  releaseProcessScope(context: ProcessContext): Promise<void>;
  listProcesses(context: ProcessContext): Promise<ProcessInfo[]>;
  readProcessOutput(context: ProcessContext, sessionId: string, cursor: ProcessCursor): Promise<ProcessOutput>;
  stopProcess(context: ProcessContext, sessionId: string): Promise<ProcessOutput>;
}
