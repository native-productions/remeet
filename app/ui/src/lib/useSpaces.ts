import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { api, type Space } from "./api";

/**
 * The user's spaces, plus which one new recordings are filed into.
 *
 * Both windows read this: the popover to offer the picker, the main window to show
 * the folders. Neither caches it beyond a render, so a space created in one window
 * shows up in the other as soon as it refreshes.
 */
export function useSpaces() {
  const [spaces, setSpaces] = useState<Space[]>([]);
  const [activeSpace, setActive] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);

  const refresh = useCallback(async (): Promise<Space[]> => {
    let list: Space[] = [];
    try {
      list = await api.listSpaces();
      const settings = await api.getSettings();
      setActive(settings.active_space);
    } catch {
      list = [];
    }
    setSpaces(list);
    setLoaded(true);
    return list;
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The popover is hidden, never closed, so mounting once is not enough: a space
  // created in the main window has to arrive without a restart. Two sources, because
  // neither covers the other. The event catches edits made while both windows are
  // up; the focus check catches anything that changed the file behind the app's back,
  // including a hand-edited spaces.json.
  useEffect(() => {
    const unlisten = listen("spaces-changed", () => void refresh());
    const unfocus = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) void refresh();
    });

    return () => {
      void unlisten.then((off) => off());
      void unfocus.then((off) => off());
    };
  }, [refresh]);

  const chooseActive = useCallback(async (id: string | null) => {
    // Optimistic: the picker must feel instant, and the only cost of a failed
    // write is that the next recording lands in the previous space.
    setActive(id);
    try {
      await api.setActiveSpace(id);
    } catch {
      // Left as chosen; the next refresh will correct it.
    }
  }, []);

  return { spaces, activeSpace, loaded, refresh, chooseActive };
}
