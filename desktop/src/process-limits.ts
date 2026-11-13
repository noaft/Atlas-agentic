export interface ProcessLimits { perChat: number; total: number }
export const DEFAULT_PROCESS_LIMITS: Readonly<ProcessLimits> = Object.freeze({perChat: 8, total: 32});
export const MAX_COMMANDS_PER_CHAT = 32;
export const MAX_COMMANDS_TOTAL = 128;

export function validProcessLimits(limits: ProcessLimits): boolean {
  return Number.isSafeInteger(limits.perChat) && limits.perChat >= 1 && limits.perChat <= MAX_COMMANDS_PER_CHAT
    && Number.isSafeInteger(limits.total) && limits.total >= 1 && limits.total <= MAX_COMMANDS_TOTAL;
}
