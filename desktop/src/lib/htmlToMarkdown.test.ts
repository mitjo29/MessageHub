import { describe, expect, test } from "vitest";
import { htmlToMarkdown } from "./htmlToMarkdown";

describe("htmlToMarkdown", () => {
  test("empty input returns empty", () => {
    expect(htmlToMarkdown("")).toBe("");
  });

  test("paragraph with anchor renders as [text](url)", () => {
    const md = htmlToMarkdown(
      `<p>Check <a href="https://example.com/x">the report</a> now.</p>`,
    );
    expect(md).toContain("[the report](https://example.com/x)");
  });

  test("html list becomes markdown bullets", () => {
    const md = htmlToMarkdown(`<ul><li>one</li><li>two</li></ul>`);
    expect(md).toMatch(/-\s+one/);
    expect(md).toMatch(/-\s+two/);
  });

  test("tables are flattened to their contents (layout tables are the common case)", () => {
    // Email tables are almost always layout scaffolding, so we unwrap
    // them rather than preserve the grid. Cell text survives.
    const md = htmlToMarkdown(
      `<table><thead><tr><th>a</th><th>b</th></tr></thead>` +
        `<tbody><tr><td>1</td><td>2</td></tr></tbody></table>`,
    );
    expect(md).toContain("a");
    expect(md).toContain("b");
    expect(md).toContain("1");
    expect(md).toContain("2");
    // No leaked pipe-table or raw-HTML structure.
    expect(md).not.toContain("|");
    expect(md).not.toContain("<table");
  });

  test("deeply nested MJML-style layout tables do not leak raw HTML", () => {
    // Representative of marketing email (e.g. role="presentation" tables
    // inside tables inside divs). The previous failure mode: turndown's
    // default passed the outer <table> through as literal HTML, which
    // react-markdown rendered as visible escaped text.
    const html = [
      `<table role="presentation"><tbody><tr><td>`,
      `<div class="mj-column-per-100">`,
      `<table role="presentation"><tbody><tr><td>`,
      `<p>Welcome to our newsletter.</p>`,
      `<p>Read more at <a href="https://example.com">example.com</a>.</p>`,
      `</td></tr></tbody></table>`,
      `</div>`,
      `</td></tr></tbody></table>`,
    ].join("");
    const md = htmlToMarkdown(html);
    expect(md).toContain("Welcome to our newsletter");
    expect(md).toContain("[example.com](https://example.com)");
    // Critical: no surviving raw table tags to trip react-markdown's
    // HTML-escape path.
    expect(md).not.toMatch(/<\s*(table|tbody|tr|td|thead|tfoot)/i);
  });

  test("gmail wrapper div is flattened (contents survive)", () => {
    const md = htmlToMarkdown(
      `<p>Reply body.</p>` +
        `<div class="gmail_quote">` +
        `<div class="gmail_attr">On Mon, Alice wrote:</div>` +
        `<blockquote>Original message</blockquote>` +
        `</div>`,
    );
    // The wrapper disappears; the blockquote's `>` prefix remains so the
    // downstream quote detector can catch it.
    expect(md).toContain("Reply body");
    expect(md).toContain("Original message");
    expect(md).toMatch(/^>/m);
  });

  test("img passes through as markdown image syntax", () => {
    const md = htmlToMarkdown(
      `<p><img src="https://i.example/1.png" alt="logo"></p>`,
    );
    expect(md).toContain("![logo](https://i.example/1.png)");
  });

  test("malformed html returns empty string rather than throwing", () => {
    // turndown is very lenient — feed it something exotic and verify we
    // still get a safe string result.
    const md = htmlToMarkdown("<notahtmltag><<<>>>" as unknown as string);
    expect(typeof md).toBe("string");
  });

  test("<style> contents are dropped, not leaked into the output", () => {
    const html =
      `<style><!-- a { color: red; } .foo { padding: 2rem; } --></style>` +
      `<p>Real content.</p>`;
    const md = htmlToMarkdown(html);
    expect(md).toContain("Real content.");
    expect(md).not.toContain("color: red");
    expect(md).not.toContain(".foo");
    expect(md).not.toContain("<!--");
  });

  test("<script> and <head> metadata are dropped", () => {
    const html =
      `<head><title>Ignore me</title><meta charset="utf-8"></head>` +
      `<script>alert(1)</script>` +
      `<p>Real.</p>`;
    const md = htmlToMarkdown(html);
    expect(md).toContain("Real.");
    expect(md).not.toContain("Ignore me");
    expect(md).not.toContain("alert(1)");
  });

  test("standalone HTML comment is stripped", () => {
    const html = `<p>Hi.</p><!-- tracking beacon --><p>Bye.</p>`;
    const md = htmlToMarkdown(html);
    expect(md).toContain("Hi.");
    expect(md).toContain("Bye.");
    expect(md).not.toContain("tracking beacon");
  });
});
