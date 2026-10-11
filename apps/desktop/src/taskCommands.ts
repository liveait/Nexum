type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export type TaskCommand = "task_queue" | "task_pause" | "task_resume" | "task_remove";

interface CreatedDownload {
  id: string;
  destination?: string;
}

export type AddDownloadResult =
  | { id: string; destination?: string; queued: true }
  | { id: string; destination?: string; queued: false; error: unknown };

export function invokeTaskCommand(invoke: Invoke, command: TaskCommand, taskId: string): Promise<boolean> {
  return invoke<boolean>(command, { taskId });
}

/** The backend chooses the connection, task ID and collision-safe destination. */
export async function addDownload(invoke: Invoke, source: string, filename: string, directory: string): Promise<AddDownloadResult> {
  const created = await invoke<CreatedDownload>("task_create", { source, filename, directory });
  if (!created || typeof created.id !== "string" || !created.id) throw new Error("The download could not be created. Try again.");
  const { id, destination } = created;
  try {
    await invokeTaskCommand(invoke, "task_queue", id);
    return { id, destination, queued: true };
  } catch (error) {
    return { id, destination, queued: false, error };
  }
}
