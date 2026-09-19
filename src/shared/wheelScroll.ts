interface FrameScheduler {
  request(callback: FrameRequestCallback): number;
  cancel(id: number): void;
}

type Axis = "x" | "y";
interface ScrollMetadata {
  overflowX: string;
  overflowY: string;
  containmentX: string;
  containmentY: string;
  boundsX: [number, number];
  boundsY: [number, number];
  lineHeight: number;
  pageWidth: number;
  pageHeight: number;
}
interface PendingScroll {
  left: number;
  top: number;
  appliedLeft: number;
  appliedTop: number;
}

// Delegate from document so dynamic lists, dialogs and menus share one frame
// of wheel input instead of queuing WebView smooth-scroll targets.
export function installWheelScroll(
  root: Document = document,
  frames: FrameScheduler = {
    request: (callback) => requestAnimationFrame(callback),
    cancel: (id) => cancelAnimationFrame(id),
  },
): () => void {
  let frame: number | null = null;
  const pending = new Map<HTMLElement, PendingScroll>();
  const metadata = new Map<HTMLElement, ScrollMetadata>();

  function bounds(element: HTMLElement, axis: Axis, style: CSSStyleDeclaration): [number, number] {
    const maximum = Math.max(0, axis === "y"
      ? element.scrollHeight - element.clientHeight
      : element.scrollWidth - element.clientWidth);
    return axis === "x" && style.direction === "rtl" ? [-maximum, 0] : [0, maximum];
  }

  // Snapshot primitive values, since CSSStyleDeclaration itself is live.
  // Reuse only until this frame flushes; current bounds are checked before writing.
  function metadataFor(element: HTMLElement): ScrollMetadata {
    let value = metadata.get(element);
    if (!value) {
      const style = getComputedStyle(element);
      value = {
        overflowX: style.overflowX, overflowY: style.overflowY,
        containmentX: style.overscrollBehaviorX, containmentY: style.overscrollBehaviorY,
        boundsX: bounds(element, "x", style), boundsY: bounds(element, "y", style),
        lineHeight: Number.parseFloat(style.lineHeight) || 16,
        pageWidth: element.clientWidth, pageHeight: element.clientHeight,
      };
      metadata.set(element, value);
    }
    return value;
  }

  function stateFor(element: HTMLElement): PendingScroll {
    const previous = pending.get(element);
    if (previous) {
      if (previous.appliedTop !== element.scrollTop) {
        previous.top = previous.appliedTop = element.scrollTop;
      }
      if (previous.appliedLeft !== element.scrollLeft) {
        previous.left = previous.appliedLeft = element.scrollLeft;
      }
      return previous;
    }
    return {
      top: element.scrollTop, left: element.scrollLeft,
      appliedTop: element.scrollTop, appliedLeft: element.scrollLeft,
    };
  }

  function queueAxis(
    elements: HTMLElement[], axis: Axis, delta: number, mode: number,
  ): boolean {
    if (!Number.isFinite(delta) || delta === 0) return false;
    let remaining = delta;
    let convertedToPixels = false;
    let consumed = false;
    for (const element of elements) {
      const info = metadataFor(element);
      const overflow = axis === "y" ? info.overflowY : info.overflowX;
      if (!/^(auto|scroll|overlay)$/.test(overflow)
        && !(element === root.scrollingElement && overflow === "visible")) continue;
      const [minimum, maximum] = axis === "y" ? info.boundsY : info.boundsX;
      const state = stateFor(element);
      const position = axis === "y" ? state.top : state.left;
      const containment = axis === "y" ? info.containmentY : info.containmentX;
      const contained = containment === "contain" || containment === "none";
      // Keep the nearest eligible scroller's units even when it starts at its boundary.
      if (!convertedToPixels) {
        let unit = 1;
        if (mode === 1) unit = info.lineHeight;
        else if (mode === 2) unit = axis === "y" ? info.pageHeight : info.pageWidth;
        remaining *= unit;
        convertedToPixels = true;
      }
      const canMove = remaining > 0 ? position < maximum : position > minimum;
      if (!canMove) {
        if (contained) return true;
        continue;
      }

      const next = Math.max(minimum, Math.min(maximum, position + remaining));
      if (axis === "y") state.top = next;
      else state.left = next;
      if (state.top === state.appliedTop && state.left === state.appliedLeft) pending.delete(element);
      else pending.set(element, state);
      consumed = true;
      remaining -= next - position;
      if (contained || remaining === 0) return true;
    }
    return consumed;
  }

  function flush(): void {
    frame = null;
    for (const [element, state] of pending) {
      if (!element.isConnected) continue;
      const moveX = state.left !== state.appliedLeft && element.scrollLeft === state.appliedLeft;
      const moveY = state.top !== state.appliedTop && element.scrollTop === state.appliedTop;
      if (!moveX && !moveY) continue;
      const style = getComputedStyle(element);
      const options: ScrollToOptions = { behavior: "instant" };
      if (moveX) {
        const [minX, maxX] = bounds(element, "x", style);
        options.left = Math.max(minX, Math.min(maxX, state.left));
      }
      if (moveY) {
        const [, maxY] = bounds(element, "y", style);
        options.top = Math.max(0, Math.min(maxY, state.top));
      }
      element.scrollTo(options);
    }
    pending.clear();
    metadata.clear();
  }

  function onWheel(event: WheelEvent): void {
    // Bubble after component handlers, respecting image zoom and custom gestures.
    if (event.defaultPrevented || !event.cancelable || event.ctrlKey || event.metaKey) return;
    const elements = event.composedPath().filter((node): node is HTMLElement => node instanceof HTMLElement);
    const horizontal = event.shiftKey && event.deltaX === 0 ? event.deltaY : event.deltaX;
    const vertical = event.shiftKey && event.deltaX === 0 ? 0 : event.deltaY;
    const movedX = queueAxis(elements, "x", horizontal, event.deltaMode);
    const movedY = queueAxis(elements, "y", vertical, event.deltaMode);
    if (pending.size === 0) metadata.clear();
    if (!movedX && !movedY) return;
    event.preventDefault();
    if (pending.size > 0 && frame === null) frame = frames.request(flush);
    else if (pending.size === 0 && frame !== null) {
      frames.cancel(frame);
      frame = null;
    }
  }

  root.addEventListener("wheel", onWheel, { passive: false });
  return () => {
    if (frame !== null) frames.cancel(frame);
    pending.clear();
    metadata.clear();
    root.removeEventListener("wheel", onWheel);
  };
}
