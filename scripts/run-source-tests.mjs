import { readdir } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { pathToFileURL } from "node:url";

export async function sourceTests(directory, readDirectory = readdir) {
  const entries = await readDirectory(directory, { withFileTypes: true });
  const tests = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) tests.push(...await sourceTests(entryPath, readDirectory));
    else if (entry.isFile() && /\.test\.(?:mjs|ts)$/u.test(entry.name)) tests.push(entryPath);
  }
  return tests.sort();
}

export async function discoverSourceTests() {
  return [...await sourceTests("src"), "scripts/run-source-tests.test.mjs"].sort();
}

export async function runSourceTests({
  discover = discoverSourceTests,
  spawn = spawnSync,
  stderr = console.error,
  execPath = process.execPath,
} = {}) {
  let tests;
  try {
    tests = await discover();
  } catch (error) {
    stderr("Failed to discover source tests:", error);
    return 1;
  }
  if (!tests.length) {
    stderr("No source tests found under src or scripts.");
    return 1;
  }
  const result = spawn(execPath, ["--import", "tsx", "--test", ...tests], {
    stdio: "inherit",
  });
  if (result.error) {
    stderr("Failed to start source tests:", result.error);
    return 1;
  }
  if (result.signal) {
    stderr(`Source tests terminated by signal ${result.signal}.`);
    return 1;
  }
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exit(await runSourceTests());
}
