export type ConnectionMode = "managed" | "external";

export interface ManagedConnectionStatus {
  mode: ConnectionMode;
  state: "starting" | "running" | "restarting" | "failed" | "stopped" | "external";
  generation: number;
  error: string | null;
}

/** A mode transition or unavailable child must not route task requests. */
export function isConnectionReady(mode: ConnectionMode, status: ManagedConnectionStatus | null): boolean {
  return status?.mode === mode && (mode === "managed" ? status.state === "running" : status.state === "external");
}
