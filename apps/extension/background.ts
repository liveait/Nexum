// Nexum browser extension background service worker

type ServerConfig = {
  server?: string;
};

type JsonRpcError = {
  code?: number;
  message?: string;
};

type JsonRpcResponse<T> = {
  result?: T;
  error?: JsonRpcError;
};

type RuntimeMessage = {
  type?: unknown;
  url?: unknown;
};

let nextRequestId = 1;

type NexumTask = {
  id: string;
  source: string;
  destination: string;
  state: string;
  downloaded_bytes: number;
  total_bytes: number | null;
};

/** Add the context-menu item once the extension is installed or updated. */
function registerContextMenu(): void {
  // Context-menu entries survive service-worker restarts. Remove this
  // extension's old entry first so updates and reloads stay idempotent.
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "sendToNexum",
      title: "Send to Nexum",
      contexts: ["link"],
    });
  });
}

chrome.runtime.onInstalled.addListener(registerContextMenu);

/** Listen for context menu clicks */
chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === "sendToNexum" && info.linkUrl) {
    sendToNexum(info.linkUrl, tab?.id);
  }
});

/** Listen for send requests from the content script's link badge. */
chrome.runtime.onMessage.addListener((message: unknown, sender) => {
  if (!isSendToNexumMessage(message)) {
    return;
  }

  void sendToNexum(message.url, sender.tab?.id);
});

function isSendToNexumMessage(message: unknown): message is { type: "sendToNexum"; url: string } {
  if (typeof message !== "object" || message === null) {
    return false;
  }
  const candidate = message as RuntimeMessage;
  return candidate.type === "sendToNexum" && typeof candidate.url === "string";
}

/** Get the configured server address */
async function getServerAddress(): Promise<string> {
  const result = await chrome.storage.local.get("server");
  const config = result as ServerConfig;
  return config.server?.trim() || "127.0.0.1:39100";
}

/** Send a URL to the Nexum server */
async function sendToNexum(url: string, tabId?: number): Promise<void> {
  try {
    const server = await getServerAddress();
    const task = await createTask(server, url);
    // Show notification with task ID
    chrome.notifications?.create({
      type: "basic",
      iconUrl: "icons/nexum-48.png",
      title: "Nexum",
      message: `Task created: ${task.id}`,
      priority: 1,
    });
    // Notify content script if tabId provided
    if (tabId !== undefined) {
      chrome.tabs?.sendMessage(tabId, { type: "taskCreated", task });
    }
  } catch (error) {
    chrome.notifications?.create({
      type: "basic",
      iconUrl: "icons/nexum-48.png",
      title: "Nexum",
      message: `Failed: ${error instanceof Error ? error.message : String(error)}`,
      priority: 1,
    });
  }
}

/** Create a task on the Nexum server via JSON-RPC */
async function createTask(server: string, source: string): Promise<NexumTask> {
  const id = `browser-${Date.now()}`;
  const destination = `/tmp/nexum-${id}`;

  // The server keeps creation and queueing as separate operations. Create the
  // persisted task first, then queue that same ID so the dispatcher can start it.
  const task = await callRpc<NexumTask>(server, "task.create", {
    id,
    source,
    destination,
  });
  const queued = await callRpc<boolean>(server, "task.queue", { id: task.id });
  if (!queued) {
    throw new Error("Server did not queue the task");
  }
  return task;
}

/** Call the server's HTTP JSON-RPC bridge. */
async function callRpc<T>(server: string, method: string, params: unknown): Promise<T> {
  const response = await fetch(jsonRpcUrl(server), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: nextRequestId++,
      method,
      params,
    }),
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${response.statusText}`.trim());
  }

  let data: JsonRpcResponse<T>;
  try {
    data = (await response.json()) as JsonRpcResponse<T>;
  } catch {
    throw new Error("Server returned invalid JSON");
  }
  if (data.error) {
    throw new Error(data.error.message || `RPC error ${data.error.code ?? "unknown"}`);
  }
  if (!("result" in data)) {
    throw new Error("Server response did not include a result");
  }
  return data.result as T;
}

function jsonRpcUrl(server: string): string {
  const base = /^https?:\/\//i.test(server) ? server : `http://${server}`;
  return `${base.replace(/\/+$/, "")}/jsonrpc`;
}
