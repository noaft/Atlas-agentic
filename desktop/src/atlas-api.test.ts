import { afterEach, describe, expect, test } from "vitest";
import { Channel } from "@tauri-apps/api/core";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import { createTauriAtlasApi } from "./atlas-api";
import type {
  ChatEvent,
  Conversation,
  ConversationAskResult,
  ConversationMetadata,
} from "./types";

const metadata: ConversationMetadata = {
  id: "00000000-0000-4000-8000-000000000001",
  title: "Desktop chat",
  titleAttempted: true,
  createdAt: "2026-10-04T10:00:00Z",
  updatedAt: "2026-10-04T10:00:01Z",
  messageCount: 2,
  workspace: { path: "/work/atlas", name: "atlas" },
  permissionMode: "ask",
};

const conversation: Conversation = {
  metadata,
  messages: [
    { role: "user", content: "hello", createdAt: metadata.createdAt },
    { role: "assistant", content: "hi", createdAt: metadata.updatedAt },
  ],
};

afterEach(() => clearMocks());

describe("Tauri Atlas API", () => {
  test("uses the exact CRUD command names and camelCase payloads", async () => {
    const calls: Array<{ command: string; payload: unknown }> = [];
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      switch (command) {
        case "list_conversations":
          return [metadata];
        case "load_conversation":
          return conversation;
        case "rename_conversation":
          return { ...metadata, title: "Renamed" };
        case "delete_conversation":
          return { id: metadata.id };
        default:
          throw new Error(`unexpected command: ${command}`);
      }
    });
    const api = createTauriAtlasApi();

    await api.listConversations();
    await api.loadConversation(metadata.id);
    await api.renameConversation(metadata.id, "Renamed");
    await api.deleteConversation(metadata.id);

    expect(calls).toEqual([
      { command: "list_conversations", payload: {} },
      { command: "load_conversation", payload: { id: metadata.id } },
      {
        command: "rename_conversation",
        payload: { id: metadata.id, title: "Renamed" },
      },
      { command: "delete_conversation", payload: { id: metadata.id } },
    ]);
  });

  test("delivers ordered ask channel deltas and returns metadata", async () => {
    const result: ConversationAskResult = {
      answer: "Hello desktop",
      metadata,
      created: true,
    };
    mockIPC((command, payload) => {
      expect(command).toBe("ask");
      const args = payload as {
        prompt: string;
        conversationId: string | null;
        workspacePath: string | null;
        channel: Channel<ChatEvent>;
      };
      expect(args.prompt).toBe("hello");
      expect(args.conversationId).toBeNull();
      expect(args.workspacePath).toBe("/work/atlas");
      expect((args as typeof args & { permissionMode: string }).permissionMode).toBe("ask");
      args.channel.onmessage({ event: "delta", data: { content: "Hello" } });
      args.channel.onmessage({ event: "delta", data: { content: " desktop" } });
      return result;
    });
    const deltas: string[] = [];

    const completed = await createTauriAtlasApi().ask(
      "hello",
      null,
      "/work/atlas",
      "ask",
      (event) => {
        if (event.event === "delta") deltas.push(event.data.content);
      },
    );

    expect(deltas).toEqual(["Hello", " desktop"]);
    expect(completed).toEqual(result);
  });

  test("sends the permission mode and resolves streamed tool approvals", async () => {
    const commands: Array<{ command: string; payload: unknown }> = [];
    mockIPC((command, payload) => {
      commands.push({ command, payload });
      if (command === "ask") {
        const args = payload as { channel: Channel<ChatEvent> };
        args.channel.onmessage({
          event: "approvalRequested",
          data: {
            sequence: 0,
            request: {
              id: "approval-1",
              tool: "delete_file",
              arguments: '{"path":"note.txt"}',
              reason: "deletes or overwrites data",
            },
          },
        });
        return { answer: "Done", metadata, created: true };
      }
      if (command === "resolve_tool_approval") return null;
      throw new Error(`unexpected command: ${command}`);
    });
    const api = createTauriAtlasApi();

    await api.ask("clean up", null, "/work/atlas", "workspace", async (event) => {
      if (event.event === "approvalRequested") {
        await api.resolveToolApproval(event.data.request.id, false);
      }
    });

    expect(commands[0]).toMatchObject({
      command: "ask",
      payload: { permissionMode: "workspace" },
    });
    expect(commands[1]).toEqual({
      command: "resolve_tool_approval",
      payload: { id: "approval-1", allow: false },
    });
  });

  test("uses exact settings and workspace commands and treats dialog cancellation as null", async () => {
    const calls: Array<{ command: string; payload: unknown }> = [];
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      if (command === "plugin:dialog|open") return null;
      if (command === "list_models") {
        return [{ id: "deepseek-flash", name: "DeepSeek Flash" }];
      }
      if (command === "load_settings" || command === "save_settings" || command === "set_model") {
        return {
          apiKeySource: "missing",
          baseUrl: "https://api.deepseek.com",
          model: "deepseek-flash",
          activeModel: "deepseek-flash",
          historyLimit: 40,
          defaultWorkspace: null,
          defaultWorkspaceAutomatic: true,
          defaultWorkspaceError: "automatic workspace unavailable",
          defaultPermissionMode: "ask",
          environmentOverrides: [],
        };
      }
      if (command === "validate_workspace") return metadata.workspace;
      if (command === "assign_conversation_workspace") return metadata;
      throw new Error(`unexpected command: ${command}`);
    });
    const api = createTauriAtlasApi();
    const input = {
      apiKey: null,
      clearApiKey: false,
      baseUrl: "https://api.deepseek.com",
      model: "deepseek-flash",
      historyLimit: 40,
      defaultWorkspacePath: null,
      defaultPermissionMode: "ask" as const,
    };

    await api.loadSettings();
    await api.saveSettings(input);
    expect(await api.listModels()).toEqual([
      { id: "deepseek-flash", name: "DeepSeek Flash" },
    ]);
    await api.setModel("deepseek-v4-pro");
    await api.validateWorkspace("/work/atlas");
    await api.assignConversationWorkspace(metadata.id, "/work/atlas");
    expect(await api.chooseWorkspace()).toBeNull();

    expect(calls.slice(0, 6)).toEqual([
      { command: "load_settings", payload: {} },
      { command: "save_settings", payload: { input } },
      { command: "list_models", payload: {} },
      { command: "set_model", payload: { model: "deepseek-v4-pro" } },
      { command: "validate_workspace", payload: { path: "/work/atlas" } },
      {
        command: "assign_conversation_workspace",
        payload: { id: metadata.id, path: "/work/atlas" },
      },
    ]);
    expect(calls[6]?.command).toBe("plugin:dialog|open");
  });

  test("toggles_the_current_window_fullscreen_state", async () => {
    mockWindows("main");
    let fullscreen = false;
    const calls: Array<{ command: string; payload: unknown }> = [];
    mockIPC((command, payload) => {
      calls.push({ command, payload });
      if (command === "plugin:window|is_fullscreen") return fullscreen;
      if (command === "plugin:window|set_fullscreen") {
        fullscreen = (payload as { value: boolean }).value;
        return null;
      }
      throw new Error(`unexpected command: ${command}`);
    });
    const api = createTauriAtlasApi();

    expect(await api.toggleFullscreen()).toBe(true);
    expect(await api.toggleFullscreen()).toBe(false);

    expect(calls.map(({ command }) => command)).toEqual([
      "plugin:window|is_fullscreen",
      "plugin:window|set_fullscreen",
      "plugin:window|is_fullscreen",
      "plugin:window|set_fullscreen",
    ]);
    expect(calls[1]?.payload).toMatchObject({ value: true });
    expect(calls[3]?.payload).toMatchObject({ value: false });
  });
});

test("compact sends the saved chat ID and forwards progress without an ask", async () => {
 const events: ChatEvent[] = [];
 const calls: string[] = [];
 mockIPC((command, payload) => {
  calls.push(command);
  const args = payload as { id: string; channel?: Channel<ChatEvent> };
  expect(args.id).toBe(metadata.id);
  if (command === "compact_conversation") {
   args.channel!.onmessage({ event: "compaction", data: { progress: { completedBlocks: 1, totalBlocks: 2 } } });
   return { memory: null, message: "done" };
  }
 });
 const api = createTauriAtlasApi();
 expect((await api.compactConversation(metadata.id, (event) => events.push(event))).message).toBe("done");
 await api.cancelCompaction(metadata.id);
 expect(calls).toEqual(["compact_conversation", "cancel_compaction"]);
 expect(events).toHaveLength(1);
});

test("task catalog and execution use validated request envelope and event channel", async () => {
  mockIPC((command, payload) => {
    if (command === "list_tasks") return [];
    expect(command).toBe("run_task");
    const args = payload as {request: unknown; channel: Channel<ChatEvent>};
    expect(args.request).toEqual({taskId: "conversation.compact", arguments: {}, conversationId: metadata.id});
    args.channel.onmessage({event: "compaction", data: {progress: {completedBlocks: 1, totalBlocks: 1}}});
    return {taskId: "conversation.compact", outcome: {kind: "compaction", result: {memory: null, message: "Done"}}};
  });
  const api = createTauriAtlasApi();
  expect(await api.listTasks()).toEqual([]);
  const events: ChatEvent[] = [];
  const result = await api.runTask("conversation.compact", {}, metadata.id, event => events.push(event));
  expect(result.outcome.kind).toBe("compaction");
  expect(events).toHaveLength(1);
});


test("MCP commands expose status and server names without credentials", async () => {
  const calls: Array<{command: string; payload: unknown}> = [];
  mockIPC((command, payload) => { calls.push({command,payload}); return command === "list_mcp_connections" ? [] : undefined; });
  const api = createTauriAtlasApi();
  expect(await api.listMcpConnections()).toEqual([]);
  await api.connectMcpServer("server"); await api.cancelMcpConnection("server"); await api.disconnectMcpServer("server");
  expect(calls).toEqual([
    {command:"list_mcp_connections",payload:{}},
    {command:"connect_mcp_server",payload:{server:"server"}},
    {command:"cancel_mcp_connection",payload:{server:"server"}},
    {command:"disconnect_mcp_server",payload:{server:"server"}},
  ]);
});

test("skill commands preserve explicit workspace ownership and pinned sources", async () => {
  const calls: Array<{command:string,payload:unknown}>=[];
  mockIPC((command,payload)=>{calls.push({command,payload});return command === "list_skills" ? {skills:[],issues:[]} : null;});
  const api=createTauriAtlasApi();
  const context={conversationId:metadata.id,workspacePath:"/work/draft"};
  const source={repository:"owner/repo",reference:"main",revision:"a".repeat(40),path:"skills/review"};
  await api.listSkills(context);await api.readSkill("project:review",context);
  await api.previewSkillImport("https://github.com/owner/repo");
  await api.installSkill(source,"project",false,context);await api.setSkillEnabled("personal:review",false,context);
  expect(calls).toEqual([
    {command:"list_skills",payload:{context}},
    {command:"read_skill",payload:{id:"project:review",context}},
    {command:"preview_skill_import",payload:{url:"https://github.com/owner/repo"}},
    {command:"install_skill",payload:{source,scope:"project",replace:false,context}},
    {command:"set_skill_enabled",payload:{id:"personal:review",enabled:false,context}},
  ]);
});

test("local skill creation, editing and import retain explicit scope and ownership", async () => {
  const calls: Array<{command:string,payload:unknown}>=[];
  mockIPC((command,payload)=>{calls.push({command,payload});return command === "plugin:dialog|open" ? "/imports/review" : null;});
  const api=createTauriAtlasApi();
  const context={conversationId:metadata.id,workspacePath:"/work/draft"};
  const input={name:"review",description:"Review changes",instructions:"Inspect first."};
  await api.createSkill(input,"personal",context);
  await api.updateSkillDocument("project:review","new content","fingerprint",context);
  expect(await api.chooseSkillImport("folder")).toBe("/imports/review");
  await api.previewLocalSkills("/imports/review");
  await api.installLocalSkill("/imports/review","fingerprint","project",context);
  expect(calls).toEqual([
    {command:"create_skill",payload:{input,scope:"personal",context}},
    {command:"update_skill_document",payload:{id:"project:review",content:"new content",fingerprint:"fingerprint",context}},
    {command:"plugin:dialog|open",payload:{options:{directory:true,multiple:false}}},
    {command:"preview_local_skills",payload:{path:"/imports/review"}},
    {command:"install_local_skill",payload:{path:"/imports/review",fingerprint:"fingerprint",scope:"project",context}},
  ]);
});
