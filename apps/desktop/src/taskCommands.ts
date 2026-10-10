type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export type TaskCommand = "task_queue" | "task_pause" | "task_resume" | "task_remove";

export type AddDownloadResult =
  | { id: string; queued: true }
  | { id: string; queued: false; error: unknown };

export function invokeTaskCommand(invoke: Invoke, command: TaskCommand, server: string, taskId: string): Promise<boolean> {
  return invoke<boolean>(command, { server, taskId });
}

function generateTaskId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `download-${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export async function addDownload(invoke: Invoke, server: string, source: string, destination: string): Promise<AddDownloadResult> {
  const id = generateTaskId();
  await invoke("task_create", { server, id, source, destination });
  try {
    await invokeTaskCommand(invoke, "task_queue", server, id);
    return { id, queued: true };
  } catch (error) {
    return { id, queued: false, error };
  }
}
