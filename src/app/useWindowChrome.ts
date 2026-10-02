import { onMounted, onBeforeUnmount, ref } from "vue";
import { useMessage } from "naive-ui";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { TauriEvent, type UnlistenFn } from "@tauri-apps/api/event";
import { isBrowserDemo } from "../shared/api/tauriRuntime";
import { t } from "../shared/i18n";
import { titlebarGesture } from "./windowGestures";

export type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>["startResizeDragging"]>[0];
export const resizeDirections: ResizeDirection[] = ["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"];

export function latestWindowRead<T>(read: () => Promise<T>, apply: (value: T) => void, revision: () => number = () => 0): () => Promise<void> {
  let generation = 0;
  return async () => {
    const current = ++generation;
    const eventRevision = revision();
    const value = await read();
    if (current === generation && eventRevision === revision()) apply(value);
  };
}

/** Retry background reads without overlapping requests or repeated error toasts. */
export function pollWindowState(refresh: () => Promise<void>): () => void {
  let busy = false;
  const timer = setInterval(() => {
    if (busy) return;
    busy = true;
    void refresh().catch(() => {}).finally(() => { busy = false; });
  }, 1_000);
  return () => clearInterval(timer);
}

/** Track each listener independently, including registrations that finish after disposal. */
export function windowSubscriptions(register: Array<() => Promise<UnlistenFn>>, onFailure: () => void) {
  let disposed = false;
  const subscriptions = register.map((subscribe) => ({ subscribe, pending: false, unlisten: undefined as UnlistenFn | undefined }));
  return {
    ready: () => subscriptions.every((subscription) => subscription.unlisten),
    retry() {
      for (const subscription of subscriptions) {
        if (disposed || subscription.pending || subscription.unlisten) continue;
        subscription.pending = true;
        void (async () => {
          try {
            const unlisten = await subscription.subscribe();
            if (disposed) unlisten();
            else subscription.unlisten = unlisten;
          } catch {
            if (!disposed) onFailure();
          } finally {
            subscription.pending = false;
          }
        })();
      }
    },
    dispose() {
      disposed = true;
      subscriptions.forEach((subscription) => subscription.unlisten?.());
    },
  };
}

export function useWindowChrome() {
  const preview = isBrowserDemo();
  const nativeWindow = preview ? null : getCurrentWindow();
  // Cocoa resizes undecorated windows natively; Tao's resize-drag API is unsupported there.
  const resizeHandlesEnabled = preview || !navigator.userAgent.includes("Macintosh");
  // All desktop windows start undecorated; native state remains authoritative.
  const enabled = ref(true);
  const focused = ref(true);
  const maximized = ref(false);
  const fullscreen = ref(false);
  const message = useMessage();
  let stopFallback: (() => void) | undefined;
  let disposed = false;
  let focusRevision = 0;
  let recoveryRevision = 0;
  let failureReported = false;

  const refreshMaximized = latestWindowRead(
    async () => nativeWindow ? nativeWindow.isMaximized() : maximized.value,
    (value) => { if (!disposed) maximized.value = value; },
  );
  const refreshFullscreen = latestWindowRead(
    async () => nativeWindow ? nativeWindow.isFullscreen() : fullscreen.value,
    (value) => { if (!disposed) fullscreen.value = value; },
  );
  const refreshFocus = latestWindowRead(
    async () => nativeWindow ? nativeWindow.isFocused() : focused.value,
    (value) => { if (!disposed) focused.value = value; },
    () => focusRevision,
  );

  async function refreshDecoration() {
    if (!nativeWindow) return;
    const decorated = await nativeWindow.isDecorated();
    if (!disposed) enabled.value = !decorated;
  }

  function synchronizeWindow() {
    return Promise.allSettled([refreshMaximized(), refreshFullscreen(), refreshFocus(), refreshDecoration()]);
  }

  const subscriptions = windowSubscriptions(nativeWindow ? [
    () => nativeWindow.onResized(() => { void perform(refreshGeometry); }),
    () => nativeWindow.listen(TauriEvent.WINDOW_FOCUS, () => { setFocus(true); }),
    () => nativeWindow.listen(TauriEvent.WINDOW_BLUR, () => { setFocus(false); }),
  ] : [], reportFailure);

  async function refreshGeometry() {
    const results = await Promise.allSettled([refreshMaximized(), refreshFullscreen()]);
    if (results.some((result) => result.status === "rejected")) throw new Error("Window geometry read failed");
  }

  async function synchronizeAndRecover() {
    const revision = recoveryRevision;
    subscriptions.retry();
    const results = await synchronizeWindow();
    if (disposed) return;
    if (results.some((result) => result.status === "rejected")) reportFailure();
    else if (subscriptions.ready() && revision === recoveryRevision) {
      stopFallback?.();
      stopFallback = undefined;
      failureReported = false;
    }
  }

  function ensurePolling() {
    recoveryRevision += 1;
    if (disposed || stopFallback) return;
    stopFallback = pollWindowState(synchronizeAndRecover);
  }

  function reportFailure() {
    if (disposed) return;
    if (!failureReported) message.error(t("workspace.header.windowOperationFailed"));
    failureReported = true;
    ensurePolling();
  }

  function setFocus(value: boolean) {
    focusRevision += 1;
    if (!disposed) focused.value = value;
  }

  async function perform(action: () => Promise<unknown>) {
    try {
      await action();
    } catch {
      reportFailure();
    }
  }

  async function toggleMaximize() {
    if (fullscreen.value) return;
    if (!nativeWindow) { maximized.value = !maximized.value; return; }
    await perform(async () => {
      await nativeWindow.toggleMaximize();
      await refreshGeometry();
    });
  }

  function startTitlebarGesture(event: MouseEvent) {
    if (!enabled.value || fullscreen.value) return;
    const gesture = titlebarGesture(event);
    if (!gesture) return;
    event.preventDefault();
    if (gesture === "maximize") void toggleMaximize();
    else if (nativeWindow) void perform(() => nativeWindow.startDragging());
  }

  function startResize(event: MouseEvent, direction: ResizeDirection) {
    if (event.button !== 0 || !enabled.value || !resizeHandlesEnabled || maximized.value || fullscreen.value || !nativeWindow) return;
    event.preventDefault();
    void perform(() => nativeWindow.startResizeDragging(direction));
  }

  onMounted(async () => {
    if (!nativeWindow) return;
    // Subscribe before querying so maximize/restore and focus cannot miss startup events.
    subscriptions.retry();
    ensurePolling();
    await synchronizeAndRecover();
  });

  onBeforeUnmount(() => {
    disposed = true;
    stopFallback?.();
    subscriptions.dispose();
  });

  return {
    enabled, focused, maximized, fullscreen, resizeHandlesEnabled, toggleMaximize, startTitlebarGesture, startResize,
    minimize: () => nativeWindow && perform(() => nativeWindow.minimize()),
    close: () => nativeWindow && perform(() => nativeWindow.close()),
  };
}
