import type { AiDraftSummary } from "../types";

interface Props {
  drafts: AiDraftSummary[];
  onRestore: (body: string) => void;
}

export function PriorDraftsDropdown({ drafts, onRestore }: Props) {
  return (
    <ul className="prior-drafts">
      {drafts.map((d) => (
        <li key={d.id} className="prior-draft-row">
          <div className="prior-draft-meta">
            <span>{new Date(d.createdAt).toLocaleString()}</span>
            <span>·</span>
            <span>conf {d.confidence.toFixed(2)}</span>
            {d.hasUserEdit && (
              <span className="prior-draft-edited">(edited)</span>
            )}
          </div>
          <div className="prior-draft-preview">{d.preview}</div>
          <button
            className="prior-draft-restore"
            onClick={() => onRestore(d.body)}
          >
            Restore
          </button>
        </li>
      ))}
    </ul>
  );
}
