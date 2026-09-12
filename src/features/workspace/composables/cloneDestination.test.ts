import assert from "node:assert/strict";
import test from "node:test";
import { cloneDestination } from "./cloneDestination";

test("clone paths preserve POSIX, Windows drive and UNC parents", () => {
  for (const [parent, remote, expected] of [
    ["/home/me/projects/", "https://github.com/org/repo.git", "/home/me/projects/repo"],
    ["/", "git@github.com:org/repo.git", "/repo"],
    ["/tmp/team\\archive", "https://host/org/repo.git", "/tmp/team\\archive/repo"],
    ["/tmp/team\\", "https://host/org/repo.git", "/tmp/team\\/repo"],
    ["C:\\", "ssh://git@host/org/repo.git", "C:\\repo"],
    ["C:/Projects/", "https://host/org/repo/", "C:/Projects/repo"],
    ["\\\\server\\share\\", "git@host:org/repo.git", "\\\\server\\share\\repo"],
    ["/Users/me/工作 项目", "https://host/org/工具.git", "/Users/me/工作 项目/工具"],
  ]) assert.equal(cloneDestination(parent, remote), expected);
});

test("suggested clone folders cannot escape the parent or use reserved Windows names", () => {
  assert.equal(cloneDestination("/projects", "https://host/org/.."), "/projects/repository");
  assert.equal(cloneDestination("C:\\projects", "https://host/org/CON.git"), "C:\\projects\\repository-CON");
  assert.equal(cloneDestination("/projects", ""), "");
  assert.equal(cloneDestination("", "git@host:org/repo.git"), "");
});
