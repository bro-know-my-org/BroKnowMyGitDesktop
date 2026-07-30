import accountsEnUS from "./en-US/accounts.json";
import commonEnUS from "./en-US/common.json";
import demoEnUS from "./en-US/demo.json";
import errorsEnUS from "./en-US/errors.json";
import repositoryEnUS from "./en-US/repository.json";
import workspaceEnUS from "./en-US/workspace.json";
import accountsZhCN from "./zh-CN/accounts.json";
import commonZhCN from "./zh-CN/common.json";
import demoZhCN from "./zh-CN/demo.json";
import errorsZhCN from "./zh-CN/errors.json";
import repositoryZhCN from "./zh-CN/repository.json";
import workspaceZhCN from "./zh-CN/workspace.json";

export const zhCNMessages = {
  ...commonZhCN,
  ...workspaceZhCN,
  ...repositoryZhCN,
  ...accountsZhCN,
  ...errorsZhCN,
  ...demoZhCN,
} as const;

export type MessageKey = keyof typeof zhCNMessages;

export const enUSMessages = {
  ...commonEnUS,
  ...workspaceEnUS,
  ...repositoryEnUS,
  ...accountsEnUS,
  ...errorsEnUS,
  ...demoEnUS,
} as const satisfies Record<MessageKey, string>;
