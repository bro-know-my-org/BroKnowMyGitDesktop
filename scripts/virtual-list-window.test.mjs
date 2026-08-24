import assert from "node:assert/strict";
import test from "node:test";

import { calculateVirtualWindow, preserveLogicalScrollTop } from "../src/shared/virtual-list/window.mjs";

const itemCount = 8_000;
const itemHeight = 40;
const viewportHeight = 600;
const overscanViewports = 3;

function windowAt(scrollTop) {
  return calculateVirtualWindow({ itemCount, itemHeight, scrollTop, viewportHeight, overscanViewports });
}

test("8,000 items only render the viewport plus a bounded wide overscan", () => {
  for (const scrollTop of [0, 4_000, 120_000, itemCount * itemHeight - viewportHeight]) {
    const window = windowAt(scrollTop);
    assert.ok(window.end - window.start <= 106);
    assert.ok(window.start * itemHeight <= scrollTop);
    assert.ok(window.end * itemHeight >= scrollTop + viewportHeight);
  }
});

test("virtual window includes partially visible rows at non-aligned offsets", () => {
  const scrollTop = 120_001;
  const window = windowAt(scrollTop);

  assert.deepEqual(window, { start: 2_955, end: 3_061 });
  assert.ok(window.start * itemHeight <= scrollTop - viewportHeight * overscanViewports);
  assert.ok(window.end * itemHeight >= scrollTop + viewportHeight * (overscanViewports + 1));
});

test("virtual window clamps cleanly at the top and bottom", () => {
  assert.deepEqual(windowAt(0), { start: 0, end: 60 });
  assert.deepEqual(windowAt(itemCount * itemHeight), { start: 7_940, end: 8_000 });
});

test("empty and temporarily unmeasured lists render no items", () => {
  assert.deepEqual(calculateVirtualWindow({ itemCount: 0, itemHeight, scrollTop: 0, viewportHeight, overscanViewports }), { start: 0, end: 0 });
  assert.deepEqual(calculateVirtualWindow({ itemCount, itemHeight: 0, scrollTop: 0, viewportHeight, overscanViewports }), { start: 0, end: 0 });
  assert.deepEqual(calculateVirtualWindow({ itemCount, itemHeight, scrollTop: 0, viewportHeight: 0, overscanViewports }), { start: 0, end: 0 });
});

test("changing row height preserves the same logical scroll position", () => {
  assert.equal(preserveLogicalScrollTop(120_020, 40, 62), 186_031);
  assert.equal(preserveLogicalScrollTop(-20, 40, 62), 0);
});
