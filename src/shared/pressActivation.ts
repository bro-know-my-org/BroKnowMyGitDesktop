const releaseActivationSelector = [
  ".n-dialog__action button",
  ".n-card__action button",
  "[data-activate-on-release]",
].join(", ");
const pressActivationSelector = [
  "button",
  '[role="button"]',
  ".n-dropdown-option-body",
  ".n-base-selection",
  ".n-base-select-option",
  ".change-file-row",
].join(", ");

function closestActivationTarget(target: EventTarget | null): HTMLElement | null {
  if (!(target instanceof Element)) return null;
  return target.closest<HTMLElement>(pressActivationSelector);
}

function activatableTarget(target: EventTarget | null): HTMLElement | null {
  const activationTarget = closestActivationTarget(target);
  const eventElement = target instanceof Element ? target : null;
  if (
    !activationTarget ||
    (activationTarget instanceof HTMLButtonElement && activationTarget.disabled) ||
    activationTarget.getAttribute("aria-disabled") === "true" ||
    activationTarget.matches(releaseActivationSelector) ||
    activationTarget.classList.contains("n-dropdown-option-body--disabled") ||
    activationTarget.classList.contains("n-dropdown-option-body--group") ||
    activationTarget.classList.contains("n-base-selection--disabled") ||
    activationTarget.classList.contains("n-base-select-option--disabled") ||
    (activationTarget.classList.contains("change-file-row") && Boolean(eventElement?.closest(".change-file-checkbox")))
  ) return null;
  return activationTarget;
}

export function installPressActivation(): () => void {
  const pointerTargets = new Map<number, HTMLElement>();
  const releasedPointerIds: number[] = [];

  const forgetReleasedPointer = (pointerId: number) => {
    const index = releasedPointerIds.indexOf(pointerId);
    if (index !== -1) releasedPointerIds.splice(index, 1);
  };

  const clearPendingPointer = (pointerId: number) => {
    pointerTargets.delete(pointerId);
    forgetReleasedPointer(pointerId);
  };

  const clearAllPendingPointers = () => {
    pointerTargets.clear();
    releasedPointerIds.length = 0;
  };

  const handlePointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || !event.isPrimary) return;
    const activationTarget = activatableTarget(event.target);
    if (!activationTarget) return;
    pointerTargets.set(event.pointerId, activationTarget);
    // Preserve the component's existing click handler and keyboard behavior,
    // but dispatch the pointer activation after the target's pointerdown
    // handlers have run instead of waiting for pointer release.
    queueMicrotask(() => {
      if (
        event.defaultPrevented ||
        !activationTarget.isConnected ||
        activatableTarget(activationTarget) !== activationTarget
      ) {
        clearPendingPointer(event.pointerId);
        return;
      }
      if (activationTarget instanceof HTMLButtonElement) {
        activationTarget.focus({ preventScroll: true });
      }
      activationTarget.click();
    });
  };

  const handleClick = (event: MouseEvent) => {
    // Programmatic and keyboard clicks must keep their ordinary behavior.
    if (!event.isTrusted || event.detail === 0) return;
    const pointerId = event instanceof PointerEvent
      ? event.pointerId
      : releasedPointerIds[0];
    if (pointerId === undefined || !pointerTargets.has(pointerId)) return;
    clearPendingPointer(pointerId);
    event.preventDefault();
    event.stopImmediatePropagation();
  };

  const handlePointerUp = (event: PointerEvent) => {
    const activationTarget = pointerTargets.get(event.pointerId);
    if (!activationTarget) return;
    if (!releasedPointerIds.includes(event.pointerId)) releasedPointerIds.push(event.pointerId);
    // A native click follows pointerup in the same task. Clear only afterward
    // so that click can be suppressed without affecting later keyboard input.
    setTimeout(() => {
      if (pointerTargets.get(event.pointerId) === activationTarget) {
        clearPendingPointer(event.pointerId);
      }
    }, 0);
  };

  const handlePointerCancel = (event: PointerEvent) => {
    clearPendingPointer(event.pointerId);
  };

  const handleVisibilityChange = () => {
    if (document.hidden) clearAllPendingPointers();
  };

  document.addEventListener("pointerdown", handlePointerDown, true);
  document.addEventListener("click", handleClick, true);
  document.addEventListener("pointerup", handlePointerUp, true);
  document.addEventListener("pointercancel", handlePointerCancel, true);
  document.addEventListener("lostpointercapture", handlePointerCancel, true);
  document.addEventListener("visibilitychange", handleVisibilityChange);
  window.addEventListener("blur", clearAllPendingPointers);

  return () => {
    document.removeEventListener("pointerdown", handlePointerDown, true);
    document.removeEventListener("click", handleClick, true);
    document.removeEventListener("pointerup", handlePointerUp, true);
    document.removeEventListener("pointercancel", handlePointerCancel, true);
    document.removeEventListener("lostpointercapture", handlePointerCancel, true);
    document.removeEventListener("visibilitychange", handleVisibilityChange);
    window.removeEventListener("blur", clearAllPendingPointers);
  };
}
