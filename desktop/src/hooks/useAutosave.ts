import { useEffect, useRef } from "react";

/**
 * Debounced autosave. Calls `onSave(value)` once the value has been stable
 * for `delayMs`. Also flushes a final save on unmount if the value has
 * changed since the last successful save. Save errors are swallowed — the
 * next change will trigger another attempt.
 *
 * Returns a `markSaved(value)` function callers should invoke when they've
 * just persisted the value through a separate code path (e.g. explicit
 * Send/Discard) and want the hook to treat that value as the new baseline.
 * This prevents the unmount-flush from firing a redundant (or destructive)
 * save on top of the external write.
 */
export function useAutosave<T>(
  value: T,
  delayMs: number,
  onSave: (value: T) => Promise<void>,
): { markSaved: (value: T) => void } {
  const lastSavedRef = useRef<T>(value);
  const timerRef = useRef<number | null>(null);
  const onSaveRef = useRef(onSave);
  const valueRef = useRef<T>(value);

  // Keep onSave fresh without re-arming the timer on every re-render.
  useEffect(() => {
    onSaveRef.current = onSave;
  }, [onSave]);

  // Keep valueRef fresh so the unmount-flush effect reads the latest value,
  // not the mount-time value captured by its closure.
  useEffect(() => {
    valueRef.current = value;
  });

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

  // Final flush on unmount if dirty. Reads valueRef so it sees the current
  // value, not the mount-time snapshot.
  useEffect(() => {
    return () => {
      const current = valueRef.current;
      if (!Object.is(current, lastSavedRef.current)) {
        onSaveRef.current(current).catch((err) => {
          console.error("useAutosave: final flush failed", err);
        });
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return {
    markSaved(value: T) {
      lastSavedRef.current = value;
      // Cancel any pending debounced save — the caller has handled it
      // through a separate code path.
      if (timerRef.current != null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    },
  };
}
