import { expect, test } from "vitest";
import { filterTasks, taskUnavailableReason } from "./slash-commands";
import type { TaskDescriptor } from "./types";
const task: TaskDescriptor = { id: "conversation.compact", command: "compact", title: "Compact conversation", description: "Summarize older context", argumentSchema: {type: "object", properties: {}, additionalProperties: false}, requiresConversation: true, requiresModel: true, cancellable: true };
test("catalog drives filtering and availability", () => {
  expect(filterTasks([task], "SUMMARIZE")).toEqual([task]);
  expect(filterTasks([task], "missing")).toEqual([]);
  expect(taskUnavailableReason(task, {activeConversationId: null, settings: null})).toMatch(/saved conversation/);
  expect(taskUnavailableReason(task, {activeConversationId: "saved", settings: null})).toMatch(/model/);
});
