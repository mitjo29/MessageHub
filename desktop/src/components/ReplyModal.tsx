import { useEffect, useRef, useState } from "react";
import {
  deleteReplyDraft,
  getReplyDraft,
  saveReplyDraft,
  sendEmailReply,
} from "../api";
import { api } from "../api";
import { useAutosave } from "../hooks/useAutosave";
import { AiAssistPanel } from "./AiAssistPanel";

interface Props {
  messageId: string;
  threadId: string;
  onClose: () => void;
}

/**
 * Build the quoted-original block inserted below the cursor on a blank
 * compose. Two leading newlines separate the user's reply from the quote.
 */
function quotedOriginal(
  senderName: string,
  timestamp: string,
  body: string,
): string {
  const when = new Date(timestamp).toLocaleString();
  const quoted = body
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  return `\n\n> On ${when}, ${senderName} wrote:\n${quoted}\n`;
}

function ensureRePrefix(subject: string | null | undefined): string {
  const s = (subject ?? "").trim();
  if (!s) return "Re:";
  return /^re:/i.test(s) ? s : `Re: ${s}`;
}

export function ReplyModal({ messageId, threadId, onClose }: Props) {
  const [body, setBody] = useState<string>("");
  const [subject, setSubject] = useState<string>("Re:");
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  // Preserve the quoted-original block across AI Generate/Regenerate/Restore.
  // The AI returns only the user-facing reply text (ai_drafts.output never
  // includes the quote), so we keep the block in a ref set at mount time and
  // re-append it whenever onDraftReady fires.
  const quotedOriginalRef = useRef<string>("");

  // Mount: hydrate from existing draft if any, else seed with quoted original.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [draft, msg] = await Promise.all([
          getReplyDraft(threadId),
          api.getMessage(messageId),
        ]);
        if (cancelled) return;
        setSubject(ensureRePrefix(msg.subject));
        const quoted = quotedOriginal(
          msg.sender_name,
          msg.timestamp,
          msg.body,
        );
        quotedOriginalRef.current = quoted;
        if (draft && draft.body.length > 0) {
          setBody(draft.body);
        } else {
          setBody(quoted);
        }
        setLoaded(true);
      } catch (err) {
        console.error("ReplyModal mount failed:", err);
        if (!cancelled) setSendError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [messageId, threadId]);

  useAutosave(body, 5000, async (value) => {
    if (!loaded) return;
    await saveReplyDraft(threadId, messageId, value, subject);
  });

  async function handleSend() {
    setSending(true);
    setSendError(null);
    try {
      await sendEmailReply(threadId, messageId, body, subject);
      onClose();
    } catch (err) {
      setSendError(typeof err === "string" ? err : String(err));
    } finally {
      setSending(false);
    }
  }

  async function handleDiscard() {
    try {
      await deleteReplyDraft(threadId);
    } catch (err) {
      console.error("deleteReplyDraft failed:", err);
    }
    onClose();
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void handleSend();
    }
  }

  // Esc to close (Cancel — keeps the draft row).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canSend = body.trim().length > 0 && !sending && loaded;

  return (
    <div className="reply-modal-backdrop" onClick={onClose}>
      <div
        className="reply-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Reply"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="reply-modal-compose">
          <div className="reply-modal-header">
            <input
              className="reply-modal-subject"
              type="text"
              value={subject}
              readOnly
              aria-label="Subject"
            />
          </div>
          {sendError != null && (
            <div className="reply-modal-error" role="alert">
              <span>{sendError}</span>
              <button
                className="reply-modal-error-dismiss"
                onClick={() => setSendError(null)}
                aria-label="Dismiss error"
              >
                ✕
              </button>
            </div>
          )}
          <textarea
            className="reply-modal-body"
            value={body}
            onChange={(e) => setBody(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={loaded ? "Write your reply..." : "Loading..."}
            autoFocus
            disabled={!loaded}
          />
          <div className="reply-modal-actions">
            <button
              className="reply-modal-send"
              onClick={handleSend}
              disabled={!canSend}
            >
              {sending ? "Sending..." : "Send"}
            </button>
            <button className="reply-modal-discard" onClick={handleDiscard}>
              Discard
            </button>
            <button className="reply-modal-cancel" onClick={onClose}>
              Cancel
            </button>
          </div>
        </div>
        <div className="reply-modal-aside">
          <AiAssistPanel
            messageId={messageId}
            onDraftReady={(text, _conf) => {
              // Preserve the quoted original below the AI/Restored text
              // so context isn't lost on Generate/Regenerate/Restore.
              const combined = text + quotedOriginalRef.current;
              setBody(combined);
              void saveReplyDraft(threadId, messageId, combined, subject);
            }}
          />
        </div>
      </div>
    </div>
  );
}
