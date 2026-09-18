function writeClipboardTextWithSelection(text: string): boolean {
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  try {
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    textarea.remove();
  }
}

export async function writeClipboardText(text: string): Promise<void> {
  // Keep this synchronous attempt in the context-menu event call stack so
  // WebViews that require transient user activation can grant clipboard access.
  if (writeClipboardTextWithSelection(text)) return;
  if (!navigator.clipboard?.writeText) throw new Error("clipboard unavailable");
  await navigator.clipboard.writeText(text);
}
