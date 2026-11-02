import { invoke } from "@tauri-apps/api/core";

// One startup read per WebView; opening Add ends the welcome for this WebView.
export class FirstRunNoticeController {
  private requested = false;
  private dismissed: boolean;
  private active = true;
  private readonly changed: (visible: boolean) => void;

  constructor(changed: (visible: boolean) => void, initiallyDismissed = false) {
    this.changed = changed;
    this.dismissed = initiallyDismissed;
  }

  async load(): Promise<void> {
    if (this.requested || !this.active || this.dismissed) return;
    this.requested = true;
    try {
      const fresh = await invoke<boolean>("check_fresh_install");
      if (this.active && !this.dismissed) this.changed(fresh);
    } catch { /* An unavailable welcome read must not block downloads. */ }
  }

  dismiss(): void {
    this.dismissed = true;
    if (this.active) this.changed(false);
  }

  dispose(): void { this.active = false; }
}
