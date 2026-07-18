import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { api, type Recording } from "./api";

/** The recordings list, plus a refresh both windows call after a change. */
export function useRecordings() {
  const [recordings, setRecordings] = useState<Recording[]>([]);
  const [loaded, setLoaded] = useState(false);

  /** Returns the fresh list too, so a caller can reconcile against it directly. */
  const refresh = useCallback(async (): Promise<Recording[]> => {
    let list: Recording[] = [];
    try {
      list = await api.listRecordings();
    } catch {
      list = [];
    }
    setRecordings(list);
    setLoaded(true);
    return list;
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Same reason the spaces list refreshes on focus: a window that is only hidden
  // would otherwise show whatever it last fetched, however old that is. Recordings
  // also appear from outside the app entirely, when a session finishes.
  useEffect(() => {
    const unfocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) void refresh();
    });
    return () => {
      void unfocus.then((off) => off());
    };
  }, [refresh]);

  return { recordings, loaded, refresh };
}
