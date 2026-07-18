import { useCallback, useEffect, useState } from "react";

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

  return { recordings, loaded, refresh };
}
