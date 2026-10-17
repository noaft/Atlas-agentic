import { expect, test } from "vitest";
import { renderMemory } from "./memory-view";
test("memory details expose the original range and separate summary contents safely", () => {
 const memory = { version:1,conversationId:"chat",checkpointId:"cp",createdAt:"2026-10-17T00:00:00Z",model:"fake",compactedMessageCount:80,blocks:[{startIndex:0,endIndexExclusive:40,content:"<script>bad</script>"},{startIndex:40,endIndexExclusive:80,content:"second"}] };
 const element=renderMemory(memory);
 expect(element.textContent).toContain("Messages 1–80");
 expect(element.querySelectorAll("[data-memory-block]")).toHaveLength(2);
 expect(element.querySelector("script")).toBeNull();
});
