import assert from "node:assert/strict";
import test from "node:test";
import { titlebarGesture } from "./windowGestures";

test("titlebar gestures leave nested controls, typing and repository pickers interactive", (context) => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "Element");
  class TestElement {
    constructor(private ancestors: string[] = []) {}
    closest(selector: string) {
      return selector.split(", ").some((candidate) => this.ancestors.includes(candidate)) ? this : null;
    }
  }
  Object.defineProperty(globalThis, "Element", { configurable: true, value: TestElement });
  context.after(() => {
    if (original) Object.defineProperty(globalThis, "Element", original);
    else Reflect.deleteProperty(globalThis, "Element");
  });
  const mouse = (target: unknown, detail = 1, button = 0, defaultPrevented = false) =>
    ({ target, detail, button, defaultPrevented }) as MouseEvent;

  // An SVG path inside any of these ancestors must never move or maximize the window.
  for (const ancestor of ["button", "input", ".n-base-selection", '[role="combobox"]', "[contenteditable]", "[data-window-interactive]"]) {
    for (const detail of [1, 2]) assert.equal(titlebarGesture(mouse(new TestElement([ancestor]), detail)), null);
  }
  const blank = new TestElement();
  assert.equal(titlebarGesture(mouse(blank)), "drag");
  assert.equal(titlebarGesture(mouse(blank, 2)), "maximize");
  assert.equal(titlebarGesture(mouse(blank, 1, 1)), null);
  assert.equal(titlebarGesture(mouse(blank, 1, 2)), null);
  assert.equal(titlebarGesture(mouse(blank, 1, 0, true)), null);
  assert.equal(titlebarGesture(mouse(null)), null);
});
