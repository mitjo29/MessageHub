import { describe, expect, test } from "vitest";
import {
  countQuotedLines,
  detectQuotedBlock,
} from "./detectQuotedBlock";

describe("detectQuotedBlock", () => {
  test("gmail-style 'On … wrote:' preamble splits the body", () => {
    const md = [
      "My reply.",
      "",
      "On Mon, 24 Apr 2026 at 10:23, Alice <a@example.com> wrote:",
      "> Here's the draft.",
      "> Cheers, Alice",
    ].join("\n");
    const { visible, quoted } = detectQuotedBlock(md);
    expect(visible).toBe("My reply.");
    expect(quoted).not.toBeNull();
    expect(quoted!).toContain("Here's the draft.");
  });

  test("apple-mail style (already unwrapped to `>`) splits on run", () => {
    const md = [
      "Sure, see below.",
      "",
      "> Could you review this?",
      "> Thanks.",
      "> — Bob",
    ].join("\n");
    const { visible, quoted } = detectQuotedBlock(md);
    expect(visible).toBe("Sure, see below.");
    expect(quoted!).toContain("Could you review this?");
  });

  test("plain-text run of >=3 quoted lines triggers fallback", () => {
    const md = [
      "Hello.",
      "> one",
      "> two",
      "> three",
      "> four",
    ].join("\n");
    const { visible, quoted } = detectQuotedBlock(md);
    expect(visible).toBe("Hello.");
    expect(quoted!.split("\n").length).toBe(4);
  });

  test("multi-level nested quotes collapse from the outermost split point", () => {
    const md = [
      "Got it.",
      "",
      "On Tue, Alice wrote:",
      "> Thanks.",
      "> > On Mon, Bob wrote:",
      "> > earlier.",
    ].join("\n");
    const { visible, quoted } = detectQuotedBlock(md);
    expect(visible).toBe("Got it.");
    expect(quoted!).toContain("> > On Mon, Bob wrote:");
  });

  test("no quote — returns visible == input, quoted null", () => {
    const md = "Just a plain reply with no history.";
    expect(detectQuotedBlock(md)).toEqual({ visible: md, quoted: null });
  });

  test("empty input — no crash", () => {
    expect(detectQuotedBlock("")).toEqual({ visible: "", quoted: null });
  });

  test("two isolated quoted lines do NOT trigger the >=3 run rule", () => {
    const md = ["Thanks for", "> your note", "> yesterday", "", "cheers"].join(
      "\n",
    );
    expect(detectQuotedBlock(md)).toEqual({ visible: md, quoted: null });
  });

  test("countQuotedLines ignores blank lines", () => {
    const quoted = "> a\n>\n> b\n\n> c";
    expect(countQuotedLines(quoted)).toBe(4); // three `>…` + `>` alone
  });
});
