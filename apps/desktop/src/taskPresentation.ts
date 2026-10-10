interface TaskLocation {
  source: string;
  destination: string;
}

function lastPathSegment(path: string): string | null {
  const normalized = path.replace(/\\/g, "/").trim();
  if (!normalized || normalized.endsWith("/")) return null;
  const segment = normalized.slice(normalized.lastIndexOf("/") + 1);
  return segment && segment !== "." && segment !== ".." ? segment : null;
}

export function taskDisplayName(task: TaskLocation): string | null {
  const destinationName = lastPathSegment(task.destination);
  if (destinationName) return destinationName;

  try {
    const source = new URL(task.source);
    const encodedName = lastPathSegment(source.pathname);
    if (encodedName) {
      try {
        const decodedName = decodeURIComponent(encodedName);
        if (decodedName && !/[\\/\u0000-\u001f]/.test(decodedName)) return decodedName;
      } catch {
        // Keep the encoded segment when the URL contains malformed escapes.
      }
      return encodedName;
    }
    return source.hostname || null;
  } catch {
    return lastPathSegment(task.source);
  }
}
