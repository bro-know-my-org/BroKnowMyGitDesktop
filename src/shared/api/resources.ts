import { convertFileSrc } from "@tauri-apps/api/core";
import { request } from "./tauriRuntime";

export interface ResourceComparison {
  path: string;
  previousPath?: string | null;
  staged: boolean;
  revision?: string | null;
}
export interface ResourceSide {
  path: string;
  size: number;
  file: string | null;
  notice: "large" | "lfs" | "special" | "conflict" | null;
  exportable: boolean;
}
export interface ResourcePreview { id: string; sides: [ResourceSide | null, ResourceSide | null] }
export const resourceUrl = (file: string) => convertFileSrc(file);
export const getResourcePreview = (repositoryPath: string, comparison: ResourceComparison) =>
  request<ResourcePreview>("get_resource_preview", { repositoryPath, comparison }, () => {
    throw { code: "resource_unavailable", message: "" };
  });
export const releaseResourcePreview = (id: string) =>
  request<void>("release_resource_preview", { id }, () => undefined);
export const exportResourcePreview = (id: string, side: number) =>
  request<boolean>("export_resource_preview", { id, side }, () => false);
