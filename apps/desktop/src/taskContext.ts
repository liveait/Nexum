export interface TaskContext {
  server: string;
  credentialRevision: number;
}

export interface ContextValue<T> {
  context: TaskContext;
  value: T;
}

/** Context objects are replaced on every Server or credential change. */
export function valueForContext<T>(scoped: ContextValue<T> | null, context: TaskContext): T | null {
  return scoped?.context === context ? scoped.value : null;
}
