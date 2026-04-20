import {
  createContext,
  useContext,
  useEffect,
  useReducer,
  useRef,
  type Dispatch,
  type ReactNode,
} from "react";
import { api } from "../api";
import type {
  MessageRow,
  MessageDetail,
  ChannelInfo,
  SidebarCounts,
  Filter,
} from "../types";

export const DEFAULT_PANEL_WIDTHS = { sidebar: 200, list: 360 };
export const PANEL_WIDTHS_KEY = "messagehub.desktop.panelWidths.v1";

export type InboxState = {
  filter: Filter;
  channels: ChannelInfo[];
  counts: SidebarCounts | null;
  messages: MessageRow[];
  hasMore: boolean;
  selectedId: string | null;
  detail: MessageDetail | null;
  panelWidths: { sidebar: number; list: number };
  error: string | null;
  loading: boolean;
};

export type InboxAction =
  | { type: "SET_FILTER"; filter: Filter }
  | { type: "SET_CHANNELS"; channels: ChannelInfo[] }
  | { type: "SET_COUNTS"; counts: SidebarCounts }
  | {
      type: "LOAD_MESSAGES_SUCCESS";
      messages: MessageRow[];
      append: boolean;
      hasMore: boolean;
    }
  | { type: "SELECT"; id: string | null }
  | { type: "LOAD_DETAIL_SUCCESS"; detail: MessageDetail }
  | { type: "MARK_READ_LOCAL"; id: string }
  | { type: "REVERT_MARK_READ_LOCAL"; id: string }
  | { type: "SET_PANEL_WIDTHS"; widths: { sidebar: number; list: number } }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "SET_LOADING"; loading: boolean };

export function loadInitialPanelWidths(): { sidebar: number; list: number } {
  try {
    const raw = window.localStorage.getItem(PANEL_WIDTHS_KEY);
    if (!raw) return DEFAULT_PANEL_WIDTHS;
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed.sidebar === "number" &&
      typeof parsed.list === "number"
    ) {
      return { sidebar: parsed.sidebar, list: parsed.list };
    }
    return DEFAULT_PANEL_WIDTHS;
  } catch {
    return DEFAULT_PANEL_WIDTHS;
  }
}

export const initialState: InboxState = {
  filter: { kind: "all" },
  channels: [],
  counts: null,
  messages: [],
  hasMore: false,
  selectedId: null,
  detail: null,
  panelWidths: DEFAULT_PANEL_WIDTHS, // hydrated on mount; see provider
  error: null,
  loading: false,
};

function bumpCounts(
  counts: SidebarCounts | null,
  row: MessageRow,
  delta: number,
): SidebarCounts | null {
  if (!counts) return counts;
  const byChannel = counts.byChannel.map((c) =>
    c.channelType === row.channel
      ? { ...c, unread: Math.max(0, c.unread + delta) }
      : c,
  );
  // `all` and `priorityHigh` are totals, not unread counts — mark-read
  // doesn't move them. Only `unread` (overall) and per-channel `unread` do.
  return {
    ...counts,
    unread: Math.max(0, counts.unread + delta),
    byChannel,
  };
}

export function inboxReducer(
  state: InboxState,
  action: InboxAction,
): InboxState {
  switch (action.type) {
    case "SET_FILTER":
      return {
        ...state,
        filter: action.filter,
        messages: [],
        hasMore: false,
        selectedId: null,
        detail: null,
        error: null,
      };

    case "SET_CHANNELS":
      return { ...state, channels: action.channels };

    case "SET_COUNTS":
      return { ...state, counts: action.counts };

    case "LOAD_MESSAGES_SUCCESS":
      return {
        ...state,
        messages: action.append
          ? [...state.messages, ...action.messages]
          : action.messages,
        hasMore: action.hasMore,
        loading: false,
        error: null,
      };

    case "SELECT":
      return {
        ...state,
        selectedId: action.id,
        detail: action.id === null ? null : state.detail,
      };

    case "LOAD_DETAIL_SUCCESS":
      return { ...state, detail: action.detail, error: null };

    case "MARK_READ_LOCAL": {
      const row = state.messages.find((m) => m.id === action.id);
      if (!row || row.is_read) return state;
      const markedRow: MessageRow = { ...row, is_read: true };
      const messages =
        state.filter.kind === "unread"
          ? state.messages.filter((m) => m.id !== action.id)
          : state.messages.map((m) => (m.id === action.id ? markedRow : m));
      return {
        ...state,
        messages,
        counts: bumpCounts(state.counts, row, -1),
        detail: state.detail && state.detail.id === action.id
          ? { ...state.detail, is_read: true }
          : state.detail,
      };
    }

    case "REVERT_MARK_READ_LOCAL": {
      const row =
        state.messages.find((m) => m.id === action.id) ||
        (state.detail && state.detail.id === action.id
          ? ({
              id: state.detail.id,
              timestamp: state.detail.timestamp,
              channel: state.detail.channel,
              channel_label: state.detail.channel_label,
              sender_name: state.detail.sender_name,
              subject: state.detail.subject,
              preview: state.detail.preview,
              category: state.detail.category,
              priority: state.detail.priority,
              is_read: true,
            } as MessageRow)
          : null);
      if (!row) return state;
      const revertedRow: MessageRow = { ...row, is_read: false };
      const messages =
        state.filter.kind === "unread" &&
        !state.messages.some((m) => m.id === action.id)
          ? [revertedRow, ...state.messages]
          : state.messages.map((m) =>
              m.id === action.id ? revertedRow : m,
            );
      return {
        ...state,
        messages,
        counts: bumpCounts(state.counts, revertedRow, +1),
        detail: state.detail && state.detail.id === action.id
          ? { ...state.detail, is_read: false }
          : state.detail,
      };
    }

    case "SET_PANEL_WIDTHS":
      return { ...state, panelWidths: action.widths };

    case "SET_ERROR":
      return { ...state, error: action.error };

    case "SET_LOADING":
      return { ...state, loading: action.loading };

    default:
      return state;
  }
}

const InboxContext = createContext<{
  state: InboxState;
  dispatch: Dispatch<InboxAction>;
} | null>(null);

const PAGE_SIZE = 50;
const POLL_INTERVAL_MS = 15_000;
const PERSIST_DEBOUNCE_MS = 200;

export function InboxProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(inboxReducer, initialState, (s) => ({
    ...s,
    panelWidths: loadInitialPanelWidths(),
  }));

  // One-time channels fetch on mount.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const channels = await api.listChannels();
        if (!cancelled) dispatch({ type: "SET_CHANNELS", channels });
      } catch (err) {
        if (!cancelled) dispatch({ type: "SET_ERROR", error: String(err) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Load messages + counts whenever the filter changes (and on mount).
  const filterRef = useRef(state.filter);
  filterRef.current = state.filter;

  useEffect(() => {
    let cancelled = false;
    dispatch({ type: "SET_LOADING", loading: true });
    (async () => {
      try {
        const [rows, counts] = await Promise.all([
          api.listMessages(state.filter, PAGE_SIZE, 0),
          api.sidebarCounts(),
        ]);
        if (cancelled) return;
        dispatch({
          type: "LOAD_MESSAGES_SUCCESS",
          messages: rows,
          append: false,
          hasMore: rows.length === PAGE_SIZE,
        });
        dispatch({ type: "SET_COUNTS", counts });
      } catch (err) {
        if (!cancelled) dispatch({ type: "SET_ERROR", error: String(err) });
      } finally {
        if (!cancelled) dispatch({ type: "SET_LOADING", loading: false });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [state.filter]);

  // Poll every POLL_INTERVAL_MS + on window focus. Refetches counts + page 0
  // only; preserves selection and already-loaded older pages.
  useEffect(() => {
    const tick = async () => {
      try {
        const [rows, counts] = await Promise.all([
          api.listMessages(filterRef.current, PAGE_SIZE, 0),
          api.sidebarCounts(),
        ]);
        dispatch({
          type: "LOAD_MESSAGES_SUCCESS",
          messages: rows,
          append: false,
          hasMore: rows.length === PAGE_SIZE,
        });
        dispatch({ type: "SET_COUNTS", counts });
      } catch {
        // Silent — polling shouldn't spam the banner.
      }
    };
    const id = window.setInterval(tick, POLL_INTERVAL_MS);
    const onVis = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, []);

  // Persist panel widths with debounce.
  useEffect(() => {
    const handle = window.setTimeout(() => {
      try {
        window.localStorage.setItem(
          PANEL_WIDTHS_KEY,
          JSON.stringify(state.panelWidths),
        );
      } catch {
        // localStorage can throw in private-browsing modes; ignore.
      }
    }, PERSIST_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [state.panelWidths]);

  return (
    <InboxContext.Provider value={{ state, dispatch }}>
      {children}
    </InboxContext.Provider>
  );
}

export function useInbox() {
  const ctx = useContext(InboxContext);
  if (!ctx) {
    throw new Error("useInbox must be used inside <InboxProvider>");
  }
  return ctx;
}
