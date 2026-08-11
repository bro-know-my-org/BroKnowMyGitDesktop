import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  escapeReleaseNoteText,
  generateCommitNotes,
  parseGitLogRecords,
  releaseLogArgs,
} from "./release-notes.mjs";
import { parseReleaseTag, parseVersionInput, setCargoPackageVersion } from "./release-version-utils.mjs";

test("parses canonical stable versions", () => {
  assert.equal(parseReleaseTag("v1.2.3"), "1.2.3");
  assert.equal(parseVersionInput("1.2.3"), "1.2.3");
  assert.equal(parseVersionInput("v1.2.3"), "1.2.3");
  const max = "65535";
  assert.equal(parseReleaseTag(`v${max}.${max}.${max}`), `${max}.${max}.${max}`);
  assert.equal(parseVersionInput(`${max}.${max}.${max}`), `${max}.${max}.${max}`);
});

test("rejects malformed, padded, and leading-zero versions", () => {
  for (const value of [
    "1.2.3", "v01.2.3", "v1.02.3", "v1.2.03", "v1.2", "v1.2.3-beta",
    " v1.2.3", "v1.2.3 ", "v1.2.3\t", "v1.2.3\n", "v1.2.3\r\n",
    "v65536.0.0", "v0.65536.0", "v0.0.65536",
  ]) {
    assert.equal(parseReleaseTag(value), null);
  }
  for (const value of [
    "01.2.3", "1.02.3", "1.2.03", "1.2", "1.2.3-beta",
    " 1.2.3", "1.2.3 ", "1.2.3\t", "1.2.3\n", "1.2.3\r\n",
    "65536.0.0", "0.65536.0", "0.0.65536",
  ]) {
    assert.equal(parseVersionInput(value), null);
  }
});

test("updates the checked-in Cargo package version without changing other content", async () => {
  const cargoToml = await readFile(new URL("../src-tauri/Cargo.toml", import.meta.url), "utf8");
  const updated = setCargoPackageVersion(cargoToml, "1.2.3");
  assert.equal(updated, cargoToml.replace('version = "0.0.0"', 'version = "1.2.3"'));
});

test("supports CRLF while failing safely when the fixed prefix changes", () => {
  const cargoToml = '[package]\r\nname = "broknowmygitdesktop"\r\nversion = "0.0.0" # release\r\n';
  assert.equal(
    setCargoPackageVersion(cargoToml, "1.2.3"),
    cargoToml.replace('version = "0.0.0"', 'version = "1.2.3"'),
  );
  assert.throws(
    () => setCargoPackageVersion(`\n${cargoToml}`, "1.2.3"),
    /fixed \[package\] version/,
  );
});

test("rejects malformed versions at the mutation boundary", () => {
  const cargoToml = '[package]\nname = "broknowmygitdesktop"\nversion = "0.0.0"\n';
  assert.throws(
    () => setCargoPackageVersion(cargoToml, '1.2.3"\ninvalid = "true'),
    /Invalid normalized stable version/,
  );
});

function runScript(script, releaseTag, repositoryRoot) {
  return spawnSync(process.execPath, [script], {
    cwd: new URL("../", import.meta.url),
    env: {
      ...process.env,
      RELEASE_TAG: releaseTag,
      ...(repositoryRoot ? { RELEASE_REPOSITORY_ROOT: repositoryRoot } : {}),
    },
    encoding: "utf8",
  });
}

function runSetVersion(
  version,
  repositoryRoot,
  actionEnv = { CI: "true", GITHUB_ACTIONS: "true" },
) {
  const env = { ...process.env, RELEASE_REPOSITORY_ROOT: repositoryRoot };
  for (const [name, value] of Object.entries(actionEnv)) {
    if (value === undefined) delete env[name];
    else env[name] = value;
  }
  return spawnSync(process.execPath, ["scripts/set-version.mjs", version], {
    cwd: new URL("../", import.meta.url),
    env,
    encoding: "utf8",
  });
}

async function createReleaseFixture(version) {
  const repositoryRoot = await mkdtemp(path.join(os.tmpdir(), "bkm-release-check-"));
  try {
    const tauriRoot = path.join(repositoryRoot, "src-tauri");
    const memberRoot = path.join(tauriRoot, "workspace-member");
    await mkdir(path.join(tauriRoot, "src"), { recursive: true });
    await mkdir(path.join(memberRoot, "src"), { recursive: true });
    await writeFile(
      path.join(memberRoot, "Cargo.toml"),
      '[package]\nname = "another-package"\nversion = "9.9.9"\nedition = "2021"\n',
    );
    await writeFile(path.join(memberRoot, "src", "lib.rs"), "pub fn another() {}\n");
    await writeFile(path.join(repositoryRoot, "package.json"), JSON.stringify({ version }));
    await writeFile(path.join(tauriRoot, "tauri.conf.json"), JSON.stringify({ version }));
    await writeFile(
      path.join(tauriRoot, "Cargo.toml"),
      `[package]\nname = "broknowmygitdesktop"\nversion = "${version}"\nedition = "2021"\n\n` +
        '[workspace]\nmembers = ["workspace-member"]\nresolver = "2"\n',
    );
    await writeFile(path.join(tauriRoot, "src", "lib.rs"), "pub fn placeholder() {}\n");
    generateFixtureLockfile(repositoryRoot, tauriRoot);
    return repositoryRoot;
  } catch (error) {
    await rm(repositoryRoot, { recursive: true, force: true });
    throw error;
  }
}

function generateFixtureLockfile(repositoryRoot, tauriRoot = path.join(repositoryRoot, "src-tauri")) {
  const generated = spawnSync(
    "cargo",
    ["generate-lockfile", "--manifest-path", path.join(tauriRoot, "Cargo.toml")],
    { cwd: repositoryRoot, encoding: "utf8" },
  );
  assert.equal(generated.status, 0, generated.stderr);
}

async function setFixtureVersion(repositoryRoot, source, version) {
  if (source === "package") {
    await writeFile(path.join(repositoryRoot, "package.json"), JSON.stringify({ version }));
  } else if (source === "tauri") {
    await writeFile(path.join(repositoryRoot, "src-tauri", "tauri.conf.json"), JSON.stringify({ version }));
  } else if (source === "cargo") {
    await writeFile(
      path.join(repositoryRoot, "src-tauri", "Cargo.toml"),
      `[package]\nname = "broknowmygitdesktop"\nversion = "${version}"\nedition = "2021"\n\n` +
        '[workspace]\nmembers = ["workspace-member"]\nresolver = "2"\n',
    );
  }
}

test("release tag CLI handles valid, malformed, and placeholder tags", () => {
  const valid = runScript("scripts/check-release-tag.mjs", "v1.2.3");
  const malformed = runScript("scripts/check-release-tag.mjs", "v01.2.3");
  const placeholder = runScript("scripts/check-release-tag.mjs", "v0.0.0");
  assert.equal(valid.status, 0);
  assert.match(valid.stdout, /is valid/);
  assert.equal(malformed.status, 1);
  assert.match(malformed.stderr, /stable vX\.Y\.Z/);
  assert.equal(placeholder.status, 1);
  assert.match(placeholder.stderr, /stable vX\.Y\.Z/);
});

test("release version CLI handles matching, mismatched, malformed, and placeholder tags", async () => {
  const repositoryRoot = await createReleaseFixture("1.2.3");
  try {
    const matching = runScript("scripts/check-release-version.mjs", "v1.2.3", repositoryRoot);
    assert.equal(matching.status, 0);
    assert.match(matching.stdout, /Versions match|versions match|Release versions match/);

    for (const source of ["package", "tauri", "cargo"]) {
      await setFixtureVersion(repositoryRoot, source, "1.2.4");
      const mismatch = runScript("scripts/check-release-version.mjs", "v1.2.3", repositoryRoot);
      assert.equal(mismatch.status, 1);
      assert.match(mismatch.stderr, /expects version 1\.2\.3/);
      await setFixtureVersion(repositoryRoot, source, "1.2.3");
    }

    const malformed = runScript("scripts/check-release-version.mjs", "v01.2.3", repositoryRoot);
    assert.equal(malformed.status, 1);
    assert.match(malformed.stderr, /non-placeholder stable/);
    const placeholder = runScript("scripts/check-release-version.mjs", "v0.0.0", repositoryRoot);
    assert.equal(placeholder.status, 1);
    assert.match(placeholder.stderr, /non-placeholder stable/);

    await writeFile(path.join(repositoryRoot, "src-tauri", "Cargo.toml"), "not valid TOML = [\n");
    const invalidCargo = runScript("scripts/check-release-version.mjs", "v1.2.3", repositoryRoot);
    assert.equal(invalidCargo.status, 1);
    assert.match(invalidCargo.stderr, /Cargo\.toml/);
  } finally {
    await rm(repositoryRoot, { recursive: true, force: true });
  }
});

test("set-version CLI updates all release metadata only in GitHub Actions", async () => {
  const repositoryRoot = await createReleaseFixture("0.0.0");
  try {
    for (const actionEnv of [
      { CI: "false", GITHUB_ACTIONS: "false" },
      { CI: "false", GITHUB_ACTIONS: "true" },
      { CI: "true", GITHUB_ACTIONS: "false" },
      { CI: undefined, GITHUB_ACTIONS: "true" },
      { CI: "true", GITHUB_ACTIONS: undefined },
    ]) {
      const guarded = runSetVersion("1.2.3", repositoryRoot, actionEnv);
      assert.equal(guarded.status, 1);
      assert.match(guarded.stderr, /only modifies the disposable GitHub Actions/);
    }

    const malformed = runSetVersion("01.2.3", repositoryRoot);
    assert.equal(malformed.status, 1);
    assert.match(malformed.stderr, /stable X\.Y\.Z/);

    const oversized = runSetVersion("65536.0.0", repositoryRoot);
    assert.equal(oversized.status, 1);
    assert.match(oversized.stderr, /stable X\.Y\.Z/);

    const metadataPaths = [
      "package.json",
      path.join("src-tauri", "tauri.conf.json"),
      path.join("src-tauri", "Cargo.toml"),
    ];
    for (const [relativePath, invalidContents] of [
      ["package.json", "{"],
      ["package.json", "{}"],
      [path.join("src-tauri", "tauri.conf.json"), "{"],
      [path.join("src-tauri", "tauri.conf.json"), JSON.stringify({ version: 0 })],
    ]) {
      const target = path.join(repositoryRoot, relativePath);
      const original = await readFile(target, "utf8");
      await writeFile(target, invalidContents);
      const beforeFailure = await Promise.all(
        metadataPaths.map((item) => readFile(path.join(repositoryRoot, item), "utf8")),
      );
      const invalidMetadata = runSetVersion("1.2.3", repositoryRoot);
      assert.equal(invalidMetadata.status, 1);
      const afterFailure = await Promise.all(
        metadataPaths.map((item) => readFile(path.join(repositoryRoot, item), "utf8")),
      );
      assert.deepEqual(afterFailure, beforeFailure);
      await writeFile(target, original);
    }

    for (const source of ["package", "tauri", "cargo"]) {
      await setFixtureVersion(repositoryRoot, source, "1.2.3");
      if (source === "cargo") generateFixtureLockfile(repositoryRoot);
      const nonPlaceholder = runSetVersion("2.0.0", repositoryRoot);
      assert.equal(nonPlaceholder.status, 1);
      assert.match(nonPlaceholder.stderr, /must all be 0\.0\.0/);
      const expectedFile = {
        package: /package\.json/,
        tauri: /tauri\.conf\.json/,
        cargo: /Cargo\.toml/,
      }[source];
      assert.match(nonPlaceholder.stderr, expectedFile);
      await setFixtureVersion(repositoryRoot, source, "0.0.0");
      if (source === "cargo") generateFixtureLockfile(repositoryRoot);
    }

    const cargoLockPath = path.join(repositoryRoot, "src-tauri", "Cargo.lock");
    const cargoLockBeforeUpdate = await readFile(cargoLockPath, "utf8");
    const updated = runSetVersion("v1.2.3", repositoryRoot);
    assert.equal(updated.status, 0);
    assert.match(updated.stdout, /Set project version to 1\.2\.3/);
    assert.equal(JSON.parse(await readFile(path.join(repositoryRoot, "package.json"))).version, "1.2.3");
    assert.equal(
      JSON.parse(await readFile(path.join(repositoryRoot, "src-tauri", "tauri.conf.json"))).version,
      "1.2.3",
    );
    assert.match(
      await readFile(path.join(repositoryRoot, "src-tauri", "Cargo.toml"), "utf8"),
      /version = "1\.2\.3"/,
    );
    const cargoLock = await readFile(cargoLockPath, "utf8");
    assert.equal(cargoLock, cargoLockBeforeUpdate);
    assert.match(cargoLock, /name = "broknowmygitdesktop"\nversion = "0\.0\.0"/);
    assert.match(cargoLock, /name = "another-package"\nversion = "9\.9\.9"/);
    const verified = runScript("scripts/check-release-version.mjs", "v1.2.3", repositoryRoot);
    assert.equal(verified.status, 0);

    const packageBeforePreflightFailure = await readFile(
      path.join(repositoryRoot, "package.json"),
      "utf8",
    );
    const tauriBeforePreflightFailure = await readFile(
      path.join(repositoryRoot, "src-tauri", "tauri.conf.json"),
      "utf8",
    );
    const invalidCargoToml = "not valid TOML = [\n";
    await writeFile(path.join(repositoryRoot, "src-tauri", "Cargo.toml"), invalidCargoToml);
    const cargoFailure = runSetVersion("1.2.4", repositoryRoot);
    assert.equal(cargoFailure.status, 1);
    assert.match(cargoFailure.stderr, /Failed to set project version to 1\.2\.4/);
    assert.match(cargoFailure.stderr, /Cargo\.toml/);
    assert.equal(
      await readFile(path.join(repositoryRoot, "package.json"), "utf8"),
      packageBeforePreflightFailure,
    );
    assert.equal(
      await readFile(path.join(repositoryRoot, "src-tauri", "tauri.conf.json"), "utf8"),
      tauriBeforePreflightFailure,
    );
    assert.equal(await readFile(path.join(repositoryRoot, "src-tauri", "Cargo.toml"), "utf8"), invalidCargoToml);
  } finally {
    await rm(repositoryRoot, { recursive: true, force: true });
  }
});

function sanitizedGitEnv() {
  const env = { ...process.env };
  for (const name of Object.keys(env)) {
    if (
      [
        "GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR", "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM", "GIT_CONFIG_NOSYSTEM", "GIT_CONFIG_COUNT",
      ].includes(name) || /^GIT_CONFIG_(?:KEY|VALUE)_\d+$/.test(name)
    ) {
      delete env[name];
    }
  }
  env.GIT_CONFIG_NOSYSTEM = "1";
  env.GIT_CONFIG_GLOBAL = os.devNull;
  env.GIT_CONFIG_COUNT = "1";
  env.GIT_CONFIG_KEY_0 = "core.hooksPath";
  env.GIT_CONFIG_VALUE_0 = os.devNull;
  return env;
}

function git(repositoryRoot, args) {
  return spawnSync("git", args, {
    cwd: repositoryRoot,
    env: sanitizedGitEnv(),
    encoding: "utf8",
    stdio: "pipe",
  });
}

function runReleaseGit(repositoryRoot, args) {
  return spawnSync(
    process.execPath,
    [fileURLToPath(new URL("./release-git.mjs", import.meta.url)), ...args],
    { cwd: repositoryRoot, env: sanitizedGitEnv(), encoding: "utf8" },
  );
}

function previousTagFromCli(repositoryRoot, commit, currentTag) {
  const result = runReleaseGit(repositoryRoot, ["previous-tag", commit, currentTag]);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim() || null;
}

async function commitFile(repositoryRoot, name, contents, message) {
  await writeFile(path.join(repositoryRoot, name), contents);
  assert.equal(git(repositoryRoot, ["add", name]).status, 0);
  assert.equal(git(repositoryRoot, ["commit", "-m", message]).status, 0);
  return git(repositoryRoot, ["rev-parse", "HEAD"]).stdout.trim();
}

test("release lineage ignores side-branch tags and selects the nearest mainline tag", async () => {
  const repositoryRoot = await mkdtemp(path.join(os.tmpdir(), "bkm-release-git-"));
  try {
    assert.equal(git(repositoryRoot, ["init", "-b", "master"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "user.name", "Release Test"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "user.email", "release@example.invalid"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "commit.gpgSign", "false"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "tag.gpgSign", "false"]).status, 0);
    const first = await commitFile(repositoryRoot, "history.txt", "first\n", "first");
    assert.equal(previousTagFromCli(repositoryRoot, first, "v0.1.0"), null);
    assert.equal(git(repositoryRoot, ["tag", "v1.0.0", first]).status, 0);
    assert.equal(git(repositoryRoot, ["branch", "v1.0.0", first]).status, 0);

    const invalid = await commitFile(repositoryRoot, "history.txt", "first\ninvalid\n", "invalid tags");
    for (const tag of ["v0.0.0", "v01.0.0", "v1.0.0-beta", "v65536.0.0"]) {
      assert.equal(git(repositoryRoot, ["tag", tag, invalid]).status, 0);
    }

    assert.equal(git(repositoryRoot, ["checkout", "-b", "side"]).status, 0);
    const side = await commitFile(repositoryRoot, "side.txt", "side\n", "side");
    assert.equal(git(repositoryRoot, ["tag", "v9.0.0", side]).status, 0);

    assert.equal(git(repositoryRoot, ["checkout", "master"]).status, 0);
    const main = await commitFile(repositoryRoot, "history.txt", "first\nmain\n", "main");
    assert.equal(
      git(repositoryRoot, ["tag", "-a", "v1.1.0", "-m", "release v1.1.0", main]).status,
      0,
    );
    assert.equal(git(repositoryRoot, ["tag", "v1.1.1", main]).status, 0);
    assert.equal(previousTagFromCli(repositoryRoot, main, "v1.1.0"), "v1.0.0");
    assert.equal(git(repositoryRoot, ["merge", "--no-ff", "side", "-m", "merge side"]).status, 0);
    const release = await commitFile(repositoryRoot, "history.txt", "first\nmain\nrelease\n", "release");
    assert.equal(git(repositoryRoot, ["tag", "v1.2.0", release]).status, 0);

    const lineageSuccess = runReleaseGit(repositoryRoot, ["assert-lineage", release, "master"]);
    assert.equal(lineageSuccess.status, 0, lineageSuccess.stderr);
    const symbolicLineage = runReleaseGit(repositoryRoot, ["assert-lineage", "HEAD", "master"]);
    assert.equal(symbolicLineage.status, 0, symbolicLineage.stderr);
    const abbreviatedLineage = runReleaseGit(
      repositoryRoot,
      ["assert-lineage", release.slice(0, 8), "master"],
    );
    assert.equal(abbreviatedLineage.status, 0, abbreviatedLineage.stderr);
    const lineageFailure = runReleaseGit(repositoryRoot, ["assert-lineage", side, "master"]);
    assert.equal(lineageFailure.status, 1);
    assert.match(lineageFailure.stderr, /not on the first-parent release line/);
    assert.equal(previousTagFromCli(repositoryRoot, release, "v1.2.0"), "v1.1.1");
    assert.equal(git(repositoryRoot, ["tag", "v1.2.1", release]).status, 0);
    assert.equal(previousTagFromCli(repositoryRoot, release, "v1.2.1"), "v1.2.0");
    assert.equal(previousTagFromCli(repositoryRoot, release, "v1.2.0"), "v1.1.1");
    assert.equal(generateCommitNotes(repositoryRoot, `v1.2.0..${release}`, "owner/repository"), "");
  } finally {
    await rm(repositoryRoot, { recursive: true, force: true });
  }
});

test("escapes commit subjects before rendering release notes", () => {
  assert.equal(
    escapeReleaseNoteText('fix: [click](https://evil.invalid) <img src="x"> #1 @someone\u202e'),
    'fix: \\[click\\]\\(https://evil\\.invalid\\) &lt;img src="x"&gt; \\#1 \\@someone�',
  );
});

test("release notes sanitize control characters inside escaped commit subjects", async () => {
  const repositoryRoot = await mkdtemp(path.join(os.tmpdir(), "bkm-release-notes-"));
  try {
    assert.equal(git(repositoryRoot, ["init", "-b", "master"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "user.name", "Release Test"]).status, 0);
    assert.equal(git(repositoryRoot, ["config", "user.email", "release@example.invalid"]).status, 0);
    const first = await commitFile(repositoryRoot, "notes.txt", "first\n", "first");
    assert.equal(git(repositoryRoot, ["checkout", "-b", "side"]).status, 0);
    const side = await commitFile(repositoryRoot, "side.txt", "side\n", "side branch detail");
    assert.equal(git(repositoryRoot, ["checkout", "master"]).status, 0);
    const second = await commitFile(
      repositoryRoot,
      "notes.txt",
      "second\n",
      "subject\x1f[link](https://evil.invalid)",
    );
    assert.equal(git(repositoryRoot, ["merge", "--no-ff", "side", "-m", "merge side"]).status, 0);
    const merge = git(repositoryRoot, ["rev-parse", "HEAD"]).stdout.trim();
    const notes = generateCommitNotes(repositoryRoot, `${first}..${merge}`, "owner/repository");
    assert.match(notes, /subject�\\\[link\\\]/);
    assert.match(notes, new RegExp(second));
    assert.doesNotMatch(notes, new RegExp(side));
    assert.doesNotMatch(notes, /side branch detail/);

    const longSubjectCommit = await commitFile(
      repositoryRoot,
      "notes.txt",
      "third\n",
      "x".repeat(700),
    );
    const longSubjectNotes = generateCommitNotes(
      repositoryRoot,
      `${merge}..${longSubjectCommit}`,
      "owner/repository",
    );
    assert.ok(longSubjectNotes.length < 700);
    assert.doesNotMatch(longSubjectNotes, / {2,}\(\[/);
  } finally {
    await rm(repositoryRoot, { recursive: true, force: true });
  }
});

test("release note parser rejects malformed records and hash fields", () => {
  const hash = "a".repeat(40);
  const olderHash = "b".repeat(40);
  const sha256Hash = "c".repeat(64);
  assert.equal(
    parseGitLogRecords(
      `${hash}\0abcdef0\0newer\0${olderHash}\0bcdef01\0older\0`,
      "owner/repository",
    ),
    `- newer ([abcdef0](https://github.com/owner/repository/commit/${hash}))\n` +
      `- older ([bcdef01](https://github.com/owner/repository/commit/${olderHash}))`,
  );
  assert.match(
    parseGitLogRecords(`${sha256Hash}\0cdef012\0sha256\0`, "owner/repository"),
    /sha256/,
  );
  assert.equal(parseGitLogRecords("", "owner/repository"), "");
  const truncated = parseGitLogRecords(
    `${hash}\0abcdef0\0${"newer".repeat(30)}\0${olderHash}\0bcdef01\0older\0`,
    "owner/repository",
    120,
  );
  assert.ok(truncated.length <= 120);
  assert.match(truncated, /已截断/);
  for (const invalidMaximum of [-1, 1.5, Number.NaN]) {
    assert.throws(
      () => parseGitLogRecords("", "owner/repository", invalidMaximum),
      /maximumLength/,
    );
  }
  assert.throws(() => parseGitLogRecords(`${hash}\0abcdef0\0`, "owner/repository"), /record count/);
  for (const invalidHash of ["not-a-hash", "d".repeat(41), "e".repeat(63)]) {
    assert.throws(
      () => parseGitLogRecords(`${invalidHash}\0abcdef0\0subject\0`, "owner/repository"),
      /hash field/,
    );
  }
});

test("release note git query keeps first-parent and output bounds", () => {
  const args = releaseLogArgs("v1.0.0..HEAD");
  assert.deepEqual(args.slice(0, 4), ["log", "-z", "--first-parent", "--max-count=2001"]);
  assert.match(args[4], /%<\(512,trunc\)%s$/);
  assert.deepEqual(args.slice(5), ["--end-of-options", "v1.0.0..HEAD"]);

  const records = Array.from({ length: 2001 }, (_, index) => {
    const hash = index.toString(16).padStart(40, "0");
    return `${hash}\0abcdef0\0commit ${index}\0`;
  }).join("");
  const notes = parseGitLogRecords(records, "owner/repository", 1_000_000);
  assert.match(notes, /已截断/);
  assert.doesNotMatch(notes, /commit 2000/);

  const bufferFallback = generateCommitNotes(".", "HEAD", "owner/repository", () => {
    const error = new Error("stdout maxBuffer length exceeded");
    error.code = "ENOBUFS";
    throw error;
  });
  assert.match(bufferFallback, /已截断/);
});

test("release workflow keeps Cargo locked through test and build", async () => {
  const workflow = await readFile(new URL("../.github/workflows/release.yml", import.meta.url), "utf8");
  const buildJob = workflow.slice(workflow.indexOf("  build:"), workflow.indexOf("  publish:"));
  const rustTest = buildJob.indexOf("cargo test --locked --manifest-path src-tauri/Cargo.toml --lib");
  const build = buildJob.indexOf("pnpm tauri build ${{ matrix.args }} -- --locked");
  const lockGuard = buildJob.indexOf("git diff --exit-code -- src-tauri/Cargo.lock");
  assert.ok(rustTest >= 0);
  assert.ok(build > rustTest);
  assert.ok(lockGuard > build);
  assert.doesNotMatch(buildJob, /git diff --exit-code -- Cargo\.lock/);

  const publishJob = workflow.slice(workflow.indexOf("  publish:"));
  const notesCommand = publishJob.indexOf("node scripts/release-notes.mjs");
  const comparisonLink = publishJob.indexOf("[查看完整版本对比]");
  assert.match(publishJob, /compare\/\$previous_tag\.\.\.\$RELEASE_TAG/);
  assert.match(publishJob, /commits\/\$RELEASE_TAG/);
  assert.ok(notesCommand >= 0);
  assert.ok(comparisonLink > notesCommand);
  assert.match(publishJob, /require_draft[\s\S]*gh release upload/);
});
