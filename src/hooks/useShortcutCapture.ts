import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createEventScope } from "../lib/event-scope";
import { type ShortcutBinding } from "../types";

export function useShortcutCapture(active: boolean, setActive: (active: boolean) => void, onCaptured: (binding: ShortcutBinding) => void, onError: (message: string) => void) {
  const activeRef = useRef(active);
  const capturedRef = useRef(onCaptured);
  const errorRef = useRef(onError);
  activeRef.current = active;
  capturedRef.current = onCaptured;
  errorRef.current = onError;

  useEffect(() => {
    const events = createEventScope(listen, (reason) => errorRef.current(String(reason)));
    events.listen<{ binding: ShortcutBinding }>("shortcut:captured", ({ payload }) => { capturedRef.current(payload.binding); setActive(false); });
    events.listen<string>("shortcut:capture-error", ({ payload }) => errorRef.current(payload));
    events.listen("shortcut:capture-cancelled", () => setActive(false));
    return () => {
      events.dispose();
      if (activeRef.current) void invoke("cancel_shortcut_capture").catch(() => undefined);
    };
  }, [setActive]);

  useEffect(() => {
    if (!active) return;
    const cancelWithEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.repeat) return;
      event.preventDefault();
      event.stopPropagation();
      void invoke("cancel_shortcut_capture")
        .catch((reason) => errorRef.current(String(reason)))
        .finally(() => setActive(false));
    };
    window.addEventListener("keydown", cancelWithEscape, true);
    return () => window.removeEventListener("keydown", cancelWithEscape, true);
  }, [active, setActive]);
}
