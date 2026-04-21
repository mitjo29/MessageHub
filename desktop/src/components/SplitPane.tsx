import { useCallback, useRef } from "react";

type Props = {
  /** "sidebar" resizes the sidebar↔list gap; "list" resizes the list↔detail gap. */
  target: "sidebar" | "list";
  /** Called on every mousemove with the proposed new width (clamped). */
  onResize: (width: number) => void;
  /** Current width of the *target* column so we can compute deltas. */
  currentWidth: number;
  /** Min/max for the target column. */
  min: number;
  max: number;
};

export function SplitPane({
  target,
  onResize,
  currentWidth,
  min,
  max,
}: Props) {
  const startXRef = useRef(0);
  const startWidthRef = useRef(0);

  const onMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      startXRef.current = e.clientX;
      startWidthRef.current = currentWidth;
      const prevCursor = document.body.style.cursor;
      const prevSelect = document.body.style.userSelect;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const onMove = (me: MouseEvent) => {
        const delta = me.clientX - startXRef.current;
        const next = Math.max(min, Math.min(max, startWidthRef.current + delta));
        onResize(next);
      };
      const onUp = () => {
        document.body.style.cursor = prevCursor;
        document.body.style.userSelect = prevSelect;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [currentWidth, min, max, onResize],
  );

  return (
    <div
      className="split-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={target === "sidebar" ? "Resize sidebar" : "Resize message list"}
      onMouseDown={onMouseDown}
    />
  );
}
