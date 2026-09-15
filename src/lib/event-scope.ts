import type { EventCallback, EventName, UnlistenFn, listen } from "@tauri-apps/api/event";

/** Owns async native subscriptions for one React effect, including late registrations. */
export function createEventScope(subscribe: typeof listen, onError: (reason: unknown) => void) {
  let disposed = false;
  const subscriptions = new Set<UnlistenFn>();

  return {
    listen<T>(event: EventName, handler: EventCallback<T>) {
      if (disposed) return;
      void subscribe<T>(event, (message) => {
        if (!disposed) handler(message);
      }).then((unlisten) => {
        if (disposed) unlisten();
        else subscriptions.add(unlisten);
      }).catch((reason: unknown) => {
        if (!disposed) onError(reason);
      });
    },
    dispose() {
      disposed = true;
      for (const unlisten of subscriptions) unlisten();
      subscriptions.clear();
    },
  };
}
