/**
 * Split a markdown body into the user's visible message and the quoted
 * reply history below it.
 *
 * Heuristic (single pass):
 *   1. Look for the first "On … wrote:" line followed (allowing one blank
 *      line) by a line starting with `>`. Split there.
 *   2. If no match, look for a run of >=3 consecutive `>`-prefixed lines
 *      and split at its start.
 *   3. If still nothing, return { visible: md, quoted: null } — no toggle.
 *
 * English-only for v1 — locale variants can be added later.
 */
export function detectQuotedBlock(md: string): {
  visible: string;
  quoted: string | null;
} {
  if (!md) return { visible: md, quoted: null };

  const lines = md.split("\n");
  const wrotePreambleRe = /^On .+ wrote:\s*$/i;

  // Pass 1: "On … wrote:" preamble.
  for (let i = 0; i < lines.length; i++) {
    if (!wrotePreambleRe.test(lines[i])) continue;
    // Next non-blank line must be quoted.
    let j = i + 1;
    if (j < lines.length && lines[j].trim() === "") j++;
    if (j < lines.length && lines[j].startsWith(">")) {
      return splitAt(lines, i);
    }
  }

  // Pass 2: a run of >=3 consecutive quoted lines.
  let runStart = -1;
  let runLen = 0;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith(">")) {
      if (runLen === 0) runStart = i;
      runLen++;
      if (runLen >= 3) return splitAt(lines, runStart);
    } else {
      runLen = 0;
    }
  }

  return { visible: md, quoted: null };
}

function splitAt(
  lines: string[],
  at: number,
): { visible: string; quoted: string } {
  // Trim trailing blank lines from visible so the toggle button doesn't
  // float away from the message.
  let visEnd = at;
  while (visEnd > 0 && lines[visEnd - 1].trim() === "") visEnd--;
  return {
    visible: lines.slice(0, visEnd).join("\n"),
    quoted: lines.slice(at).join("\n"),
  };
}

/** Number of non-empty lines in the quoted block — for the toggle label. */
export function countQuotedLines(quoted: string): number {
  return quoted.split("\n").filter((l) => l.trim() !== "").length;
}
