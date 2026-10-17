import { expect, test } from "vitest";
import { parseSlashCommand, filterTasks, taskUnavailableReason, taskArguments } from "./slash-commands";
import type { TaskDescriptor } from "./types";
const task: TaskDescriptor = { id: "conversation.compact", command: "compact", title: "Compact conversation", description: "Summarize older context", argumentSchema: {type: "object", properties: {}, additionalProperties: false}, requiresConversation: true, requiresModel: true, cancellable: true };
test("only first slash-name tokens are commands, preserving paths and examples", () => {
  expect(parseSlashCommand("/CoMpAcT extra")).toEqual({name: "compact", argumentsText: "extra"});
  expect(parseSlashCommand("/")).toEqual({name: "", argumentsText: ""});
  for (const text of ["/home/user/file.rs", "explain /compact", "`/compact`", '"/compact"', "https://site/a", "/tmp/file"]) expect(parseSlashCommand(text)).toBeNull();
});
test("catalog drives filtering, availability and argument handling", () => {
  expect(filterTasks([task], "SUMMARIZE")).toEqual([task]);
  expect(filterTasks([task], "missing")).toEqual([]);
  expect(taskUnavailableReason(task, {activeConversationId: null, settings: null})).toMatch(/saved conversation/);
  expect(taskUnavailableReason(task, {activeConversationId: "saved", settings: null})).toMatch(/model/);
  expect(taskArguments(task, "")).toEqual({});
  expect(() => taskArguments(task, "extra")).toThrow(/arguments/);
});
