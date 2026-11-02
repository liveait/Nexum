import { invoke } from "@tauri-apps/api/core";

export interface UpgradeInfo { old_version: string; new_version: string }
export interface UpgradeNoticeState { info: UpgradeInfo | null; busy: boolean; failed: boolean }
export const EMPTY_UPGRADE_NOTICE: UpgradeNoticeState = { info: null, busy: false, failed: false };

// Shared by the real banner and IPC tests, including stale reads and failed writes.
export class UpgradeNoticeController {
  private state: UpgradeNoticeState = EMPTY_UPGRADE_NOTICE;
  private generation = 0;
  private active = true;
  private readonly changed: (state: UpgradeNoticeState) => void;

  constructor(changed: (state: UpgradeNoticeState) => void) { this.changed = changed; }

  private update(state: UpgradeNoticeState) {
    this.state = state;
    this.changed(state);
  }

  async load(): Promise<void> {
    const generation = ++this.generation;
    try {
      const info = await invoke<UpgradeInfo | null>("get_upgrade_info");
      if (this.active && generation === this.generation) this.update({ ...this.state, info });
    } catch { /* A read failure never dismisses an existing message. */ }
  }

  async dismiss(): Promise<void> {
    if (!this.active || !this.state.info || this.state.busy) return;
    ++this.generation;
    this.update({ ...this.state, busy: true, failed: false });
    try {
      await invoke("acknowledge_upgrade");
      ++this.generation;
      if (this.active) this.update(EMPTY_UPGRADE_NOTICE);
    } catch {
      if (this.active) this.update({ ...this.state, busy: false, failed: true });
    }
  }

  dispose(): void {
    this.active = false;
    ++this.generation;
  }
}
