import type { CodeHostEndpoint } from "../types/domain";

export const builtinEndpoints: CodeHostEndpoint[] = [
  {
    id: "github.com",
    kind: "gitHub",
    displayName: "GitHub.com",
    webUrl: "https://github.com",
    apiUrl: "https://api.github.com",
    gitHost: "github.com",
    isBuiltin: true,
  },
  {
    id: "gitlab.com",
    kind: "gitLab",
    displayName: "GitLab.com",
    webUrl: "https://gitlab.com",
    apiUrl: "https://gitlab.com/api/v4",
    gitHost: "gitlab.com",
    isBuiltin: true,
  },
  {
    id: "gitee.com",
    kind: "gitee",
    displayName: "Gitee.com",
    webUrl: "https://gitee.com",
    apiUrl: "https://gitee.com/api/v5",
    gitHost: "gitee.com",
    isBuiltin: true,
  },
];
