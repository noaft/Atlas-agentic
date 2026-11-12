export interface ProcessContext { conversationId: string | null; scopeId: string | null }
export interface ProcessCursor { stdout: number; stderr: number }
export interface ProcessInfo {
  sessionId: string; kind: "process"; lifetime: "job" | "background";
  status: "starting" | "running" | "stopping" | "exited" | "stopped" | "timedOut" | "failed";
  running: boolean; workspacePath: string; startedAt: string; timeoutMs: number; exitCode: number | null;
}
export interface ProcessOutputView {
  start_byte: number; end_byte: number; total_bytes: number;
  start_line: number | null; end_line: number | null; total_lines: number;
  returned_lines: number; lines_after: number | null;
  starts_mid_line: boolean; older_output_dropped: boolean; has_more: boolean;
}
export interface ProcessOutput {
  session_id: string; running: boolean; stdout: string; stderr: string;
  stdout_truncated: boolean; stderr_truncated: boolean;
  next_cursor: ProcessCursor; oldest_cursor: ProcessCursor;
  tail_lines?: number | null; stdout_view?: ProcessOutputView; stderr_view?: ProcessOutputView;
  display_truncated?: boolean;
  exit_code: number | null; timed_out: boolean; stopped: boolean; stop_requested: boolean; error: string | null;
}
export interface ProcessesApi {
  openProcessScope(context: ProcessContext): Promise<{scopeId: string}>;
  releaseProcessScope(context: ProcessContext): Promise<void>;
  listProcesses(context: ProcessContext): Promise<ProcessInfo[]>;
  readProcessOutput(context: ProcessContext, sessionId: string, cursor: ProcessCursor): Promise<ProcessOutput>;
  stopProcess(context: ProcessContext, sessionId: string): Promise<ProcessOutput>;
}
