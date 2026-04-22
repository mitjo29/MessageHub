import { useEffect, useRef } from "react";

/**
 * Debounced autosave. Calls `onSave(value)` once the value has been stable
 * for `delayMs`. Also flushes a final save on unmount if the value has
 * changed since the last successful save. Save errors are swallowed — the
 * next change will trigger another attempt.
 */
export function useAutosave<T>(
  value: T,
  delayMs: number,
  onSave: (value: T) => Promise<void>,
): void {
  const lastSavedRef = useRef<T>(value);
  const timerRef = useRef<number | null>(null);
  const onSaveRef = useRef(onSave);

  // Keep onSave fresh without re-arming the timer on every re-render.
  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  useEffect(() => {
    if (Object.is(value, lastSavedRef.current)) {
      return;
    }
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
    }
    const handle = window.setTimeout(() => {
      const snapshot = value;
      onSaveRef
        .current(snapshot)
        .then(() => {
          lastSavedRef.current = snapshot;
        })
        .catch((err) => {
          console.error("useAutosave: save failed", err);
        });
    }, delayMs);
    timerRef.current = handle;
    return () => {
      window.clearTimeout(handle);
    };
  }, [value, delayMs]);

  // Final flush on unmount if dirty.
  useEffect(() => {
    return () => {
      if (!Object.is(value, lastSavedRef.current)) {
        onSaveRef.current(value).catch((err) => {
          console.error("useAutosave: final flush failed", err);
        });
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
