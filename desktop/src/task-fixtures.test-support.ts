import type { TaskDescriptor } from "./types";
export const taskFixtures: TaskDescriptor[] = [{
  id: "conversation.compact", command: "compact", title: "Compact conversation",
  description: "Summarize older context while keeping the latest messages.",
  argumentSchema: {type: "object", properties: {}, additionalProperties: false},
  requiresConversation: true, requiresModel: true, cancellable: true,
}];
