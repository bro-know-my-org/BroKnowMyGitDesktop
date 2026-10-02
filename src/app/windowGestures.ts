const interactiveSelector = 'button, a, input, select, textarea, [role="button"], [role="combobox"], [contenteditable], .n-base-selection, [data-window-interactive]';

/** Keep repository pickers and window buttons out of native titlebar gestures. */
export function titlebarGesture(event: MouseEvent): "drag" | "maximize" | null {
  if (event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) return null;
  if (event.target.closest(interactiveSelector)) return null;
  return event.detail === 2 ? "maximize" : "drag";
}
