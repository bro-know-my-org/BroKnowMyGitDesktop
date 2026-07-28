import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const repositoryPanelWidthStorageKey = "broknowmygitdesktop.repository-panel-width";
const minimumListPaneWidth = 240;
const minimumDetailPaneWidth = 480;
const resizerWidth = 12;

function initialRepositoryPanelWidth(): number | null {
  if (typeof window === "undefined") return null;
  try {
    const saved = Number.parseFloat(window.localStorage.getItem(repositoryPanelWidthStorageKey) ?? "");
    return Number.isFinite(saved) && saved >= minimumListPaneWidth ? Math.round(saved) : null;
  } catch {
    return null;
  }
}

const repositoryPanelWidth = ref<number | null>(initialRepositoryPanelWidth());

function maximumListPaneWidth(containerWidth: number): number {
  return Math.max(minimumListPaneWidth, containerWidth - minimumDetailPaneWidth - resizerWidth);
}

function clampListPaneWidth(width: number, containerWidth: number): number {
  return Math.round(Math.min(Math.max(width, minimumListPaneWidth), maximumListPaneWidth(containerWidth)));
}

function rememberRepositoryPanelWidth(): void {
  if (typeof window === "undefined" || repositoryPanelWidth.value === null) return;
  try {
    window.localStorage.setItem(repositoryPanelWidthStorageKey, String(repositoryPanelWidth.value));
  } catch {
    // The resized width still applies for this session if storage is unavailable.
  }
}

function resetRememberedRepositoryPanelWidth(): void {
  repositoryPanelWidth.value = null;
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(repositoryPanelWidthStorageKey);
  } catch {
    // Ignore unavailable browser storage; the in-memory value has still been reset.
  }
}

export function useRepositoryPanelSplit() {
  let resizing = false;
  let startPointerX = 0;
  let startListPaneWidth = 0;
  let resizeContainerWidth = 0;
  let activePointerId: number | null = null;
  let activeResizer: HTMLElement | null = null;
  const repositoryPanelResizer = ref<HTMLElement | null>(null);
  const repositoryPanelAriaValue = ref(minimumListPaneWidth);
  const repositoryPanelAriaMax = ref(minimumListPaneWidth);

  const repositoryPanelStyle = computed<Record<string, string>>(() => ({
    "--repository-list-pane-width": repositoryPanelWidth.value === null
      ? "27.2%"
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
    resizing = false;
    document.documentElement.classList.remove("is-resizing-repository-columns");
    window.removeEventListener("pointermove", resizeRepositoryPanel);
    window.removeEventListener("pointerup", stopRepositoryPanelResize);
    window.removeEventListener("pointercancel", stopRepositoryPanelResize);
    window.removeEventListener("blur", stopRepositoryPanelResize);
    updateRepositoryPanelAriaValues();
    repositoryPanelWidth.value = repositoryPanelAriaValue.value;
    rememberRepositoryPanelWidth();
    if (activeResizer && activePointerId !== null && activeResizer.hasPointerCapture(activePointerId)) {
      activeResizer.releasePointerCapture(activePointerId);
    }
    activePointerId = null;
    activeResizer = null;
  }

  function resizeRepositoryPanel(event: PointerEvent): void {
    if (!resizing || event.pointerId !== activePointerId) return;
    const container = splitContainer(activeResizer);
    if (!container) {
      stopRepositoryPanelResize();
      return;
    }
    resizeContainerWidth = container.getBoundingClientRect().width;
    repositoryPanelWidth.value = clampListPaneWidth(
      startListPaneWidth + event.clientX - startPointerX,
      resizeContainerWidth,
    );
    repositoryPanelAriaValue.value = repositoryPanelWidth.value;
    repositoryPanelAriaMax.value = maximumListPaneWidth(resizeContainerWidth);
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
    resizeContainerWidth = container.getBoundingClientRect().width;
    updateRepositoryPanelAriaValues();
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

  onMounted(() => {
    window.addEventListener("resize", handleViewportResize);
    void nextTick(updateRepositoryPanelAriaValues);
  });

  onBeforeUnmount(() => {
    window.removeEventListener("resize", handleViewportResize);
    stopRepositoryPanelResize();
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
