export interface DestroyableDialog {
  destroy(): void;
}

export type GitLockInventoryDisposition = "showLocks" | "inconclusive" | "emptyReference" | "empty";

export function gitLockInventoryDisposition(
  lockCount: number,
  incomplete: boolean,
  referenceLockError: boolean,
): GitLockInventoryDisposition {
  if (lockCount > 0) return "showLocks";
  if (incomplete) return "inconclusive";
  return referenceLockError ? "emptyReference" : "empty";
}

export function beginGitLockInspection(busyOwners: Set<symbol>, owner: symbol): boolean {
  if (busyOwners.has(owner)) return false;
  busyOwners.add(owner);
  return true;
}

export function finishGitLockInspection(busyOwners: Set<symbol>, owner: symbol): void {
  busyOwners.delete(owner);
}

export async function runOwnedGitLockInspection<T>(
  busyOwners: Set<symbol>,
  owner: symbol,
  isCurrent: () => boolean,
  inspect: () => Promise<T>,
): Promise<T | null> {
  if (!isCurrent() || !beginGitLockInspection(busyOwners, owner)) return null;
  try {
    const result = await inspect();
    return isCurrent() ? result : null;
  } catch (error) {
    if (isCurrent()) throw error;
    return null;
  } finally {
    finishGitLockInspection(busyOwners, owner);
  }
}

export function gitLockInspectionIsCurrent(
  repositoryIsCurrent: boolean,
  activeOwner: symbol | null,
  requestOwner: symbol,
): boolean {
  return repositoryIsCurrent && activeOwner === requestOwner;
}

export function replaceGitLockRecoveryDialog<T extends DestroyableDialog>(
  current: T | null,
  next: T | null,
): T | null {
  current?.destroy();
  return next;
}
