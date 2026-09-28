import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import "./App.css";

type TaskState =
  | "Created"
  | "Queued"
  | "Downloading"
  | "Paused"
  | "Completed"
  | "Failed"
  | "Retrying";

interface TaskItem {
  id: string;
  source: string;
  destination: string;
  state: TaskState;
  downloaded_bytes: number;
  total_bytes: number | null;
  error: string | null;
}

interface DesktopSettings {
  server: string;
  refresh_interval_secs: number;
}

type Section = "dashboard" | "downloads" | "trackers" | "plugins" | "notifications" | "settings";
type TaskFilter = "all" | "active" | "queued" | "completed" | "failed";
type SettingsCategory = "home" | "general" | "appearance" | "downloads" | "bittorrent" | "integration" | "network" | "advanced" | "about";
type ConnectionState = "disconnected" | "connecting" | "connected" | "error";
type TaskCommand = "task_queue" | "task_pause" | "task_resume" | "task_remove";
type NoticeTone = "success" | "error";

interface Notice {
  id: number;
  message: string;
  tone: NoticeTone;
  createdAt: number;
}

interface ServerEvent {
  server: string;
  sequence: number;
  event: string;
  data: Record<string, unknown>;
}

interface EventStreamStatus {
  server: string;
  connected: boolean;
  error: string | null;
}

const DEFAULT_SERVER = "127.0.0.1:39100";

const DOWNLOAD_FILTERS: Array<{ id: TaskFilter; label: string }> = [
  { id: "all", label: "All Downloads" },
  { id: "active", label: "Active" },
  { id: "queued", label: "Queued" },
  { id: "completed", label: "Completed" },
  { id: "failed", label: "Failed" },
];

const NAV_ITEMS: Array<{ id: Section; label: string; icon: IconName }> = [
  { id: "dashboard", label: "Dashboard", icon: "dashboard" },
  { id: "downloads", label: "Downloads", icon: "downloads" },
  { id: "trackers", label: "Trackers", icon: "trackers" },
  { id: "plugins", label: "Plugins", icon: "plugins" },
];

const SETTINGS_CARDS: Array<{
  id: Exclude<SettingsCategory, "home">;
  label: string;
  description: string;
  icon: IconName;
  available: boolean;
}> = [
  { id: "general", label: "General", description: "Startup, Server and system integration", icon: "general", available: true },
  { id: "appearance", label: "Appearance", description: "Theme, language and sidebar", icon: "appearance", available: true },
  { id: "downloads", label: "Downloads", description: "Refresh, destination and task behavior", icon: "downloads", available: true },
  { id: "bittorrent", label: "BitTorrent", description: "DHT, trackers and peer settings", icon: "bittorrent", available: false },
  { id: "integration", label: "Integration", description: "Browser extension and CLI", icon: "integration", available: false },
  { id: "network", label: "Network", description: "Proxy, TLS and connection limits", icon: "network", available: false },
  { id: "advanced", label: "Advanced", description: "RPC, storage and diagnostics", icon: "advanced", available: true },
  { id: "about", label: "About", description: "Version, updates and documentation", icon: "about", available: true },
];

function isActiveTask(task: TaskItem): boolean {
  return task.state === "Downloading" || task.state === "Paused";
}

function matchesFilter(task: TaskItem, filter: TaskFilter): boolean {
  switch (filter) {
    case "active": return isActiveTask(task);
    case "queued": return task.state === "Queued" || task.state === "Retrying";
    case "completed": return task.state === "Completed";
    case "failed": return task.state === "Failed";
    default: return true;
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatBytesMaybe(bytes: number | null): string {
  return bytes === null ? "Unknown size" : formatBytes(bytes);
}

function formatUpdatedAt(value: number | null): string {
  if (value === null) return "Not updated yet";
  const seconds = Math.max(0, Math.round((Date.now() - value) / 1000));
  if (seconds < 5) return "Updated just now";
  if (seconds < 60) return `Updated ${seconds}s ago`;
  return `Updated ${Math.round(seconds / 60)}m ago`;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "The operation failed";
}

type IconName = "dashboard" | "downloads" | "trackers" | "plugins" | "notifications" | "settings" | "general" | "appearance" | "bittorrent" | "integration" | "network" | "advanced" | "about" | "search" | "info" | "list" | "more";

function Icon({ name, size = 20 }: { name: IconName; size?: number }) {
  const common = { width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round" as const, strokeLinejoin: "round" as const, "aria-hidden": true };
  switch (name) {
    case "dashboard": return <svg {...common}><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></svg>;
    case "downloads": return <svg {...common}><path d="M12 3v12" /><path d="m7 10 5 5 5-5" /><path d="M4 19h16" /></svg>;
    case "trackers": return <svg {...common}><circle cx="12" cy="12" r="2" /><path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M4.9 4.9a10 10 0 0 0 0 14.2M19.1 4.9a10 10 0 0 1 0 14.2" /></svg>;
    case "plugins": return <svg {...common}><path d="M5 8h14v12H5z" /><path d="M8 8V5h8v3M9 12h6M9 16h4" /></svg>;
    case "notifications": return <svg {...common}><path d="M18 9a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4" /></svg>;
    case "settings": return <svg {...common}><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-1.8 1.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-2.6V20a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1-1.8-1.8.1-.1A1.7 1.7 0 0 0 8 15a1.7 1.7 0 0 0-1.6-1H6v-2.6h.2a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9l-.1-.1 1.8-1.8.1.1a1.7 1.7 0 0 0 1.9.3 1.7 1.7 0 0 0 1-1.6V5h2.6v.2a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1 1.8 1.8-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2V14h-.2a1.7 1.7 0 0 0-1.6 1Z" /></svg>;
    case "general": return <svg {...common}><circle cx="12" cy="12" r="7" /><path d="M12 8v4l3 2" /></svg>;
    case "appearance": return <svg {...common}><path d="M12 3a9 9 0 1 0 9 9c0-1.1-.9-2-2-2h-2a2 2 0 0 1-2-2V6a3 3 0 0 0-3-3Z" /><circle cx="7.5" cy="12" r=".7" /><circle cx="9" cy="7.5" r=".7" /><circle cx="14" cy="6.5" r=".7" /></svg>;
    case "bittorrent": return <svg {...common}><circle cx="12" cy="12" r="8" /><path d="M12 4v16M4 12h16M6.3 6.3l11.4 11.4M17.7 6.3 6.3 17.7" /></svg>;
    case "integration": return <svg {...common}><path d="M8 8h8v8H8z" /><path d="M12 8V4M12 20v-4M8 12H4M20 12h-4" /></svg>;
    case "network": return <svg {...common}><circle cx="12" cy="12" r="2" /><circle cx="5" cy="6" r="2" /><circle cx="19" cy="6" r="2" /><circle cx="5" cy="18" r="2" /><circle cx="19" cy="18" r="2" /><path d="m10.5 10.5-4-3M13.5 10.5l4-3M10.5 13.5l-4 3M13.5 13.5l4 3" /></svg>;
    case "advanced": return <svg {...common}><path d="M4 7h16M4 12h16M4 17h16" /><circle cx="9" cy="7" r="2" fill="currentColor" stroke="none" /><circle cx="15" cy="12" r="2" fill="currentColor" stroke="none" /><circle cx="11" cy="17" r="2" fill="currentColor" stroke="none" /></svg>;
    case "about": return <svg {...common}><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></svg>;
    case "search": return <svg {...common}><circle cx="10.8" cy="10.8" r="6.5" /><path d="m16 16 4.5 4.5" /></svg>;
    case "info": return <svg {...common}><circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" /></svg>;
    case "list": return <svg {...common}><rect x="4" y="4" width="16" height="16" rx="2" /><path d="M8 8h8M8 12h8M8 16h5" /></svg>;
    case "more": return <svg {...common}><circle cx="5" cy="12" r="1" fill="currentColor" stroke="none" /><circle cx="12" cy="12" r="1" fill="currentColor" stroke="none" /><circle cx="19" cy="12" r="1" fill="currentColor" stroke="none" /></svg>;
  }
}

export default function App() {
  const [section, setSection] = useState<Section>("downloads");
  const [filter, setFilter] = useState<TaskFilter>("all");
  const [settingsCategory, setSettingsCategory] = useState<SettingsCategory>("home");
  const [server, setServer] = useState(DEFAULT_SERVER);
  const [serverDraft, setServerDraft] = useState(DEFAULT_SERVER);
  const [refreshSeconds, setRefreshSeconds] = useState(5);
  const [settingsReady, setSettingsReady] = useState(false);
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("disconnected");
  const [eventStreamConnected, setEventStreamConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [actionByTask, setActionByTask] = useState<Record<string, TaskCommand | "start">>({});
  const [lastUpdatedAt, setLastUpdatedAt] = useState<number | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [notifications, setNotifications] = useState<Notice[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [showInspector, setShowInspector] = useState(true);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [newId, setNewId] = useState("");
  const [newSource, setNewSource] = useState("");
  const [newDest, setNewDest] = useState("");
  const [addLoading, setAddLoading] = useState(false);
  const [destinationPicking, setDestinationPicking] = useState(false);
  const [addError, setAddError] = useState("");
  const eventRefreshTimer = useRef<number | null>(null);

  const pushNotification = (message: string, tone: NoticeTone) => {
    const item = { id: Date.now(), message, tone, createdAt: Date.now() };
    setNotifications((current) => [item, ...current].slice(0, 30));
    if (tone === "success") {
      setNotice(message);
      setError("");
    } else {
      setError(message);
      setNotice("");
    }
  };

  const refreshTasks = async (address = server, options: { silent?: boolean } = {}): Promise<void> => {
    const silent = options.silent ?? false;
    if (!address.trim()) {
      setConnection("error");
      pushNotification("Enter a Server address first.", "error");
      return;
    }
    if (!silent) {
      setLoading(true);
      setConnection("connecting");
    }
    setError("");
    try {
      const result = await invoke<TaskItem[]>("task_list", { server: address });
      setTasks(result);
      setSelectedId((current) => current && result.some((task) => task.id === current) ? current : result[0]?.id ?? null);
      setLastUpdatedAt(Date.now());
      setConnection("connected");
    } catch (caught) {
      setConnection("error");
      setError(errorMessage(caught));
    } finally {
      if (!silent) setLoading(false);
    }
  };

  useEffect(() => {
    let mounted = true;
    void invoke<DesktopSettings>("load_settings")
      .then((settings) => {
        if (!mounted) return;
        const address = settings.server.trim() || DEFAULT_SERVER;
        setServer(address);
        setServerDraft(address);
        setRefreshSeconds(settings.refresh_interval_secs);
      })
      .catch((caught) => { if (mounted) setError(errorMessage(caught)); })
      .finally(() => { if (mounted) setSettingsReady(true); });
    return () => { mounted = false; };
  }, []);

  useEffect(() => {
    if (!settingsReady || !server.trim()) return undefined;
    let active = true;
    let eventUnlisten: UnlistenFn | undefined;
    let statusUnlisten: UnlistenFn | undefined;
    let streamGeneration: number | undefined;

    const scheduleRefresh = () => {
      if (!active || eventRefreshTimer.current !== null) return;
      eventRefreshTimer.current = window.setTimeout(() => {
        eventRefreshTimer.current = null;
        if (active) void refreshTasks(server, { silent: true });
      }, 120);
    };

    const handleServerEvent = (payload: ServerEvent) => {
      if (payload.server !== server) return;
      if (!payload.event.startsWith("task.") && !payload.event.startsWith("scheduler.")) return;
      scheduleRefresh();
      const taskId = typeof payload.data.task_id === "string" ? payload.data.task_id : "Task";
      if (payload.event === "scheduler.completed") pushNotification(`${taskId} completed`, "success");
      if (payload.event === "scheduler.failed") pushNotification(`${taskId} failed`, "error");
      if (payload.event === "scheduler.retrying") pushNotification(`${taskId} will retry`, "error");
    };

    const handleEventStreamStatus = (payload: EventStreamStatus) => {
      if (!active || payload.server !== server) return;
      setEventStreamConnected(payload.connected);
      if (payload.connected) scheduleRefresh();
    };

    void Promise.all([
      listen<ServerEvent>("server-event", ({ payload }) => { if (active) handleServerEvent(payload); }),
      listen<EventStreamStatus>("server-event-status", ({ payload }) => handleEventStreamStatus(payload)),
    ])
      .then(([removeEvent, removeStatus]) => {
        if (!active) {
          removeEvent();
          removeStatus();
          return;
        }
        eventUnlisten = removeEvent;
        statusUnlisten = removeStatus;
        return invoke<number>("start_event_stream", { server }).then((generation) => {
          if (!active) {
            return invoke("stop_event_stream", { generation });
          }
          streamGeneration = generation;
          return undefined;
        });
      })
      .catch((caught) => { if (active) { setEventStreamConnected(false); setError(errorMessage(caught)); } });

    return () => {
      active = false;
      if (eventRefreshTimer.current !== null) {
        window.clearTimeout(eventRefreshTimer.current);
        eventRefreshTimer.current = null;
      }
      eventUnlisten?.();
      statusUnlisten?.();
      setEventStreamConnected(false);
      if (streamGeneration !== undefined) {
        void invoke("stop_event_stream", { generation: streamGeneration }).catch(() => undefined);
      }
    };
    // Event subscriptions follow the active server and settings only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server, settingsReady]);

  useEffect(() => {
    if (settingsReady) void refreshTasks();
    // refreshTasks intentionally follows the active server only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server, settingsReady]);

  const activeDownload = tasks.some((task) => task.state === "Downloading");
  const pollingSeconds = refreshSeconds > 0 && activeDownload ? 1 : refreshSeconds;

  useEffect(() => {
    if (!settingsReady || eventStreamConnected || pollingSeconds <= 0) return undefined;
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refreshTasks();
    }, pollingSeconds * 1000);
    return () => window.clearInterval(timer);
    // refreshTasks is intentionally kept behind the active server/settings inputs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connection, eventStreamConnected, pollingSeconds, server, settingsReady]);

  const counts = useMemo(() => DOWNLOAD_FILTERS.reduce<Record<TaskFilter, number>>((result, item) => {
    result[item.id] = tasks.filter((task) => matchesFilter(task, item.id)).length;
    return result;
  }, { all: tasks.length, active: 0, queued: 0, completed: 0, failed: 0 }), [tasks]);

  const visibleTasks = useMemo(() => tasks.filter((task) => {
    if (!matchesFilter(task, filter)) return false;
    const query = searchQuery.trim().toLowerCase();
    return !query || [task.id, task.source, task.destination, task.state].some((value) => value.toLowerCase().includes(query));
  }), [tasks, filter, searchQuery]);
  const selectedTask = tasks.find((task) => task.id === selectedId) ?? null;

  const selectSection = (next: Section) => {
    setSection(next);
    if (next === "settings") setSettingsCategory("home");
    if (next === "downloads") setFilter("all");
  };

  const applyServer = async () => {
    const nextServer = serverDraft.trim();
    if (!nextServer) {
      pushNotification("Enter a Server address first.", "error");
      return;
    }
    try {
      await invoke("save_settings", { settings: { server: nextServer, refresh_interval_secs: refreshSeconds } });
      setServer(nextServer);
      pushNotification("Settings saved", "success");
    } catch (caught) {
      pushNotification(errorMessage(caught), "error");
    }
  };

  const runTaskCommand = async (id: string, command: TaskCommand, successMessage: string): Promise<void> => {
    setActionByTask((current) => ({ ...current, [id]: command }));
    setNotice("");
    setError("");
    try {
      await invoke<boolean>(command, { server, task_id: id });
      pushNotification(successMessage, "success");
      await refreshTasks();
    } catch (caught) {
      pushNotification(errorMessage(caught), "error");
    } finally {
      setActionByTask((current) => { const next = { ...current }; delete next[id]; return next; });
    }
  };

  const startNextTask = async (taskId?: string): Promise<void> => {
    const actionKey = taskId ?? selectedId ?? "__next__";
    setActionByTask((current) => ({ ...current, [actionKey]: "start" }));
    try {
      await invoke<string>("task_start", { server });
      pushNotification("Download started", "success");
      await refreshTasks();
    } catch (caught) {
      pushNotification(errorMessage(caught), "error");
    } finally {
      setActionByTask((current) => { const next = { ...current }; delete next[actionKey]; return next; });
    }
  };

  const chooseDestination = async (): Promise<void> => {
    setDestinationPicking(true);
    setAddError("");
    try {
      const destination = await save({
        title: "Choose download destination",
        defaultPath: newDest.trim() || undefined,
      });
      if (destination) setNewDest(destination);
    } catch (caught) {
      setAddError(errorMessage(caught));
    } finally {
      setDestinationPicking(false);
    }
  };

  const createAndQueue = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    setAddLoading(true);
    setAddError("");
    try {
      const id = newId.trim();
      await invoke("task_create", { server, id, source: newSource.trim(), destination: newDest.trim() });
      try {
        await invoke<boolean>("task_queue", { server, task_id: id });
      } catch (caught) {
        setAddError(`Task created, but queueing failed: ${errorMessage(caught)}`);
        await refreshTasks();
        return;
      }
      setNewId(""); setNewSource(""); setNewDest(""); setShowAdd(false);
      selectSection("downloads");
      pushNotification("Download added and queued", "success");
      await refreshTasks();
    } catch (caught) {
      setAddError(errorMessage(caught));
    } finally {
      setAddLoading(false);
    }
  };

  return (
    <main className="app-shell">
      <aside className="sidebar" aria-label="Nexum navigation">
        <div className="sidebar-topbar">
          <span className="sidebar-brand">NEXUM</span>
          <div className="sidebar-top-actions">
            <button className="sidebar-icon-button" onClick={() => setShowInspector((current) => !current)} title="Toggle inspector"><Icon name="list" size={18} /></button>
            <button className="sidebar-icon-button add-sidebar-button" onClick={() => { setShowAdd(true); setAddError(""); }} title="Add download">＋</button>
          </div>
        </div>
        <nav className="primary-nav">
          {NAV_ITEMS.map((item) => (
            <button className={`nav-item ${section === item.id ? "selected" : ""}`} key={item.id} onClick={() => selectSection(item.id)}>
              <Icon name={item.icon} size={20} /><span>{item.label}</span>
              {item.id === "downloads" && counts.active > 0 && <span className="nav-count">{counts.active}</span>}
            </button>
          ))}
        </nav>
        <div className="sidebar-spacer" />
        <button className={`nav-item ${section === "notifications" ? "selected" : ""}`} onClick={() => selectSection("notifications")}>
          <Icon name="notifications" size={20} /><span>Notifications</span>{notifications.length > 0 && <span className="nav-count">{notifications.length}</span>}
        </button>
        <div className="sidebar-divider" />
        <button className={`nav-item ${section === "settings" ? "selected" : ""}`} onClick={() => selectSection("settings")}><Icon name="settings" size={20} /><span>Settings</span></button>
      </aside>

      <section className="workspace">
        {section === "downloads" && (
          <DownloadsView
            tasks={tasks}
            visibleTasks={visibleTasks}
            counts={counts}
            filter={filter}
            selectedTask={selectedTask}
            actionByTask={actionByTask}
            connection={connection}
            eventStreamConnected={eventStreamConnected}
            server={server}
            loading={loading}
            error={error}
            notice={notice}
            lastUpdatedAt={lastUpdatedAt}
            showInspector={showInspector}
            searchOpen={searchOpen}
            searchQuery={searchQuery}
            onFilterChange={setFilter}
            onSelect={setSelectedId}
            onSearchOpen={() => setSearchOpen((current) => !current)}
            onSearchQuery={setSearchQuery}
            onToggleInspector={() => setShowInspector((current) => !current)}
            onRefresh={() => void refreshTasks()}
            onAdd={() => { setShowAdd(true); setAddError(""); }}
            onPause={(id) => void runTaskCommand(id, "task_pause", "Download paused")}
            onResume={(id) => void runTaskCommand(id, "task_resume", "Download resumed")}
            onRemove={(id) => void runTaskCommand(id, "task_remove", "Task removed")}
            onQueue={(id) => void runTaskCommand(id, "task_queue", "Task queued")}
            onStart={(id) => void startNextTask(id)}
          />
        )}
        {section === "dashboard" && <DashboardView tasks={tasks} connection={connection} onAdd={() => setShowAdd(true)} onDownloads={() => selectSection("downloads")} />}
        {section === "trackers" && <UnavailableView icon="trackers" title="Trackers" description="Tracker discovery and health checks will appear here when the BitTorrent engine is connected." />}
        {section === "plugins" && <UnavailableView icon="plugins" title="Plugins" description="The plugin manager is currently a runtime foundation. Executable providers and a plugin catalog are planned." />}
        {section === "notifications" && <NotificationsView notifications={notifications} onDownloads={() => selectSection("downloads")} />}
        {section === "settings" && <SettingsView category={settingsCategory} server={serverDraft} connection={connection} refreshSeconds={refreshSeconds} onCategory={setSettingsCategory} onServerChange={setServerDraft} onRefreshSecondsChange={setRefreshSeconds} onApply={() => void applyServer()} />}
      </section>

      {showAdd && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => !addLoading && !destinationPicking && setShowAdd(false)}>
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="add-download-title" onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-header"><div><span className="eyebrow">New task</span><h2 id="add-download-title">Add Download</h2></div><button className="close-button" onClick={() => setShowAdd(false)} disabled={addLoading || destinationPicking} aria-label="Close">×</button></div>
            <form onSubmit={createAndQueue}>
              <label>Source URL<input value={newSource} onChange={(event) => setNewSource(event.target.value)} placeholder="https://example.com/file.zip" autoFocus required /></label>
              <label>Destination<div className="input-with-action"><input value={newDest} onChange={(event) => setNewDest(event.target.value)} placeholder="/Users/you/Downloads/file.zip" required /><button type="button" className="button chooser-button" onClick={() => void chooseDestination()} disabled={addLoading || destinationPicking}>{destinationPicking ? "Choosing…" : "Choose…"}</button></div></label>
              <label>Task ID<input value={newId} onChange={(event) => setNewId(event.target.value)} placeholder="file-download" required /></label>
              {addError && <p className="form-error" role="alert">{addError}</p>}
              <div className="modal-actions"><button type="button" className="button" onClick={() => setShowAdd(false)} disabled={addLoading || destinationPicking}>Cancel</button><button type="submit" className="button primary" disabled={addLoading || destinationPicking || !newId.trim() || !newSource.trim() || !newDest.trim()}>{addLoading ? "Adding…" : "Add and Queue"}</button></div>
            </form>
          </section>
        </div>
      )}
    </main>
  );
}

function DownloadsView({
  tasks, visibleTasks, counts, filter, selectedTask, actionByTask, connection, eventStreamConnected, server, loading, error, notice, lastUpdatedAt, showInspector, searchOpen, searchQuery,
  onFilterChange, onSelect, onSearchOpen, onSearchQuery, onToggleInspector, onRefresh, onAdd, onPause, onResume, onRemove, onQueue, onStart,
}: {
  tasks: TaskItem[];
  visibleTasks: TaskItem[];
  counts: Record<TaskFilter, number>;
  filter: TaskFilter;
  selectedTask: TaskItem | null;
  actionByTask: Record<string, TaskCommand | "start">;
  connection: ConnectionState;
  eventStreamConnected: boolean;
  server: string;
  loading: boolean;
  error: string;
  notice: string;
  lastUpdatedAt: number | null;
  showInspector: boolean;
  searchOpen: boolean;
  searchQuery: string;
  onFilterChange: (filter: TaskFilter) => void;
  onSelect: (id: string) => void;
  onSearchOpen: () => void;
  onSearchQuery: (query: string) => void;
  onToggleInspector: () => void;
  onRefresh: () => void;
  onAdd: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onRemove: (id: string) => void;
  onQueue: (id: string) => void;
  onStart: (id: string) => void;
}) {
  const title = DOWNLOAD_FILTERS.find((item) => item.id === filter)?.label ?? "All Downloads";
  return (
    <>
      <header className="content-header">
        <div className="content-title"><select value={filter} onChange={(event) => onFilterChange(event.target.value as TaskFilter)} aria-label="Download filter">{DOWNLOAD_FILTERS.map((item) => <option value={item.id} key={item.id}>{item.label}</option>)}</select><span className="count-pill">{counts[filter]}/{tasks.length}</span></div>
        <div className="header-tools"><button className="round-tool" title="More actions"><Icon name="more" /></button><button className={`round-tool ${!showInspector ? "muted" : ""}`} onClick={onToggleInspector} title="Toggle inspector"><Icon name="list" /></button><button className={`round-tool ${showInspector ? "active" : ""}`} onClick={onToggleInspector} title="Task details"><Icon name="info" /></button><button className="round-tool" onClick={onSearchOpen} title="Search"><Icon name="search" /></button></div>
      </header>
      {searchOpen && <div className="search-row"><Icon name="search" size={16} /><input value={searchQuery} onChange={(event) => onSearchQuery(event.target.value)} placeholder="Search downloads" autoFocus /></div>}
      {(error || notice) && <div className={`message-banner ${error ? "error" : "success"}`} role={error ? "alert" : "status"}><span>{error || notice}</span>{error && <button onClick={onRefresh}>Retry</button>}</div>}
      <div className="download-content">
        {visibleTasks.length === 0 ? <EmptyDownloads onAdd={onAdd} hasFilter={filter !== "all" || Boolean(searchQuery)} /> : <div className="download-layout"><section className="task-list" aria-label="Downloads">{visibleTasks.map((task) => <TaskRow key={task.id} task={task} selected={selectedTask?.id === task.id} action={actionByTask[task.id]} onSelect={() => onSelect(task.id)} onPause={() => onPause(task.id)} onResume={() => onResume(task.id)} onRemove={() => onRemove(task.id)} onQueue={() => onQueue(task.id)} onStart={() => onStart(task.id)} />)}</section>{showInspector && <TaskInspector task={selectedTask} server={server} />}</div>}
      </div>
      <footer className="status-bar"><span className="status-mode">HTTP/HTTPS</span><span className="status-transfer"><span>↓ —</span><span>↑ —</span></span><span className="status-spacer" /><span className="status-item"><span className={`status-light ${connection}`} />{connection === "connected" ? "Server connected" : "Server unavailable"}</span><span className="status-item"><span className={`status-light ${eventStreamConnected ? "ready" : "connecting"}`} />{eventStreamConnected ? "Live updates" : "Polling fallback"}</span><span className="status-item"><span className="status-light ready" />{loading ? "Syncing" : formatUpdatedAt(lastUpdatedAt)}</span></footer>
      <button className="floating-add" onClick={onAdd} title="Add download">＋</button>
      <span className="sr-only">{title}</span>
    </>
  );
}

function TaskRow({ task, selected, action, onSelect, onPause, onResume, onRemove, onQueue, onStart }: { task: TaskItem; selected: boolean; action?: TaskCommand | "start"; onSelect: () => void; onPause: () => void; onResume: () => void; onRemove: () => void; onQueue: () => void; onStart: () => void }) {
  const progress = task.total_bytes && task.total_bytes > 0 ? Math.min(100, (task.downloaded_bytes / task.total_bytes) * 100) : null;
  return <article className={`task-row ${selected ? "selected" : ""}`} onClick={onSelect}>
    <div className="task-row-main"><div className="task-row-heading"><strong>{task.id}</strong><span className={`state-badge state-${task.state.toLowerCase()}`}>{task.state}</span></div><div className="task-source" title={task.source}>{task.source}</div><div className="progress-track"><span className={progress === null ? "indeterminate" : ""} style={progress === null ? undefined : { width: `${progress}%` }} /></div><div className="task-row-meta"><span>{formatBytes(task.downloaded_bytes)} / {formatBytesMaybe(task.total_bytes)}</span><span className="task-destination" title={task.destination}>{task.destination}</span></div>{task.error && <div className="task-error">{task.error}</div>}</div>
    <div className="task-row-actions" onClick={(event) => event.stopPropagation()}>{task.state === "Downloading" && <button className="row-action" disabled={Boolean(action)} onClick={onPause}>{action === "task_pause" ? "Pausing…" : "Pause"}</button>}{task.state === "Paused" && <button className="row-action" disabled={Boolean(action)} onClick={onResume}>{action === "task_resume" ? "Resuming…" : "Resume"}</button>}{task.state === "Queued" && <button className="row-action" disabled={Boolean(action)} onClick={onStart}>{action === "start" ? "Starting…" : "Start"}</button>}{task.state === "Created" && <button className="row-action" disabled={Boolean(action)} onClick={onQueue}>{action === "task_queue" ? "Queueing…" : "Queue"}</button>}<button className="row-action danger-text" disabled={Boolean(action)} onClick={onRemove}>{action === "task_remove" ? "Removing…" : "Remove"}</button></div>
  </article>;
}

function TaskInspector({ task, server }: { task: TaskItem | null; server: string }) {
  if (!task) return <aside className="inspector inspector-empty"><span className="inspector-icon"><Icon name="info" size={24} /></span><p>Select a download to inspect it.</p></aside>;
  const progress = task.total_bytes === null ? null : Math.min(100, (task.downloaded_bytes / Math.max(task.total_bytes, 1)) * 100);
  return <aside className="inspector" aria-label="Task details"><span className="eyebrow">Task details</span><h2>{task.id}</h2><span className={`state-badge state-${task.state.toLowerCase()}`}>{task.state}</span><div className="inspector-progress"><div className="progress-track"><span className={progress === null ? "indeterminate" : ""} style={progress === null ? undefined : { width: `${progress}%` }} /></div><strong>{formatBytes(task.downloaded_bytes)} / {formatBytesMaybe(task.total_bytes)}</strong></div><dl className="detail-list"><div><dt>Source</dt><dd title={task.source}>{task.source}</dd></div><div><dt>Destination</dt><dd title={task.destination}>{task.destination}</dd></div><div><dt>Server</dt><dd>{server}</dd></div></dl>{task.error && <div className="inspector-error"><strong>Latest error</strong><p>{task.error}</p><button className="text-button" onClick={() => void navigator.clipboard?.writeText(task.error ?? "")}>Copy error</button></div>}</aside>;
}

function EmptyDownloads({ onAdd, hasFilter }: { onAdd: () => void; hasFilter: boolean }) {
  return <div className="empty-state"><div className="empty-art" aria-hidden="true"><div className="art-block art-one" /><div className="art-block art-two" /><div className="art-block art-three" /><div className="art-block art-four" /></div><h2>{hasFilter ? "No matching downloads" : "No downloads yet"}</h2><p>{hasFilter ? "Try another filter or search term." : "Press + to add one"}</p>{hasFilter && <button className="button" onClick={onAdd}>Add Download</button>}</div>;
}

function DashboardView({ tasks, connection, onAdd, onDownloads }: { tasks: TaskItem[]; connection: ConnectionState; onAdd: () => void; onDownloads: () => void }) {
  const active = tasks.filter((task) => task.state === "Downloading").length;
  const queued = tasks.filter((task) => task.state === "Queued" || task.state === "Retrying").length;
  const completed = tasks.filter((task) => task.state === "Completed").length;
  return <div className="dashboard-page"><div className="page-heading"><div><span className="eyebrow">Overview</span><h1>Dashboard</h1><p>Current activity from your Nexum Server.</p></div><button className="button primary" onClick={onAdd}>＋ Add Download</button></div><div className="metric-grid"><MetricCard label="All downloads" value={tasks.length} icon="downloads" /><MetricCard label="Active" value={active} icon="trackers" /><MetricCard label="Queued" value={queued} icon="list" /><MetricCard label="Completed" value={completed} icon="general" /></div><section className="dashboard-card"><div className="dashboard-card-heading"><div><span className="eyebrow">Recent activity</span><h2>{tasks.length ? "Latest downloads" : "Nothing here yet"}</h2></div><button className="text-button" onClick={onDownloads}>View downloads</button></div>{tasks.length ? tasks.slice(0, 5).map((task) => <div className="activity-row" key={task.id}><span className={`status-light ${task.state === "Completed" ? "ready" : "connected"}`} /><strong>{task.id}</strong><span>{task.state}</span></div>) : <p className="dashboard-empty">Add an HTTP or HTTPS URL to see it here.</p>}</section><div className="dashboard-connection"><span className={`status-light ${connection}`} />{connection === "connected" ? "Server connected" : "Start nexum-server to connect"}</div></div>;
}

function MetricCard({ label, value, icon }: { label: string; value: number; icon: IconName }) {
  return <div className="metric-card"><div className="metric-icon"><Icon name={icon} size={20} /></div><span>{label}</span><strong>{value}</strong></div>;
}

function UnavailableView({ icon, title, description }: { icon: IconName; title: string; description: string }) {
  return <div className="unavailable-page"><div className="unavailable-icon"><Icon name={icon} size={34} /></div><span className="eyebrow">Planned capability</span><h1>{title}</h1><p>{description}</p><span className="planned-badge">Not connected yet</span></div>;
}

function NotificationsView({ notifications, onDownloads }: { notifications: Notice[]; onDownloads: () => void }) {
  return <div className="notifications-page"><div className="page-heading"><div><span className="eyebrow">Activity center</span><h1>Notifications</h1><p>Task and connection events from this Desktop session.</p></div><button className="button" onClick={onDownloads}>View downloads</button></div>{notifications.length === 0 ? <div className="notifications-empty"><Icon name="notifications" size={32} /><h2>No notifications</h2><p>Completion and failure events will appear here.</p></div> : <div className="notification-list">{notifications.map((item) => <div className={`notification-row ${item.tone}`} key={item.id}><span className={`status-light ${item.tone === "success" ? "ready" : "error"}`} /><div><strong>{item.message}</strong><span>{new Date(item.createdAt).toLocaleTimeString()}</span></div></div>)}</div>}</div>;
}

function SettingsView({ category, server, connection, refreshSeconds, onCategory, onServerChange, onRefreshSecondsChange, onApply }: { category: SettingsCategory; server: string; connection: ConnectionState; refreshSeconds: number; onCategory: (category: SettingsCategory) => void; onServerChange: (value: string) => void; onRefreshSecondsChange: (value: number) => void; onApply: () => void }) {
  if (category !== "home") {
    const card = SETTINGS_CARDS.find((item) => item.id === category);
    if (category === "general") return <SettingsDetail title={card?.label ?? "General"} icon="general" onBack={() => onCategory("home")}><div className="settings-card"><div className="settings-card-heading"><div><span className="eyebrow">Connection</span><h2>Server</h2></div><span className={`status-pill ${connection}`}>{connection}</span></div><label>Server address<input value={server} onChange={(event) => onServerChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") onApply(); }} placeholder={DEFAULT_SERVER} /></label><p className="field-help">The Server runs as a separate process. The saved address is kept in the Mac application support directory.</p><button className="button primary" onClick={onApply}>Save and test connection</button></div></SettingsDetail>;
    if (category === "downloads") return <SettingsDetail title={card?.label ?? "Downloads"} icon="downloads" onBack={() => onCategory("home")}><div className="settings-card"><span className="eyebrow">Updates</span><h2>Refresh policy</h2><p className="field-help">Idle tasks refresh every five seconds. Active downloads refresh every second.</p><select value={refreshSeconds} onChange={(event) => onRefreshSecondsChange(Number(event.target.value))}><option value={0}>Manual</option><option value={5}>Automatic</option></select><button className="button primary settings-save" onClick={onApply}>Save settings</button></div></SettingsDetail>;
    if (category === "appearance") return <SettingsDetail title={card?.label ?? "Appearance"} icon="appearance" onBack={() => onCategory("home")}><div className="settings-card"><span className="eyebrow">Theme</span><h2>Follow system appearance</h2><p className="field-help">Nexum follows the Mac light or dark appearance. Language choices will be added to the persisted settings model.</p></div></SettingsDetail>;
    return <SettingsDetail title={card?.label ?? "Settings"} icon={card?.icon ?? "settings"} onBack={() => onCategory("home")}><div className="settings-card settings-planned"><span className="eyebrow">Planned capability</span><h2>{card?.label}</h2><p>{card?.description}. This page is reserved so the navigation can remain stable while the underlying engine and protocol are implemented.</p><span className="planned-badge">Coming later</span></div></SettingsDetail>;
  }
  return <div className="settings-page"><div className="page-heading"><div><span className="eyebrow">Client preferences</span><h1>Settings</h1><p>Configure Nexum without mixing connection options into the download list.</p></div></div><div className="settings-grid">{SETTINGS_CARDS.map((card) => <button className={`settings-card-tile ${card.available ? "" : "planned"}`} key={card.id} onClick={() => onCategory(card.id)}><span className={`settings-tile-icon icon-${card.id}`}><Icon name={card.icon} size={28} /></span><strong>{card.label}</strong><p>{card.description}</p>{!card.available && <span className="planned-badge">Planned</span>}</button>)}</div></div>;
}

function SettingsDetail({ title, icon, onBack, children }: { title: string; icon: IconName; onBack: () => void; children: ReactNode }) {
  return <div className="settings-page"><button className="back-button" onClick={onBack}>‹ Settings</button><div className="settings-detail-heading"><span className="settings-tile-icon"><Icon name={icon} size={27} /></span><div><span className="eyebrow">Settings</span><h1>{title}</h1></div></div>{children}</div>;
}
