import assert from "node:assert/strict";
import test, { after } from "node:test";
import { installWheelScroll } from "./wheelScroll";

class ScrollElement extends EventTarget {
  scrollTop = 0;
  scrollLeft = 0;
  scrollHeight = 100_000;
  clientHeight = 600;
  scrollWidth = 800;
  clientWidth = 800;
  isConnected = true;
  styleReads = 0;
  writes: ScrollToOptions[] = [];
  style = {
    overflowY: "auto", overflowX: "auto", direction: "ltr", lineHeight: "normal",
    overscrollBehaviorY: "auto", overscrollBehaviorX: "auto",
  };
  scrollTo(options: ScrollToOptions) {
    this.writes.push(options);
    if (options.top !== undefined) this.scrollTop = options.top;
    if (options.left !== undefined) this.scrollLeft = options.left;
  }
}

const globals = {
  HTMLElement: ScrollElement,
  getComputedStyle: (element: ScrollElement) => {
    element.styleReads++;
    return element.style;
  },
};
for (const [name, value] of Object.entries(globals)) {
  const original = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, { value, configurable: true });
  after(() => {
    if (original) Object.defineProperty(globalThis, name, original);
    else Reflect.deleteProperty(globalThis, name);
  });
}

function setup() {
  const callbacks = new Map<number, FrameRequestCallback>();
  let nextId = 0;
  const root = new EventTarget();
  const viewport = new ScrollElement();
  const writes = viewport.writes;
  const dispose = installWheelScroll(root as unknown as Document, {
    request(callback) { callbacks.set(++nextId, callback); return nextId; },
    cancel(id) { callbacks.delete(id); },
  });
  function wheel(deltaY: number, options: Record<string, unknown> = {}, path = [viewport]) {
    const event = Object.assign(new Event("wheel", { cancelable: true }), {
      deltaY, deltaX: 0, deltaMode: 0, ctrlKey: false, metaKey: false, shiftKey: false,
      composedPath: () => path, ...options,
    });
    if (options.consumed) event.preventDefault();
    root.dispatchEvent(event);
    return event;
  }
  function paint() {
    const pending = [...callbacks.values()];
    callbacks.clear();
    for (const callback of pending) callback(0);
  }
  return { viewport, callbacks, writes, dispose, wheel, paint };
}

test("high-frequency wheel input advances every frame without a tail after stopping", () => {
  const h = setup();
  for (let frame = 1; frame <= 20; frame++) {
    for (let event = 0; event < 40; event++) assert.ok(h.wheel(3).defaultPrevented);
    assert.equal(h.callbacks.size, 1);
    h.paint();
    assert.equal(h.viewport.scrollTop, frame * 120);
    h.viewport.dispatchEvent(new Event("scroll"));
  }
  h.paint();
  assert.equal(h.writes.length, 20);
  assert.ok(h.writes.every((write) => write.behavior === "instant"));
});

test("edge overshoot does not accumulate and reverse input takes effect immediately", () => {
  const h = setup();
  h.wheel(200_000);
  h.wheel(-30);
  h.paint();
  assert.equal(h.viewport.scrollTop, 99_370);
  h.wheel(-200_000);
  h.wheel(30);
  h.paint();
  assert.equal(h.viewport.scrollTop, 30);
});

test("pixel fractions, line units and page units retain their distance", () => {
  const h = setup();
  h.wheel(0.5);
  h.wheel(2, { deltaMode: 1 });
  h.wheel(1, { deltaMode: 2 });
  h.paint();
  assert.equal(h.viewport.scrollTop, 632.5);
});

test("zoom remains native; non-scrollable lists do not consume input", () => {
  const h = setup();
  for (const options of [{ ctrlKey: true }, { metaKey: true }]) {
    assert.equal(h.wheel(20, options).defaultPrevented, false);
  }
  h.viewport.scrollHeight = 600;
  assert.equal(h.wheel(20).defaultPrevented, false);
  assert.equal(h.callbacks.size, 0);
});

test("external position changes take priority even before their scroll event is delivered", () => {
  const h = setup();
  h.wheel(100);
  h.viewport.scrollTop = 500;
  h.paint();
  assert.equal(h.viewport.scrollTop, 500);
  h.wheel(20);
  h.paint();
  assert.equal(h.viewport.scrollTop, 520);
});

test("a delayed scroll event from our own write does not discard new input", () => {
  const h = setup();
  h.wheel(100);
  h.paint();
  h.wheel(100);
  h.viewport.dispatchEvent(new Event("scroll"));
  h.paint();
  assert.equal(h.viewport.scrollTop, 200);
});

test("external position changes cancel pending input, and disposal cancels frames", () => {
  const h = setup();
  h.wheel(100);
  h.viewport.scrollTop = 500;
  h.viewport.dispatchEvent(new Event("scroll"));
  h.paint();
  assert.equal(h.viewport.scrollTop, 500);
  h.wheel(20);
  h.dispose();
  h.paint();
  assert.equal(h.viewport.scrollTop, 500);
  assert.equal(h.wheel(20).defaultPrevented, false);
});

test("nested scrolling selects the closest scroller and chains at an auto boundary", () => {
  const h = setup();
  const inner = new ScrollElement();
  inner.scrollHeight = 700;
  h.wheel(100, {}, [inner, h.viewport]);
  h.paint();
  assert.equal(inner.scrollTop, 100);
  assert.equal(h.viewport.scrollTop, 0);
  h.wheel(30, {}, [inner, h.viewport]);
  h.paint();
  assert.equal(h.viewport.scrollTop, 30);
  inner.style.overscrollBehaviorY = "contain";
  h.wheel(30, {}, [inner, h.viewport]);
  h.paint();
  assert.equal(h.viewport.scrollTop, 30);
  h.wheel(-10, {}, [inner, h.viewport]);
  h.paint();
  assert.equal(inner.scrollTop, 90);
});

test("hidden overflow wrappers are skipped and dynamic popup elements work", () => {
  const h = setup();
  const popup = new ScrollElement();
  const wrapper = new ScrollElement();
  wrapper.style.overflowY = "hidden";
  h.wheel(35, {}, [wrapper, popup]);
  h.paint();
  assert.equal(wrapper.scrollTop, 0);
  assert.equal(popup.scrollTop, 35);
  assert.equal(h.viewport.scrollTop, 0);
});

test("horizontal, shift-wheel, diagonal and RTL input use the appropriate axes", () => {
  const h = setup();
  h.viewport.scrollWidth = 2000;
  h.wheel(0, { deltaX: 50 });
  h.wheel(30, { shiftKey: true });
  h.wheel(20, { deltaX: 10 });
  h.paint();
  assert.equal(h.viewport.scrollTop, 20);
  assert.equal(h.viewport.scrollLeft, 90);
  h.viewport.style.direction = "rtl";
  h.viewport.scrollLeft = 0;
  h.wheel(0, { deltaX: -50 });
  h.paint();
  assert.equal(h.viewport.scrollLeft, -50);
});

test("component-consumed gestures, detached elements and shrinking content are safe", () => {
  const h = setup();
  h.wheel(30, { consumed: true });
  assert.equal(h.callbacks.size, 0);
  h.wheel(100);
  h.viewport.isConnected = false;
  h.paint();
  assert.equal(h.viewport.scrollTop, 0);
  h.viewport.isConnected = true;
  h.wheel(100);
  h.viewport.scrollHeight = 650;
  h.paint();
  assert.equal(h.viewport.scrollTop, 50);
});

test("diagonal input reads each ancestor style only once per event", () => {
  const h = setup();
  h.viewport.scrollWidth = 2000;
  const wrapper = new ScrollElement();
  wrapper.style.overflowX = "hidden";
  wrapper.style.overflowY = "hidden";
  h.wheel(30, { deltaX: 20 }, [wrapper, h.viewport]);
  assert.equal(wrapper.styleReads, 1);
  assert.equal(h.viewport.styleReads, 1);
  h.paint();
  assert.equal(h.viewport.scrollLeft, 20);
  assert.equal(h.viewport.scrollTop, 30);
});

test("contained boundaries consume input without scheduling no-op frames", () => {
  const h = setup();
  h.viewport.style.overscrollBehaviorY = "contain";
  for (let i = 0; i < 100; i++) assert.ok(h.wheel(-30).defaultPrevented);
  assert.equal(h.callbacks.size, 0);
  h.paint();
  assert.equal(h.writes.length, 0);
  h.wheel(30);
  h.wheel(-30);
  assert.equal(h.callbacks.size, 0);
});

test("boundary remainders reach ancestors unless containment stops them", () => {
  const h = setup();
  const inner = new ScrollElement();
  inner.scrollHeight = 610;
  inner.style.lineHeight = "20px";
  h.viewport.style.lineHeight = "40px";
  h.wheel(2, { deltaMode: 1 }, [inner, h.viewport]);
  h.paint();
  assert.equal(inner.scrollTop, 10);
  assert.equal(h.viewport.scrollTop, 30);
  inner.scrollTop = 0;
  inner.style.overscrollBehaviorY = "none";
  h.wheel(40, {}, [inner, h.viewport]);
  h.paint();
  assert.equal(inner.scrollTop, 10);
  assert.equal(h.viewport.scrollTop, 30);
});

test("a wheel burst reuses geometry within its frame and refreshes it next frame", () => {
  const h = setup();
  let geometryReads = 0;
  Object.defineProperty(h.viewport, "scrollHeight", {
    get() { geometryReads++; return 100_000; },
  });
  for (let i = 0; i < 100; i++) h.wheel(2);
  assert.equal(h.viewport.styleReads, 1);
  assert.equal(geometryReads, 1);
  h.paint();
  assert.equal(h.viewport.scrollTop, 200);
  assert.equal(h.viewport.styleReads, 2);
  assert.equal(geometryReads, 2);
  for (let i = 0; i < 100; i++) h.wheel(2);
  assert.equal(h.viewport.styleReads, 3);
  assert.equal(geometryReads, 3);
  h.paint();
  assert.equal(h.viewport.scrollTop, 400);
});

test("line/page units stay anchored to an inner scroller already at its boundary", () => {
  for (const [mode, expected] of [[1, 40], [2, 200]]) {
    const h = setup();
    const inner = new ScrollElement();
    inner.clientHeight = 100;
    inner.scrollHeight = 110;
    inner.scrollTop = 10;
    inner.style.lineHeight = "20px";
    h.viewport.style.lineHeight = "40px";
    h.wheel(2, { deltaMode: mode }, [inner, h.viewport]);
    h.paint();
    assert.equal(h.viewport.scrollTop, expected);
  }
});

test("external changes on one axis preserve pending input on the other axis", () => {
  const h = setup();
  h.viewport.scrollWidth = 2000;
  h.wheel(20, { deltaX: 10 });
  h.viewport.scrollLeft = 100;
  h.wheel(20);
  h.paint();
  assert.equal(h.viewport.scrollTop, 40);
  assert.equal(h.viewport.scrollLeft, 100);
  h.wheel(20, { deltaX: 10 });
  h.viewport.scrollTop = 200;
  h.paint();
  assert.equal(h.viewport.scrollTop, 200);
  assert.equal(h.viewport.scrollLeft, 110);
});
