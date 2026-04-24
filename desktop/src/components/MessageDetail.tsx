import { useEffect } from "react";
import { api } from "../api";
import { useInbox } from "../state/InboxContext";
import { MessageBody } from "./MessageBody";

function formatTime(isoString: string): string {
  const d = new Date(isoString);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function MessageDetail() {
  const { state, dispatch } = useInbox();
  const { detail } = state;

  useEffect(() => {
    if (!detail) return;
    if (detail.isRead) return;

    const id = detail.id;
    dispatch({ type: "MARK_READ_LOCAL", id });

    let cancelled = false;
    api.markRead(id, true).catch((err) => {
      if (cancelled) return;
      dispatch({ type: "REVERT_MARK_READ_LOCAL", id });
      dispatch({ type: "SET_ERROR", error: String(err) });
    });
    return () => {
      cancelled = true;
    };
  }, [detail?.id, detail?.isRead, dispatch]);

  if (!detail) {
    return (
      <div className="detail-pane empty">
        <p>Select a message.</p>
      </div>
    );
  }

  return (
    <div className="detail-pane">
      <div className="detail-head">
        <span className="channel">
          [{detail.channelLabel ?? detail.channel}]
        </span>
        <span className="sender">{detail.senderName}</span>
        <span className="time">{formatTime(detail.timestamp)}</span>
        {detail.channel === "Email" && (
          <button
            className="message-detail-reply"
            onClick={() =>
              dispatch({
                type: "OPEN_REPLY",
                messageId: detail.id,
                threadId: detail.threadId,
              })
            }
          >
            Reply
          </button>
        )}
      </div>
      <h2 className="detail-subject">{detail.subject ?? "(no subject)"}</h2>
      <div className="detail-meta">
        {detail.category ?? "—"}
        {detail.priority !== null ? ` · P${detail.priority}` : ""}
      </div>
      <MessageBody detail={detail} />
      {detail.attachments.length > 0 && (
        <div className="detail-attachments">
          <strong>Attachments:</strong>
          <ul>
            {detail.attachments.map((a) => (
              <li key={a.filename}>
                {a.filename} ({(a.sizeBytes / 1024).toFixed(1)} KB)
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
