import { useInbox } from "../state/InboxContext";
import type { Filter } from "../types";

type ItemProps = {
  active: boolean;
  label: string;
  total: number | null;
  unread?: number | null;
  onClick: () => void;
  disabled?: boolean;
};

function Item({ active, label, total, unread, onClick, disabled }: ItemProps) {
  const totalText =
    total === null ? "—" : total === 0 ? "—" : total.toString();
  return (
    <div
      className={`sidebar-item${active ? " active" : ""}${disabled ? " disabled" : ""}`}
      aria-selected={active}
      role="option"
      onClick={onClick}
    >
      <span className="sidebar-label">{label}</span>
      <span className="sidebar-counts">
        {unread != null && unread > 0 && (
          <span className="sidebar-unread">{unread}</span>
        )}
        <span className="sidebar-total">{totalText}</span>
      </span>
    </div>
  );
}

function filtersEqual(a: Filter, b: Filter): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "channel" && b.kind === "channel") {
    return a.channelType === b.channelType;
  }
  return true;
}

export function Sidebar() {
  const { state, dispatch } = useInbox();
  const { channels, counts, filter } = state;

  const setFilter = (next: Filter) => {
    if (filtersEqual(filter, next)) return;
    dispatch({ type: "SET_FILTER", filter: next });
  };

  return (
    <nav className="sidebar" aria-label="Inbox navigation">
      <div className="sidebar-section-label">Views</div>
      <Item
        active={filter.kind === "all"}
        label="All"
        total={counts?.all ?? null}
        onClick={() => setFilter({ kind: "all" })}
      />
      <Item
        active={filter.kind === "unread"}
        label="Unread"
        total={counts?.unread ?? null}
        unread={counts?.unread ?? null}
        onClick={() => setFilter({ kind: "unread" })}
      />
      <Item
        active={filter.kind === "priorityHigh"}
        label="Priority"
        total={counts?.priorityHigh ?? null}
        onClick={() => setFilter({ kind: "priorityHigh" })}
      />

      <div className="sidebar-section-label">Channels</div>
      {channels.length === 0 ? (
        <div className="sidebar-empty">No channels</div>
      ) : (
        channels.map((c) => {
          const cc = counts?.byChannel.find(
            (x) => x.channelType === c.channel_type,
          );
          const active =
            filter.kind === "channel" && filter.channelType === c.channel_type;
          return (
            <Item
              key={c.id}
              active={active}
              label={c.label || c.channel_type}
              total={cc?.total ?? null}
              unread={cc?.unread ?? null}
              disabled={!c.enabled}
              onClick={() =>
                setFilter({
                  kind: "channel",
                  channelType: c.channel_type,
                })
              }
            />
          );
        })
      )}
    </nav>
  );
}
