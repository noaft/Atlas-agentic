import type { MemoryCheckpoint } from "./types";
export function renderMemory(memory: MemoryCheckpoint): HTMLElement {
  const details = document.createElement("details");
  details.className = "memory-divider";
  details.dataset.memoryCheckpoint = memory.checkpointId;
  const summary = document.createElement("summary");
  summary.textContent = "✦ Earlier context summarized · View memory";
  const meta = document.createElement("p");
  meta.textContent = `Messages 1–${memory.compactedMessageCount} · ${memory.model} · ${new Date(memory.createdAt).toLocaleString()}`;
  details.append(summary, meta);
  memory.blocks.forEach((block, index) => {
    const section = document.createElement("section");
    section.dataset.memoryBlock = "";
    const title = document.createElement("strong");
    title.textContent = `Memory ${index + 1} · Messages ${block.startIndex + 1}–${block.endIndexExclusive}`;
    const content = document.createElement("p");
    content.textContent = block.content;
    section.append(title, content);
    details.append(section);
  });
  return details;
}
