import { InboxProvider, useInbox } from "./state/InboxContext";
import { Sidebar } from "./components/Sidebar";
import { SplitPane } from "./components/SplitPane";
import { MessageList } from "./components/MessageList";
import { MessageDetail } from "./components/MessageDetail";

const MIN = { sidebar: 160, list: 260, detail: 320 };
const HANDLE_PX = 6;

function InboxLayout() {
  const { state, dispatch } = useInbox();
  const { panelWidths, error } = state;
  const winW = typeof window !== "undefined" ? window.innerWidth : 1000;

  const sidebarMax = Math.max(
    MIN.sidebar,
    winW - (MIN.list + MIN.detail + HANDLE_PX * 2),
  );
  const listMax = Math.max(
    MIN.list,
    winW - (panelWidths.sidebar + MIN.detail + HANDLE_PX * 2),
  );

  const gridCols = `${panelWidths.sidebar}px ${HANDLE_PX}px ${panelWidths.list}px ${HANDLE_PX}px 1fr`;

  const setSidebar = (w: number) =>
    dispatch({
      type: "SET_PANEL_WIDTHS",
      widths: { ...panelWidths, sidebar: w },
    });
  const setList = (w: number) =>
    dispatch({
      type: "SET_PANEL_WIDTHS",
      widths: { ...panelWidths, list: w },
    });

  return (
    <div className="app">
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button
            className="error-dismiss"
            onClick={() => dispatch({ type: "SET_ERROR", error: null })}
            aria-label="Dismiss error"
          >
            ×
          </button>
        </div>
      )}
      <div className="inbox-grid" style={{ gridTemplateColumns: gridCols }}>
        <Sidebar />
        <SplitPane
          target="sidebar"
          onResize={setSidebar}
          currentWidth={panelWidths.sidebar}
          min={MIN.sidebar}
          max={sidebarMax}
        />
        <MessageList />
        <SplitPane
          target="list"
          onResize={setList}
          currentWidth={panelWidths.list}
          min={MIN.list}
          max={listMax}
        />
        <MessageDetail />
      </div>
    </div>
  );
}

export default function App() {
  return (
    <InboxProvider>
      <InboxLayout />
    </InboxProvider>
  );
}
