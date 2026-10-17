import type { ChatState, TaskDescriptor } from "./types";

export function filterTasks(tasks: TaskDescriptor[], query: string): TaskDescriptor[] {
  const search = query.toLowerCase();
  return tasks.filter(task => [task.command, task.title, task.description].some(text => text.toLowerCase().includes(search)));
}

export type TaskAvailabilityState = Pick<ChatState, "activeConversationId" | "settings"> &
  Partial<Pick<ChatState, "status" | "historyLoaded" | "workspaceBusy" | "permissionBusy" | "modelListStatus">>;

export function taskUnavailableReason(
  task: TaskDescriptor,
  state: TaskAvailabilityState,
): string | null {
  if (state.status === "streaming" || state.status === "loading" || state.historyLoaded === false ||
      state.workspaceBusy || state.permissionBusy || state.modelListStatus === "saving") {
    return "This chat is busy. Wait for it to finish.";
  }
  if (task.requiresConversation && !state.activeConversationId) return "Start a saved conversation first";
  if (task.requiresModel && (!state.settings || state.settings.apiKeySource === "missing" || !state.settings.activeModel)) return "Configure a model first";
  return null;
}
