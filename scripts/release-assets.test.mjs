import assert from "node:assert/strict";
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { collectReleaseAssets, verifyReleaseAssets } from "./release-assets.mjs";

const combinations = [
  ["x86_64-pc-windows-msvc", [["nsis", ".exe"]]],
  ["aarch64-pc-windows-msvc", [["nsis", ".exe"]]],
  ["x86_64-unknown-linux-gnu", [["deb", ".deb"], ["rpm", ".rpm"], ["appimage", ".AppImage"]]],
  ["aarch64-unknown-linux-gnu", [["deb", ".deb"], ["rpm", ".rpm"], ["appimage", ".AppImage"]]],
  ["x86_64-apple-darwin", [["dmg", ".dmg"]]],
  ["aarch64-apple-darwin", [["dmg", ".dmg"]]],
];

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "bkm-release-assets-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

async function createBundles(root, target, formats) {
  for (const [directory, extension] of formats) {
    const source = path.join(root, "src-tauri", "target", target, "release", "bundle", directory);
    await mkdir(source, { recursive: true });
    // Deliberately identical names test that architectures cannot overwrite one another.
    await writeFile(path.join(source, `BroKnowMyGitDesktop_1.2.3${extension}`), target);
  }
}

test("collects and merges all six targets without losing installers with identical names", async (t) => {
  const root = await fixture(t);
  const merged = path.join(root, "release-assets");
  await mkdir(merged);
  for (const [target, formats] of combinations) {
    const job = path.join(root, target);
    await createBundles(job, target, formats);
    assert.equal(await collectReleaseAssets(job, target), formats.length);
    const collected = path.join(job, "release-assets");
    for (const name of await readdir(collected)) {
      assert.equal(await readFile(path.join(collected, name), "utf8"), target);
    }
    await cp(collected, merged, { recursive: true });
  }
  assert.equal((await readdir(merged)).length, 10);
  assert.equal(await verifyReleaseAssets(root), 10);
  await writeFile(path.join(merged, "SHA256SUMS.txt"), "checksum fixture");
  assert.equal(await verifyReleaseAssets(root), 10);

  const missing = "aarch64-apple-darwin-BroKnowMyGitDesktop_1.2.3.dmg";
  await rm(path.join(merged, missing));
  await assert.rejects(verifyReleaseAssets(root), /aarch64-apple-darwin \.dmg.*found 0/);
  await writeFile(path.join(merged, missing), "restored");
  await writeFile(path.join(merged, "unexpected.exe"), "stale installer");
  await assert.rejects(verifyReleaseAssets(root), /Unexpected assets/);
});

test("refuses missing, empty, duplicate or stale installers and unsupported targets", async (t) => {
  const root = await fixture(t);
  const target = "aarch64-unknown-linux-gnu";
  const formats = combinations.find(([candidate]) => candidate === target)[1];
  await createBundles(root, target, formats);
  const rpmDirectory = path.join(root, "src-tauri", "target", target, "release", "bundle", "rpm");
  const rpm = path.join(rpmDirectory, "BroKnowMyGitDesktop_1.2.3.rpm");
  await rm(rpm);
  await assert.rejects(collectReleaseAssets(root, target), /\.rpm.*found 0/);
  await writeFile(rpm, "");
  await assert.rejects(collectReleaseAssets(root, target), /Empty installer/);
  await writeFile(rpm, "installer");
  const duplicate = path.join(rpmDirectory, "duplicate.rpm");
  await writeFile(duplicate, "duplicate");
  await assert.rejects(collectReleaseAssets(root, target), /\.rpm.*found 2/);
  await rm(duplicate);
  assert.equal(await collectReleaseAssets(root, target), 3);
  await assert.rejects(collectReleaseAssets(root, target), /must be empty/);
  await assert.rejects(collectReleaseAssets(root, "armv7-unknown-linux-gnu"), /Unsupported release target/);
});
