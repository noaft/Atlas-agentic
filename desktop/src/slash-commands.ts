import type { ChatState, TaskDescriptor } from "./types";

export function parseSlashCommand(text: string): {name: string; argumentsText: string} | null {
  const value = text.trimStart();
  if (value.trim() === "/") return {name: "", argumentsText: ""};
  const match = /^\/([A-Za-z][A-Za-z0-9_-]*)(?:\s+([\s\S]*))?$/.exec(value);
  return match ? {name: match[1].toLowerCase(), argumentsText: (match[2] ?? "").trim()} : null;
}

export function filterTasks(tasks: TaskDescriptor[], query: string): TaskDescriptor[] {
  const search = query.toLowerCase();
  return tasks.filter(task => [task.command, task.title, task.description].some(text => text.toLowerCase().includes(search)));
}

export function taskUnavailableReason(
  task: TaskDescriptor,
  state: Pick<ChatState, "activeConversationId" | "settings">,
): string | null {
  if (task.requiresConversation && !state.activeConversationId) return "Start a saved conversation first";
  if (task.requiresModel && (!state.settings || state.settings.apiKeySource === "missing" || !state.settings.activeModel)) return "Configure a model first";
  return null;
}

export function taskArguments(task: TaskDescriptor, text: string): Record<string, unknown> {
  const schema = task.argumentSchema;
  if (schema.type !== "object" || !schema.properties || Object.keys(schema.properties).length || schema.additionalProperties !== false) {
    throw new Error("This task's arguments are not supported in the slash menu yet.");
  }
  if (text.trim()) throw new Error(`/${task.command} does not accept arguments.`);
  return {};
}
