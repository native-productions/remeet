import { useEffect, useState } from "react";

import { api, type AppInfo } from "./api";

/**
 * The build's version and dev/release mode.
 *
 * Fetched once — it is baked in at compile time and never changes while the app runs,
 * so both windows can read it on mount without a refresh. `null` until it arrives.
 */
export function useAppInfo(): AppInfo | null {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    let alive = true;
    void api
      .appInfo()
      .then((value) => {
        if (alive) setInfo(value);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  return info;
}
