/** Suggest a filename from the URL path. Response headers may name it differently. */
export function suggestedFileName(source: string): string {
  if (!isHttpSource(source)) return "";
  try {
    const url = new URL(source.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") return "";
    if (url.pathname.endsWith("/")) return "";
    const segments = url.pathname.split("/").filter(Boolean);
    const encoded = segments[segments.length - 1];
    if (!encoded) return "";
    const name = decodeURIComponent(encoded);
    return isValidFileName(name) ? name : "";
  } catch {
    return "";
  }
}

export function isHttpSource(source: string): boolean {
  if (hasHardLineBreak(source)) return false;
  const value = source.trim();
  // The Server resolver requires an explicit authority separator. URL alone
  // accepts shorthand such as "https:example.com", which task.create rejects.
  if (!/^https?:\/\//i.test(value)) return false;
  const authority = value.slice(value.indexOf("://") + 3).split(/[/?#]/, 1)[0];
  if (!authority || authority.startsWith(":")) return false;
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && Boolean(url.hostname);
  } catch {
    return false;
  }
}

export function isValidFileName(name: string): boolean {
  return Boolean(name.trim())
    && name !== "."
    && name !== ".."
    && !/[\\/\u0000-\u001f\u007f]/.test(name);
}

/** Soft wrapping is visual only; one task still accepts exactly one URL. */
export function hasHardLineBreak(source: string): boolean {
  return /[\r\n]/.test(source);
}
