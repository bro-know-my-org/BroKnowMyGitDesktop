import { execFileSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { readFile, realpath, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseVersionInput, setCargoPackageVersion } from "./release-version-utils.mjs";

if (process.env.CI !== "true" || process.env.GITHUB_ACTIONS !== "true") {
  console.error("This command only modifies the disposable GitHub Actions build workspace.");
  process.exit(1);
}

const input = process.argv[2];
const version = parseVersionInput(input);

if (!version) {
  console.error("Version must use the stable X.Y.Z format (for example, 0.2.0).");
  process.exit(1);
}

const repositoryRoot = process.env.RELEASE_REPOSITORY_ROOT
  ? path.resolve(process.env.RELEASE_REPOSITORY_ROOT)
  : fileURLToPath(new URL("../", import.meta.url));
const packageJsonUrl = path.join(repositoryRoot, "package.json");
const tauriConfigUrl = path.join(repositoryRoot, "src-tauri", "tauri.conf.json");
const cargoTomlUrl = path.join(repositoryRoot, "src-tauri", "Cargo.toml");
const cargoManifest = cargoTomlUrl;

try {
  const packageJson = JSON.parse(await readFile(packageJsonUrl, "utf8"));
  const tauriConfig = JSON.parse(await readFile(tauriConfigUrl, "utf8"));
  const cargoToml = await readFile(cargoTomlUrl, "utf8");
  const metadata = JSON.parse(execFileSync(
    "cargo",
    ["metadata", "--manifest-path", cargoManifest, "--format-version", "1", "--no-deps", "--locked"],
    {
      cwd: repositoryRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 64 * 1024 * 1024,
    },
  ));
  const cargoManifestPath = await realpath(cargoManifest);
  const cargoPackage = metadata.packages.find(
    (item) => realpathSync(item.manifest_path) === cargoManifestPath,
  );
  if (!cargoPackage) {
    throw new Error(`Cargo metadata does not contain the package at ${cargoManifestPath}.`);
  }
  const checkedInVersions = [
    ["package.json", packageJson.version],
    ["src-tauri/tauri.conf.json", tauriConfig.version],
    ["src-tauri/Cargo.toml", cargoPackage.version],
  ];
  const nonPlaceholderVersions = checkedInVersions.filter(
    ([, currentVersion]) => currentVersion !== "0.0.0",
  );
  if (nonPlaceholderVersions.length > 0) {
    const details = nonPlaceholderVersions
      .map(([file, currentVersion]) => `${file}: ${String(currentVersion)}`)
      .join(", ");
    throw new Error(`Checked-in release versions must all be 0.0.0; found ${details}.`);
  }
  const updatedCargoToml = setCargoPackageVersion(cargoToml, version);
  packageJson.version = version;
  tauriConfig.version = version;
  await writeFile(packageJsonUrl, `${JSON.stringify(packageJson, null, 2)}\n`);
  await writeFile(tauriConfigUrl, `${JSON.stringify(tauriConfig, null, 2)}\n`);
  await writeFile(cargoTomlUrl, updatedCargoToml);
  execFileSync(
    "cargo",
    [
      "metadata",
      "--manifest-path",
      cargoManifest,
      "--format-version",
      "1",
      "--no-deps",
      "--locked",
    ],
    { cwd: repositoryRoot, stdio: ["ignore", "ignore", "inherit"] },
  );
} catch (error) {
  console.error(`Failed to set project version to ${version}: ${error.message}`);
  process.exit(1);
}

console.log(`Set project version to ${version}.`);
