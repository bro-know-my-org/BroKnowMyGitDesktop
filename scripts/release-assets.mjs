import { constants } from "node:fs";
import { copyFile, mkdir, readdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const bundles = {
  "pc-windows-msvc": [["nsis", ".exe"]],
  "unknown-linux-gnu": [["deb", ".deb"], ["rpm", ".rpm"], ["appimage", ".AppImage"]],
  "apple-darwin": [["dmg", ".dmg"]],
};
const targets = ["x86_64", "aarch64"].flatMap((architecture) =>
  Object.keys(bundles).map((platform) => `${architecture}-${platform}`),
);

function formatsForTarget(target) {
  if (!targets.includes(target)) throw new Error(`Unsupported release target: ${target}`);
  return bundles[target.replace(/^(x86_64|aarch64)-/, "")];
}

async function requireInstaller(directory, names, description) {
  if (names.length !== 1) {
    throw new Error(`Expected exactly one ${description} installer; found ${names.length}.`);
  }
  const installer = path.join(directory, names[0]);
  if ((await stat(installer)).size === 0) throw new Error(`Empty installer: ${installer}`);
  return installer;
}

export async function collectReleaseAssets(repositoryRoot, target) {
  const formats = formatsForTarget(target);
  const bundleRoot = path.join(repositoryRoot, "src-tauri", "target", target, "release", "bundle");
  const installers = [];
  for (const [directory, extension] of formats) {
    const source = path.join(bundleRoot, directory);
    const entries = await readdir(source, { withFileTypes: true });
    const names = entries.filter((entry) => entry.isFile() && entry.name.endsWith(extension))
      .map((entry) => entry.name);
    installers.push(await requireInstaller(source, names, `${target} ${extension}`));
  }

  const destination = path.join(repositoryRoot, "release-assets");
  await mkdir(destination, { recursive: true });
  if ((await readdir(destination)).length !== 0) {
    throw new Error("release-assets must be empty before collecting installers.");
  }
  for (const installer of installers) {
    // Namespace filenames as well as artifacts: download-artifact merges all six jobs.
    await copyFile(
      installer,
      path.join(destination, `${target}-${path.basename(installer)}`),
      constants.COPYFILE_EXCL,
    );
  }
  return installers.length;
}

export async function verifyReleaseAssets(repositoryRoot) {
  const directory = path.join(repositoryRoot, "release-assets");
  const entries = await readdir(directory, { withFileTypes: true });
  const names = entries.filter((entry) => entry.isFile() && entry.name !== "SHA256SUMS.txt")
    .map((entry) => entry.name);
  const expected = new Set();
  for (const target of targets) {
    for (const [, extension] of formatsForTarget(target)) {
      const matches = names.filter((name) => name.startsWith(`${target}-`) && name.endsWith(extension));
      await requireInstaller(directory, matches, `${target} ${extension}`);
      expected.add(matches[0]);
    }
  }
  if (entries.some((entry) => entry.name !== "SHA256SUMS.txt" && !expected.has(entry.name))) {
    throw new Error("Unexpected assets in release-assets.");
  }
  return expected.size;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  try {
    const [command, ...args] = process.argv.slice(2);
    if (command === "collect" && args.length === 1) {
      console.log(`Collected ${await collectReleaseAssets(process.cwd(), args[0])} installers for ${args[0]}.`);
    } else if (command === "verify" && args.length === 0) {
      console.log(`Verified ${await verifyReleaseAssets(process.cwd())} installers across all six release targets.`);
    } else {
      throw new Error("Usage: release-assets.mjs collect <target> | verify");
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
