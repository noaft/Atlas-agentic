import DOMPurify from "dompurify";
import { marked } from "marked";

const allowedTags = [
  "a",
  "blockquote",
  "br",
  "code",
  "del",
  "em",
  "h1",
  "h2",
  "h3",
  "h4",
  "hr",
  "li",
  "ol",
  "p",
  "pre",
  "strong",
  "table",
  "tbody",
  "td",
  "th",
  "thead",
  "tr",
  "ul",
];

export function renderMarkdown(container: HTMLElement, source: string): void {
  const html = marked.parse(source, { async: false, gfm: true });
  container.innerHTML = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: allowedTags,
    ALLOWED_ATTR: ["href", "title"],
  });
  for (const link of container.querySelectorAll<HTMLAnchorElement>("a[href]")) {
    link.target = "_blank";
    link.rel = "noopener noreferrer";
  }
}
