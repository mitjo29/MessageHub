import { useEffect, useState } from "react";
import { aiDraftReply, cloudConfigStatus, listAiDrafts } from "../api";
import type { AiDraftSummary } from "../types";
import { PriorDraftsDropdown } from "./PriorDraftsDropdown";

interface Props {
  messageId: string;
  onDraftReady: (body: string, confidence: number) => void;
}

export function AiAssistPanel({ messageId, onDraftReady }: Props) {
  const [configured, setConfigured] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(false);
  const [confidence, setConfidence] = useState<number | null>(null);
  const [redact, setRedact] = useState(true);
  const [priorDrafts, setPriorDrafts] = useState<AiDraftSummary[]>([]);
  const [priorOpen, setPriorOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cfg, drafts] = await Promise.all([
          cloudConfigStatus(),
          listAiDrafts(messageId),
        ]);
        if (cancelled) return;
        setConfigured(cfg.configured);
        setPriorDrafts(drafts);
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [messageId]);

  async function handleGenerate() {
    if (loading) return;
    setLoading(true);
    setError(null);
    try {
      const out = await aiDraftReply(messageId, redact);
      onDraftReady(out.body, out.confidence);
      setConfidence(out.confidence);
      const drafts = await listAiDrafts(messageId);
      setPriorDrafts(drafts);
    } catch (err) {
      setError(typeof err === "string" ? err : String(err));
    } finally {
      setLoading(false);
    }
  }

  if (configured === null) {
    return <div className="ai-panel">Loading...</div>;
  }
  if (!configured) {
    return (
      <div className="ai-panel ai-panel-disabled">
        <div className="ai-panel-title">AI assist</div>
        <p className="ai-panel-hint">
          Not configured — add <code>[cloud]</code> to{" "}
          <code>messagehub.toml</code>
        </p>
      </div>
    );
  }

  const btnLabel = priorDrafts.length === 0 ? "Generate draft" : "Regenerate";

  return (
    <div className="ai-panel">
      <div className="ai-panel-title">
        AI assist
        {confidence != null && (
          <span className="ai-panel-chip">{confidence.toFixed(2)}</span>
        )}
      </div>
      <button
        className="ai-panel-generate"
        onClick={handleGenerate}
        disabled={loading}
      >
        {loading ? "Generating..." : btnLabel}
      </button>
      <label className="ai-panel-redact">
        <input
          type="checkbox"
          checked={redact}
          onChange={(e) => setRedact(e.target.checked)}
        />{" "}
        Redact PII
      </label>
      <button
        className="ai-panel-prior-toggle"
        onClick={() => setPriorOpen((x) => !x)}
        disabled={priorDrafts.length === 0}
      >
        Prior drafts ({priorDrafts.length}) {priorOpen ? "▴" : "▾"}
      </button>
      {priorOpen && (
        <PriorDraftsDropdown
          drafts={priorDrafts}
          onRestore={(body) => onDraftReady(body, 0)}
        />
      )}
      {error && <div className="ai-panel-error">{error}</div>}
    </div>
  );
}
