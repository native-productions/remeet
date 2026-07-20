import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  onAction,
  registerActionTypes,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import { api } from "./api";

/** Notification action type carrying the one-tap Record button. */
const ACTION_TYPE = "call";
const RECORD_ACTION = "record";

/**
 * The "you forgot to record" reminder, owned by the popover.
 *
 * The Rust detector emits `call-detected` when another app puts a call on the mic
 * and speakers (already gated on settings, cooldown, and Remeet not recording). The
 * popover is the right owner because its webview is alive from launch even while
 * hidden: it can post a notification whose tap calls straight into `start_recording`
 * — something a notification posted from Rust cannot do — and it also raises an
 * in-window prompt for when the popover is already open.
 *
 * `detected` drives the in-app banner; `record` and `dismiss` clear it. A tap on the
 * notification records through the same path.
 */
export function useCallReminder(onRecorded?: () => void) {
  const [detected, setDetected] = useState(false);

  // Starting is idempotent on the backend, so the notification tap and the banner
  // button can both call this without racing — a double start is rejected, not
  // doubled. Clearing the prompt is unconditional: whether the start succeeded or
  // it was already recording, the reminder has served its purpose.
  const record = useCallback(async () => {
    setDetected(false);
    try {
      await api.startRecording();
      onRecorded?.();
    } catch {
      // Already recording, or capture failed; the reminder is done either way.
    }
  }, [onRecorded]);

  const dismiss = useCallback(() => setDetected(false), []);

  // Register the notification action once, and ask for permission up front so the
  // first real reminder is not the moment the OS prompt appears.
  useEffect(() => {
    void (async () => {
      try {
        if (!(await isPermissionGranted())) {
          await requestPermission();
        }
        await registerActionTypes([
          {
            id: ACTION_TYPE,
            actions: [{ id: RECORD_ACTION, title: "Record", foreground: true }],
          },
        ]);
      } catch {
        // No notification permission: the in-app banner still works on its own.
      }
    })();
  }, []);

  // Any interaction with the notification — the Record button or a tap on the body —
  // means "yes, record it". A plain dismiss does not fire this.
  useEffect(() => {
    const pending = onAction(() => void record());
    return () => {
      void pending.then((listener) => listener.unregister()).catch(() => {});
    };
  }, [record]);

  useEffect(() => {
    const pending = listen("call-detected", () => {
      setDetected(true);
      void sendNotification({
        title: "Meeting in progress?",
        body: "Your mic and speakers are both live. Tap to record it with Remeet.",
        actionTypeId: ACTION_TYPE,
      });
    });
    return () => {
      void pending.then((off) => off()).catch(() => {});
    };
  }, []);

  return { detected, record, dismiss };
}
