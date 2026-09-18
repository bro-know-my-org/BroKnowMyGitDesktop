import { test } from "node:test";
import assert from "node:assert/strict";
import { isResourceDiff, pixelDifference, resourceKind, skipResourceTextDiff } from "./resourcePreview";

test("resource routing preserves SVG source and unknown text while recognizing binary Git diffs", () => {
  assert.equal(resourceKind("assets/ICON.PNG"), "image");
  for (const name of ["png", ".png", "svg", "folder.png/text", "name."]) assert.equal(skipResourceTextDiff(name), false);
  assert.equal(resourceKind("assets/clip.webm"), "video");
  assert.equal(skipResourceTextDiff("icon.svg"), false);
  assert.equal(skipResourceTextDiff("icon.png", "icon.svg"), false);
  assert.equal(skipResourceTextDiff("icon.png", "icon.jpg"), true);
  assert.equal(isResourceDiff("icon.svg", ""), true);
  assert.equal(isResourceDiff("icon.txt", "rename from icon.svg", "icon.svg"), true);
  assert.equal(isResourceDiff("archive.dat", "diff --git a/x b/x\nBinary files a/x and b/x differ"), true);
  assert.equal(isResourceDiff("main.ts", "+const text = 'Binary files x'"), false);
});

test("pixel difference accounts for alpha, unchanged pixels and invisible RGB", () => {
  const before = new Uint8ClampedArray([20, 40, 60, 255, 255, 0, 0, 0, 0, 0, 0, 255]);
  const after = new Uint8ClampedArray([20, 40, 60, 255, 0, 255, 0, 0, 0, 0, 0, 0]);
  const result = pixelDifference(before, after);
  assert.equal(result[3], 0);
  assert.equal(result[7], 0);
  assert.equal(result[11], 255);
  assert.equal(result[8], 255);
  assert.throws(() => pixelDifference(before, new Uint8ClampedArray(4)));
});
