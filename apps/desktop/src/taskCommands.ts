type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export type TaskCommand = "task_queue" | "task_pause" | "task_resume" | "task_remove";

export interface CreatedDownload {
  id: string;
  destination?: string;
}

interface AddDownloadOptions {
  created?: CreatedDownload | null;
  isCurrent?: () => boolean;
  onCreated?: (download: CreatedDownload) => void;
}

export type AddDownloadResult =
  | { id: string; destination?: string; queued: true }
  | { id: string; destination?: string; queued: false; error: unknown };

export function invokeTaskCommand(invoke: Invoke, command: TaskCommand, taskId: string): Promise<boolean> {
  return invoke<boolean>(command, { taskId });
}

/** The backend chooses the connection, task ID and collision-safe destination. */
export async function addDownload(invoke: Invoke, source: string, filename: string, directory: string, options: AddDownloadOptions = {}): Promise<AddDownloadResult> {
  const ensureCurrent = () => {
    if (options.isCurrent && !options.isCurrent()) throw new Error("The download connection changed. Reopen Add Download to continue.");
  };
  ensureCurrent();
  const created = options.created ?? await invoke<CreatedDownload>("task_create", { source, filename, directory });
  if (!created || typeof created.id !== "string" || !created.id) throw new Error("The download could not be created. Try again.");
  ensureCurrent();
  options.onCreated?.(created);
  const { id, destination } = created;
  try {
    ensureCurrent();
    if (!await invokeTaskCommand(invoke, "task_queue", id)) throw new Error("Server did not queue the task. Try again.");
    return { id, destination, queued: true };
  } catch (error) {
    return { id, destination, queued: false, error };
  }
}

/** Saves partial metadata so an interrupted task can be resumed. */
export async function savePartialMetadata(
  invoke: Invoke,
  taskId: string,
  source: string,
  downloadedBytes: number,
  totalBytes: number | null,
): Promise<string> {
  return invoke<string>("save_partial_metadata", { taskId, source, downloaded_bytes: downloadedBytes, total_bytes: totalBytes });
}

/** Reads partial metadata for a task (for interruption recovery). */
export interface PartialMetadata {
  task_id: string;
  source: string;
  downloaded_bytes: number;
  total_bytes: number | null;
}

export async function readPartialMetadata(invoke: Invoke, taskId: string): Promise<PartialMetadata | null> {
  return invoke<PartialMetadata | null>("read_partial_metadata", { taskId });
}

/** Removes partial metadata for a completed task. */
export async function clearPartialMetadata(invoke: Invoke, taskId: string): Promise<void> {
  return invoke<void>("clear_partial_metadata", { taskId });
}
