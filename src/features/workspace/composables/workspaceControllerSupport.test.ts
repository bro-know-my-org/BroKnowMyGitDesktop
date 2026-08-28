import assert from "node:assert/strict";
import test from "node:test";
import type { RepositoryRecord } from "../../../shared/types/domain";
import { filterRepositoryGroups, type RepositorySidebarGroup } from "./workspaceControllerSupport";

function repository(path: string, alias: string | null = null): RepositoryRecord {
  return {
    path,
    name: path.split("/").filter(Boolean).pop() ?? "repository",
    alias,
    group: null,
    host: null,
    namespace: null,
    lastOpenedAt: null,
  };
}

const groups: RepositorySidebarGroup[] = [
  {
    key: "work",
    label: "Work",
    repositories: [
      repository("/projects/BroKnowMyGitDesktop", "Desktop client"),
      repository("/projects/service-api"),
    ],
  },
  {
    key: "personal",
    label: "Personal",
    repositories: [repository("/playground/notes")],
  },
];

test("repository filtering matches alias, name, and path without locale-sensitive casing", () => {
  assert.deepEqual(
    filterRepositoryGroups(groups, "DESKTOP").flatMap((group) => group.repositories.map((item) => item.path)),
    ["/projects/BroKnowMyGitDesktop"],
  );
  assert.deepEqual(
    filterRepositoryGroups(groups, "SERVICE-API").flatMap((group) => group.repositories.map((item) => item.path)),
    ["/projects/service-api"],
  );
  assert.deepEqual(
    filterRepositoryGroups(groups, "PLAYGROUND").flatMap((group) => group.repositories.map((item) => item.path)),
    ["/playground/notes"],
  );
});

test("repository filtering treats whitespace-only input as no filter", () => {
  assert.equal(filterRepositoryGroups(groups, "   "), groups);
});

test("repository filtering removes empty groups and returns no groups when nothing matches", () => {
  assert.deepEqual(filterRepositoryGroups(groups, "notes").map((group) => group.key), ["personal"]);
  assert.deepEqual(filterRepositoryGroups(groups, "missing repository"), []);
});
