import assert from "node:assert/strict";
import test from "node:test";
import type { AccountProfile, AuthenticationMethod, RepositoryRecord } from "../../../shared/types/domain";
import { filterRepositoryGroups, repositoryRemoteLabel, repositoryStarAriaLabel, supportsCloneCredentials, supportsGitRemoteCredentials, type RepositorySidebarGroup } from "./workspaceControllerSupport";

test("GitHub CLI accounts remain selectable for published remote bindings and pinned clones", () => {
  const profile = (authMethod: AuthenticationMethod): AccountProfile => ({
    id: "github-work", endpointId: "github.com", login: "work-login", displayName: null,
    authMethod, secretRef: null, status: "valid", isDefaultCandidate: false,
  });
  for (const method of ["ghCli", "personalAccessToken"] as const) {
    assert.equal(supportsGitRemoteCredentials(profile(method)), true);
    assert.equal(supportsCloneCredentials(profile(method)), true);
  }
  assert.equal(supportsGitRemoteCredentials(profile("sshAgent")), false);
  assert.equal(supportsCloneCredentials(profile("sshAgent")), true);
  for (const method of ["glabCli", "systemGitCredentialHelper"] as const) {
    assert.equal(supportsGitRemoteCredentials(profile(method)), false);
    assert.equal(supportsCloneCredentials(profile(method)), false);
  }
});

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

test("star controls identify repositories even when display names are identical", () => {
  const first = repository("/work/project", "Client");
  const second = repository("/personal/project", "Client");
  assert.equal(repositoryStarAriaLabel(first, "Star"), "Star: Client (/work/project)");
  assert.notEqual(repositoryStarAriaLabel(first, "Star"), repositoryStarAriaLabel(second, "Star"));
});

test("remote names and nested namespaces are searchable independently of local names", () => {
  const local = { ...repository("/projects/sdk-cache"), namespace: "upstream-team", remoteNamespace: "team/platform", remoteRepository: "payment-sdk" };
  const remoteGroups = [{ key: "work", label: "Work", repositories: [local] }];
  assert.equal(repositoryRemoteLabel(local), "team/platform/payment-sdk");
  assert.deepEqual(filterRepositoryGroups(remoteGroups, " PAYMENT-SDK ")[0].repositories, [local]);
  assert.equal(filterRepositoryGroups(remoteGroups, "team/platform").length, 1);
  assert.equal(repositoryRemoteLabel(repository("/local")), "");
  assert.equal(repositoryRemoteLabel({ ...local, remoteNamespace: null }), "payment-sdk");
});

test("repository filtering removes empty groups and returns no groups when nothing matches", () => {
  assert.deepEqual(filterRepositoryGroups(groups, "notes").map((group) => group.key), ["personal"]);
  assert.deepEqual(filterRepositoryGroups(groups, "missing repository"), []);
});
