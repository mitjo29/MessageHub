import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { htmlToMarkdown } from "../lib/htmlToMarkdown";
import {
  countQuotedLines,
  detectQuotedBlock,
} from "../lib/detectQuotedBlock";
import type { MessageDetail } from "../types";

type Props = { detail: MessageDetail };

const MAX_AUTOLINK_TEXT = 60;

/**
 * Dispatches on `detail.html`:
 *   - non-empty → HTML→markdown via turndown
 *   - else      → use `detail.body` verbatim (markdown autolinks bare URLs)
 *   - both empty → "(empty body)"
 *
 * Images are hidden until the user clicks "Load all"; quoted reply history
 * is collapsed behind a toggle. State is per-message — switching messages
 * resets both toggles.
 */
export function MessageBody({ detail }: Props) {
  const [loadImages, setLoadImages] = useState(false);
  const [showQuoted, setShowQuoted] = useState(false);

  useEffect(() => {
    setLoadImages(false);
    setShowQuoted(false);
  }, [detail.id]);

  const { visible, quoted } = useMemo(() => {
    const md = sourceMarkdown(detail);
    return detectQuotedBlock(md);
  }, [detail.id, detail.html, detail.body]);

  if (!visible && !quoted) {
    return <div className="message-body empty">(empty body)</div>;
  }

  const components: Components = {
    img: ({ src, alt }) => {
      const href = typeof src === "string" ? src : "";
      const isInline = href.startsWith("data:");
      if (loadImages || isInline) {
        return <img src={href} alt={alt ?? ""} loading="lazy" />;
      }
      return (
        <span className="message-body-image-placeholder">
          <span className="message-body-image-alt">
            {alt ? `🖼 ${alt}` : "🖼 image hidden"}
          </span>
          <button
            type="button"
            className="message-body-load-images"
            onClick={() => setLoadImages(true)}
          >
            Load all
          </button>
        </span>
      );
    },
    a: ({ href, children }) => {
      const url = href ?? "";
      // When react-markdown autolinks a bare URL, the text node it emits
      // equals the href. In that case, keep the href intact but truncate
      // the visible text for readability.
      const flatText = flattenText(children);
      const isAutolinked = flatText === url;
      const visibleText =
        isAutolinked && flatText.length > MAX_AUTOLINK_TEXT
          ? flatText.slice(0, MAX_AUTOLINK_TEXT - 1) + "…"
          : children;
      return (
        <a
          href={url}
          target="_blank"
          rel="noopener noreferrer"
          title={url}
        >
          {visibleText}
        </a>
      );
    },
  };

  return (
    <div className="message-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks]}
        components={components}
      >
        {visible}
      </ReactMarkdown>

      {quoted && (
        <>
          <button
            type="button"
            className="message-body-quote-toggle"
            onClick={() => setShowQuoted((v) => !v)}
          >
            {showQuoted
              ? "Hide trimmed content"
              : `Show trimmed content (${countQuotedLines(quoted)} lines hidden)`}
          </button>
          {showQuoted && (
            <div className="message-body-quoted">
              <ReactMarkdown
                remarkPlugins={[remarkGfm, remarkBreaks]}
                components={components}
              >
                {quoted}
              </ReactMarkdown>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function sourceMarkdown(detail: MessageDetail): string {
  if (detail.html) {
    const md = htmlToMarkdown(detail.html);
    if (md.trim()) return md;
  }
  const body = detail.body ?? "";
  // Some senders put raw HTML in the text/plain alternative. If we see
  // tell-tale tags, route it through turndown so <style>/<!-- --> junk
  // is stripped rather than rendered verbatim.
  if (looksLikeHtml(body)) {
    const md = htmlToMarkdown(body);
    if (md.trim()) return md;
  }
  return body;
}

function looksLikeHtml(s: string): boolean {
  return (
    /<\s*(html|body|head|style|script|div|p|table|a\s|img\s|br\s*\/?>|!--)/i.test(
      s,
    )
  );
}

/** Recursively flatten React children into a plain string for autolink compare. */
function flattenText(node: React.ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(flattenText).join("");
  if (typeof node === "object" && "props" in node) {
    const props = (node as { props?: { children?: React.ReactNode } }).props;
    return flattenText(props?.children);
  }
  return "";
}
