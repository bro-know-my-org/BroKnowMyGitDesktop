import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const repositoryPanelWidthStorageKey = "broknowmygitdesktop.repository-panel-width";

interface RepositoryPanelSplitOptions {
  storageKey?: string;
  widthProperty?: string;
  defaultWidth?: string;
  minimumListPaneWidth?: number;
  minimumDetailPaneWidth?: number;
}

function initialRepositoryPanelWidth(storageKey: string, minimumListPaneWidth: number): number | null {
  if (typeof window === "undefined") return null;
  try {
    const saved = Number.parseFloat(window.localStorage.getItem(storageKey) ?? "");
    return Number.isFinite(saved) && saved >= minimumListPaneWidth ? Math.round(saved) : null;
  } catch {
    return null;
  }
}

const defaultRepositoryPanelWidth = ref<number | null>(initialRepositoryPanelWidth(repositoryPanelWidthStorageKey, 240));

export function useRepositoryPanelSplit(options: RepositoryPanelSplitOptions = {}) {
  const {
    storageKey = repositoryPanelWidthStorageKey,
    widthProperty = "--repository-list-pane-width",
    defaultWidth = "27.2%",
    minimumListPaneWidth = 240,
    minimumDetailPaneWidth = 480,
  } = options;
  // Keep the original shared outer width; nested splits own their state and storage.
  const repositoryPanelWidth = storageKey === repositoryPanelWidthStorageKey
    ? defaultRepositoryPanelWidth
    : ref<number | null>(initialRepositoryPanelWidth(storageKey, minimumListPaneWidth));

  function maximumListPaneWidth(containerWidth: number): number {
    return Math.max(minimumListPaneWidth, containerWidth - minimumDetailPaneWidth);
  }

  function clampListPaneWidth(width: number, containerWidth: number): number {
    return Math.round(Math.min(Math.max(width, minimumListPaneWidth), maximumListPaneWidth(containerWidth)));
  }

  function rememberRepositoryPanelWidth(): void {
    if (typeof window === "undefined" || repositoryPanelWidth.value === null) return;
    try {
      window.localStorage.setItem(storageKey, String(repositoryPanelWidth.value));
    } catch {
      // The resized width still applies for this session if storage is unavailable.
    }
  }

  function resetRememberedRepositoryPanelWidth(): void {
    repositoryPanelWidth.value = null;
    if (typeof window === "undefined") return;
    try {
      window.localStorage.removeItem(storageKey);
    } catch {
      // Ignore unavailable browser storage; the in-memory value has still been reset.
    }
  }

  let resizing = false;
  let startPointerX = 0;
  let startListPaneWidth = 0;
  let resizeContainerWidth = 0;
  let liveListPaneWidth = minimumListPaneWidth;
  let pendingPointerX = 0;
  let resizeAnimationFrame: number | null = null;
  let activePointerId: number | null = null;
  let activeResizer: HTMLElement | null = null;
  let containerObserver: ResizeObserver | null = null;
  const repositoryPanelResizer = ref<HTMLElement | null>(null);
  const repositoryPanelAriaValue = ref(minimumListPaneWidth);
  const repositoryPanelAriaMax = ref(minimumListPaneWidth);

  const repositoryPanelStyle = computed<Record<string, string>>(() => ({
    [widthProperty]: repositoryPanelWidth.value === null
      ? defaultWidth
      : `${repositoryPanelWidth.value}px`,
  }));

  function splitContainer(resizer: EventTarget | null): HTMLElement | null {
    return resizer instanceof HTMLElement ? resizer.parentElement : null;
  }

  function currentListPaneWidth(container: HTMLElement): number {
    const listPane = container.firstElementChild;
    return listPane instanceof HTMLElement ? listPane.getBoundingClientRect().width : minimumListPaneWidth;
  }

  function updateRepositoryPanelAriaValues(): void {
    const container = splitContainer(repositoryPanelResizer.value);
    if (!container) return;
    const containerWidth = container.getBoundingClientRect().width;
    repositoryPanelAriaMax.value = maximumListPaneWidth(containerWidth);
    repositoryPanelAriaValue.value = clampListPaneWidth(currentListPaneWidth(container), containerWidth);
  }

  function stopRepositoryPanelResize(event?: Event): void {
    if (!resizing) return;
    if (event instanceof PointerEvent && event.pointerId !== activePointerId) return;
    if (resizeAnimationFrame !== null) {
      window.cancelAnimationFrame(resizeAnimationFrame);
      resizeAnimationFrame = null;
      applyPendingRepositoryPanelResize();
    }
    resizing = false;
    document.documentElement.classList.remove("is-resizing-repository-columns");
    window.removeEventListener("pointermove", resizeRepositoryPanel);
    window.removeEventListener("pointerup", stopRepositoryPanelResize);
    window.removeEventListener("pointercancel", stopRepositoryPanelResize);
    window.removeEventListener("blur", stopRepositoryPanelResize);
    repositoryPanelAriaValue.value = liveListPaneWidth;
    repositoryPanelAriaMax.value = maximumListPaneWidth(resizeContainerWidth);
    repositoryPanelWidth.value = liveListPaneWidth;
    rememberRepositoryPanelWidth();
    if (activeResizer && activePointerId !== null && activeResizer.hasPointerCapture(activePointerId)) {
      activeResizer.releasePointerCapture(activePointerId);
    }
    activePointerId = null;
    activeResizer = null;
  }

  function applyPendingRepositoryPanelResize(): void {
    resizeAnimationFrame = null;
    if (!resizing) return;
    const container = splitContainer(activeResizer);
    if (!container) {
      stopRepositoryPanelResize();
      return;
    }
    liveListPaneWidth = clampListPaneWidth(
      startListPaneWidth + pendingPointerX - startPointerX,
      resizeContainerWidth,
    );
    container.style.setProperty(widthProperty, `${liveListPaneWidth}px`);
  }

  function resizeRepositoryPanel(event: PointerEvent): void {
    if (!resizing || event.pointerId !== activePointerId) return;
    pendingPointerX = event.clientX;
    if (resizeAnimationFrame === null) {
      resizeAnimationFrame = window.requestAnimationFrame(applyPendingRepositoryPanelResize);
    }
  }

  function startRepositoryPanelResize(event: PointerEvent): void {
    if (event.button !== 0 || resizing) return;
    const container = splitContainer(event.currentTarget);
    if (!container) return;
    event.preventDefault();
    resizing = true;
    activePointerId = event.pointerId;
    activeResizer = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
    activeResizer?.setPointerCapture(event.pointerId);
    startPointerX = event.clientX;
    startListPaneWidth = currentListPaneWidth(container);
    liveListPaneWidth = startListPaneWidth;
    pendingPointerX = startPointerX;
    resizeContainerWidth = container.getBoundingClientRect().width;
    repositoryPanelAriaValue.value = clampListPaneWidth(startListPaneWidth, resizeContainerWidth);
    repositoryPanelAriaMax.value = maximumListPaneWidth(resizeContainerWidth);
    document.documentElement.classList.add("is-resizing-repository-columns");
    window.addEventListener("pointermove", resizeRepositoryPanel);
    window.addEventListener("pointerup", stopRepositoryPanelResize);
    window.addEventListener("pointercancel", stopRepositoryPanelResize);
    window.addEventListener("blur", stopRepositoryPanelResize);
  }

  function resizeRepositoryPanelWithKeyboard(event: KeyboardEvent): void {
    const container = splitContainer(event.currentTarget);
    if (!container) return;
    const containerWidth = container.getBoundingClientRect().width;
    const currentWidth = currentListPaneWidth(container);
    const step = event.shiftKey ? 32 : 12;
    let nextWidth: number | null = null;
    if (event.key === "ArrowLeft") nextWidth = currentWidth - step;
    if (event.key === "ArrowRight") nextWidth = currentWidth + step;
    if (event.key === "Home") nextWidth = minimumListPaneWidth;
    if (event.key === "End") nextWidth = maximumListPaneWidth(containerWidth);
    if (nextWidth === null) return;
    event.preventDefault();
    repositoryPanelWidth.value = clampListPaneWidth(nextWidth, containerWidth);
    updateRepositoryPanelAriaValues();
    rememberRepositoryPanelWidth();
  }

  function handleViewportResize(): void {
    if (resizing) stopRepositoryPanelResize();
    else updateRepositoryPanelAriaValues();
  }

  watch(repositoryPanelWidth, () => {
    void nextTick(updateRepositoryPanelAriaValues);
  });

  watch(repositoryPanelResizer, (resizer) => {
    containerObserver?.disconnect();
    const container = splitContainer(resizer);
    if (container) containerObserver?.observe(container);
    void nextTick(updateRepositoryPanelAriaValues);
  });

  onMounted(() => {
    containerObserver = new ResizeObserver(() => {
      if (!resizing) updateRepositoryPanelAriaValues();
    });
    const container = splitContainer(repositoryPanelResizer.value);
    if (container) containerObserver.observe(container);
    window.addEventListener("resize", handleViewportResize);
    void nextTick(updateRepositoryPanelAriaValues);
  });

  onBeforeUnmount(() => {
    containerObserver?.disconnect();
    window.removeEventListener("resize", handleViewportResize);
    stopRepositoryPanelResize();
    if (resizeAnimationFrame !== null) window.cancelAnimationFrame(resizeAnimationFrame);
  });

  return {
    repositoryPanelAriaMax,
    repositoryPanelAriaValue,
    repositoryPanelResizer,
    repositoryPanelStyle,
    resetRememberedRepositoryPanelWidth,
    resizeRepositoryPanelWithKeyboard,
    startRepositoryPanelResize,
  };
}
