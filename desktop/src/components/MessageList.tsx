import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { useInbox } from "../state/InboxContext";
import type { MessageRow } from "../types";

const PAGE_SIZE = 50;

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

export function MessageList() {
  const { state, dispatch } = useInbox();
  const { messages, selectedId, hasMore, loading, filter } = state;
  const [loadingMore, setLoadingMore] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  const openMessage = useCallback(
    async (id: string) => {
      dispatch({ type: "SELECT", id });
      try {
        const d = await api.getMessage(id);
        dispatch({ type: "LOAD_DETAIL_SUCCESS", detail: d });
      } catch (err) {
        dispatch({ type: "SET_ERROR", error: String(err) });
      }
    },
    [dispatch],
  );

  const onClickRow = (row: MessageRow) => {
    void openMessage(row.id);
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (messages.length === 0) return;
    const currentIdx = selectedId
      ? messages.findIndex((m) => m.id === selectedId)
      : -1;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      const next = Math.min(messages.length - 1, currentIdx + 1);
      if (next !== currentIdx) void openMessage(messages[next].id);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, currentIdx - 1);
      if (currentIdx === -1) {
        void openMessage(messages[0].id);
      } else if (next !== currentIdx) {
        void openMessage(messages[next].id);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (currentIdx >= 0) void openMessage(messages[currentIdx].id);
    } else if (e.key === "Escape") {
      e.preventDefault();
      dispatch({ type: "SELECT", id: null });
    }
  };

  useEffect(() => {
    if (!selectedId || !listRef.current) return;
    const el = listRef.current.querySelector<HTMLElement>(
      `[data-row-id="${selectedId}"]`,
    );
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  const loadMore = async () => {
    if (loadingMore) return;
    setLoadingMore(true);
    try {
      const next = await api.listMessages(filter, PAGE_SIZE, messages.length);
      dispatch({
        type: "LOAD_MESSAGES_SUCCESS",
        messages: next,
        append: true,
        hasMore: next.length === PAGE_SIZE,
      });
    } catch (err) {
      dispatch({ type: "SET_ERROR", error: String(err) });
    } finally {
      setLoadingMore(false);
    }
  };

  return (
    <div
      className="message-list"
      tabIndex={0}
      role="listbox"
      aria-activedescendant={selectedId ?? undefined}
      onKeyDown={onKeyDown}
      ref={listRef}
    >
      {messages.length === 0 && !loading && (
        <div className="empty">No messages in this view.</div>
      )}

      {messages.map((m) => (
        <div
          key={m.id}
          id={m.id}
          data-row-id={m.id}
          role="option"
          aria-selected={selectedId === m.id}
          className={`message-row${m.isRead ? "" : " unread"}${selectedId === m.id ? " selected" : ""}`}
          onClick={() => onClickRow(m)}
        >
          <div className="row-main">
            <span className="time">{formatTime(m.timestamp)}</span>
            <span className="channel">[{m.channelLabel ?? m.channel}]</span>
            <span className="sender">{m.senderName}</span>
          </div>
          <div className="row-subject">{m.subject ?? "(no subject)"}</div>
          <div className="row-preview">{m.preview}</div>
          <div className="row-meta">
            {m.category ?? "—"}
            {m.priority !== null ? ` · P${m.priority}` : ""}
          </div>
        </div>
      ))}

      {hasMore && (
        <button className="load-more" onClick={loadMore} disabled={loadingMore}>
          {loadingMore ? "Loading…" : "Load more"}
        </button>
      )}
    </div>
  );
}
