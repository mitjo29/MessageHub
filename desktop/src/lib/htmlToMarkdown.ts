import TurndownService from "turndown";
import { gfm } from "turndown-plugin-gfm";

let cached: TurndownService | null = null;

function buildService(): TurndownService {
  const td = new TurndownService({
    headingStyle: "atx",
    codeBlockStyle: "fenced",
    emDelimiter: "*",
    bulletListMarker: "-",
  });
  td.use(gfm);

  // Drop tags whose content is never meant for readers. Without these,
  // marketing emails leak their <style> CSS straight into the body
  // (including CSS hidden behind the <!-- … --> old-browser trick).
  td.remove([
    "style",
    "script",
    "head",
    "title",
    "meta",
    "link",
    "noscript",
  ]);

  // Strip HTML comments entirely. Turndown's default behavior keeps the
  // comment's text payload in some code paths.
  td.addRule("strip-comments", {
    filter: (node) => node.nodeType === 8 /* COMMENT_NODE */,
    replacement: () => "",
  });

  // Flatten Gmail wrapper divs so they don't inflate the markdown tree.
  // Content is rendered as if the wrapper weren't there — the quoted chunk
  // inside still surfaces its `>` lines for detectQuotedBlock to catch.
  td.addRule("gmail-wrappers", {
    filter: (node) =>
      node.nodeName === "DIV" &&
      typeof (node as HTMLElement).className === "string" &&
      /\b(gmail_quote|gmail_extra|gmail_attr)\b/.test(
        (node as HTMLElement).className,
      ),
    replacement: (content) => content,
  });

  return td;
}

export function htmlToMarkdown(html: string): string {
  if (!html) return "";
  try {
    if (!cached) cached = buildService();
    return cached.turndown(html);
  } catch {
    // Malformed HTML — caller falls back to plain text.
    return "";
  }
}
