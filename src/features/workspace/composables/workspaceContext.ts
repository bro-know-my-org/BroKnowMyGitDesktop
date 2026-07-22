import { inject, type InjectionKey } from "vue";
import type { WorkspaceController } from "./useWorkspaceController";

export const workspaceControllerKey: InjectionKey<WorkspaceController> = Symbol("workspace-controller");

export function useWorkspaceControllerContext(): WorkspaceController {
  const controller = inject(workspaceControllerKey);
  if (!controller) throw new Error("Workspace controller has not been provided.");
  return controller;
}
