import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseReleaseTag } from "./release-version-utils.mjs";

const tag = process.env.RELEASE_TAG ?? process.argv[2];
const expectedVersion = parseReleaseTag(tag);

if (!expectedVersion || expectedVersion === "0.0.0") {
  console.error("Release tag must use a non-placeholder stable vX.Y.Z format (for example, v0.1.0).");
  process.exit(1);
}

const repositoryRoot = process.env.RELEASE_REPOSITORY_ROOT
  ? path.resolve(process.env.RELEASE_REPOSITORY_ROOT)
  : fileURLToPath(new URL("../", import.meta.url));

async function readJsonFile(relativePath) {
  try {
    return JSON.parse(await readFile(path.join(repositoryRoot, relativePath), "utf8"));
  } catch (error) {
    throw new Error(`Could not read valid JSON from ${relativePath}: ${error.message}`);
  }
}

try {
  const packageJson = await readJsonFile("package.json");
  const tauriConfig = await readJsonFile(path.join("src-tauri", "tauri.conf.json"));
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      [
        "metadata",
        "--manifest-path",
        path.join(repositoryRoot, "src-tauri", "Cargo.toml"),
        "--format-version",
        "1",
        "--no-deps",
        "--locked",
      ],
      {
        cwd: repositoryRoot,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "inherit"],
        maxBuffer: 64 * 1024 * 1024,
      },
    ),
  );
  const cargoManifestPath = await realpath(path.join(repositoryRoot, "src-tauri", "Cargo.toml"));
  const cargoPackage = metadata.packages.find(
    (item) => realpathSync(item.manifest_path) === cargoManifestPath,
  );
  if (!cargoPackage) throw new Error(`Cargo metadata does not contain ${cargoManifestPath}.`);

  const versions = [
    ["package.json", packageJson.version],
    ["src-tauri/Cargo.toml", cargoPackage.version],
    ["src-tauri/tauri.conf.json", tauriConfig.version],
  ];
  const mismatches = versions.filter(([, version]) => version !== expectedVersion);
  if (mismatches.length > 0) {
    const details = versions.map(([file, version]) => `${file}: ${String(version)}`).join(", ");
    throw new Error(`Release tag ${tag} expects version ${expectedVersion}, but found ${details}.`);
  }
  console.log(`Release versions match ${tag}.`);
} catch (error) {
  console.error(`Failed to validate release versions: ${error.message}`);
  process.exit(1);
}
