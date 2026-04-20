import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import type { MessageRow, MessageDetail, UiConfig } from "./types";

const PAGE_SIZE = 50;

export default function App() {
  const [config, setConfig] = useState<UiConfig | null>(null);
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [detail, setDetail] = useState<MessageDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const loadInitial = useCallback(async () => {
    setError(null);
    setLoading(true);
    try {
      const [cfg, rows] = await Promise.all([
        api.getConfig(),
        api.listMessages(PAGE_SIZE, 0),
      ]);
      setConfig(cfg);
      setMessages(rows);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadInitial();
  }, [loadInitial]);

  const loadMore = useCallback(async () => {
    try {
      const next = await api.listMessages(PAGE_SIZE, messages.length);
      setMessages((m) => [...m, ...next]);
    } catch (err) {
      setError(String(err));
    }
  }, [messages.length]);

  const openDetail = useCallback(async (id: string) => {
    setError(null);
    try {
      const d = await api.getMessage(id);
      setDetail(d);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  const dbPath = config?.db_path ?? "(no config)";
  const channelCount = config?.channel_count ?? 0;

  if (detail) {
    return (
      <DetailView detail={detail} onBack={() => setDetail(null)} />
    );
  }

  return (
    <div className="app">
      <header className="header">
        <div className="brand">MessageHub</div>
        <div className="meta">
          db: <code>{dbPath}</code> · {channelCount} channel
          {channelCount === 1 ? "" : "s"}
        </div>
        <button onClick={loadInitial} disabled={loading}>
          {loading ? "Loading..." : "Refresh"}
        </button>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <ul className="message-list">
        {messages.map((m) => (
          <li
            key={m.id}
            className={`row ${m.is_read ? "" : "unread"}`}
            onClick={() => openDetail(m.id)}
          >
            <div className="row-main">
              <span className="time">{formatTime(m.timestamp)}</span>
              <span className="channel">
                [{m.channel_label ?? m.channel}]
              </span>
              <span className="sender">{m.sender_name}</span>
            </div>
            <div className="row-subject">
              {m.subject ?? "(no subject)"}
            </div>
            <div className="row-preview">{m.preview}</div>
            <div className="row-meta">
              {m.category ?? "—"}
              {m.priority !== null ? ` · P${m.priority}` : ""}
            </div>
          </li>
        ))}
      </ul>

      {messages.length > 0 && messages.length % PAGE_SIZE === 0 && (
        <button className="load-more" onClick={loadMore}>
          Load more
        </button>
      )}
      {messages.length === 0 && !loading && (
        <div className="empty">No messages yet. Run <code>runtime-demo</code> to populate the DB.</div>
      )}
    </div>
  );
}

function DetailView({
  detail,
  onBack,
}: {
  detail: MessageDetail;
  onBack: () => void;
}) {
  return (
    <div className="detail">
      <button onClick={onBack} className="back">← Back</button>
      <div className="detail-head">
        <span className="channel">[{detail.channel_label ?? detail.channel}]</span>
        <span className="sender">{detail.sender_name}</span>
        <span className="time">{formatTime(detail.timestamp)}</span>
      </div>
      <h2 className="detail-subject">{detail.subject ?? "(no subject)"}</h2>
      <div className="detail-meta">
        {detail.category ?? "—"}
        {detail.priority !== null ? ` · P${detail.priority}` : ""}
      </div>
      <pre className="detail-body">{detail.body}</pre>
      {detail.attachments.length > 0 && (
        <div className="detail-attachments">
          <strong>Attachments:</strong>
          <ul>
            {detail.attachments.map((a) => (
              <li key={a.filename}>
                {a.filename} ({(a.size_bytes / 1024).toFixed(1)} KB)
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function formatTime(isoString: string): string {
  const d = new Date(isoString);
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  return sameDay
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
