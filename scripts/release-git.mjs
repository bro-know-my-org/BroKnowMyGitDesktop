import { execFileSync, spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseReleaseTag } from "./release-version-utils.mjs";

function git(args, repositoryRoot) {
  return execFileSync("git", args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024,
  }).trim();
}

function compareStableVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) return leftParts[index] - rightParts[index];
  }
  return 0;
}

function scanFirstParentHistory(repositoryRoot, start, visit) {
  return new Promise((resolve, reject) => {
    const child = spawn("git", ["rev-list", "--first-parent", "--end-of-options", start], {
      cwd: repositoryRoot,
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    let pending = "";
    let stderr = "";
    let result = null;
    let stoppedAfterMatch = false;
    child.stdout.on("data", (chunk) => {
      const lines = `${pending}${chunk}`.split("\n");
      pending = lines.pop() ?? "";
      for (const line of lines) {
        if (result === null && line) {
          result = visit(line);
          if (result !== null) {
            stoppedAfterMatch = child.kill();
            break;
          }
        }
      }
    });
    child.stderr.on("data", (chunk) => {
      stderr = `${stderr}${chunk}`.slice(-16_384);
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (stoppedAfterMatch) {
        resolve(result);
        return;
      }
      if (code !== 0) {
        reject(new Error(stderr.trim() || `git rev-list exited with status ${code}.`));
        return;
      }
      if (result === null && pending) result = visit(pending);
      resolve(result);
    });
  });
}

export async function isFirstParentCommit(repositoryRoot, commit, branchRef) {
  const resolvedCommit = git(["rev-parse", "--verify", `${commit}^{commit}`], repositoryRoot);
  return Boolean(await scanFirstParentHistory(
    repositoryRoot,
    branchRef,
    (historicalCommit) => historicalCommit === resolvedCommit ? true : null,
  ));
}

export async function findPreviousReleaseTag(repositoryRoot, commit, currentTag) {
  const currentVersion = parseReleaseTag(currentTag);
  if (!currentVersion || currentVersion === "0.0.0") {
    throw new Error(`Invalid current release tag: ${currentTag}`);
  }
  // A lower release tag on the same commit is intentionally eligible: the resulting empty
  // comparison accurately reports that a retagged release contains no commit changes.
  const tagsByCommit = new Map();
  const refs = git(
    [
      "for-each-ref",
      "--sort=-version:refname",
      "--format=%(refname:lstrip=2)%09%(*objectname)%09%(objectname)",
      "refs/tags",
    ],
    repositoryRoot,
  );
  for (const line of refs ? refs.split("\n") : []) {
    const [tag, peeledCommit, object] = line.split("\t");
    const version = parseReleaseTag(tag);
    if (
      !version ||
      version === "0.0.0" ||
      tag === currentTag ||
      compareStableVersions(version, currentVersion) >= 0
    ) continue;
    const targetCommit = peeledCommit || object;
    const tags = tagsByCommit.get(targetCommit) ?? [];
    tags.push(tag);
    tagsByCommit.set(targetCommit, tags);
  }

  return await scanFirstParentHistory(repositoryRoot, commit, (historicalCommit) => {
    const tags = tagsByCommit.get(historicalCommit);
    return tags?.[0] ?? null;
  });
}

const invokedPath = process.argv[1]
  ? fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
  : false;
if (invokedPath) {
  const [command, ...args] = process.argv.slice(2);
  const repositoryRoot = process.cwd();
  if (command === "assert-lineage" && args.length === 2) {
    if (!await isFirstParentCommit(repositoryRoot, args[0], args[1])) {
      console.error(`Commit ${args[0]} is not on the first-parent release line ${args[1]}.`);
      process.exit(1);
    }
  } else if (command === "previous-tag" && args.length === 2) {
    console.log(await findPreviousReleaseTag(repositoryRoot, args[0], args[1]) ?? "");
  } else {
    console.error(
      "Usage: release-git.mjs assert-lineage <commit> <branch-ref> | previous-tag <commit> <current-tag>",
    );
    process.exit(1);
  }
}
