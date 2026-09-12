export function cloneDestination(parent: string, remote: string): string {
  if (!parent || !remote.trim()) return "";
  const tail = remote.trim().replace(/[?#].*$/u, "").replace(/\/+$/u, "").split(/[/:\\]/u).pop() ?? "";
  let name = tail.replace(/\.git$/iu, "").replace(/[<>:"|?*\u0000-\u001f]/gu, "-").replace(/[. ]+$/u, "");
  if (!name || /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(name)) name = `repository${name ? `-${name}` : ""}`;
  const windowsPath = /^(?:[A-Za-z]:[\\/]|\\\\)/u.test(parent);
  const separator = windowsPath && parent.includes("\\") ? "\\" : "/";
  return `${parent.replace(windowsPath ? /[\\/]+$/u : /\/+$/u, "")}${separator}${name}`;
}
