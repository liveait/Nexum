import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { downloadDir, join } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { hasHardLineBreak, isHttpSource, isValidFileName, suggestedFileName } from "./downloadDraft";
import { resolveLocale, translate, TranslationProvider, useTranslation, type LanguagePreference, type Locale, type MessageKey, type Translate } from "./i18n";
import { addDownload, invokeTaskCommand, type TaskCommand } from "./taskCommands";
import { valueForContext, type ContextValue, type TaskContext } from "./taskContext";
import { taskDisplayName } from "./taskPresentation";
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

const EMPTY_TASKS: TaskItem[] = [];
const EMPTY_TASK_ACTIONS: Record<string, TaskCommand | "start"> = {};

interface DesktopSettings {
  server: string;
  refresh_interval_secs: number;
  language: LanguagePreference;
}

type Section = "dashboard" | "downloads" | "trackers" | "plugins" | "notifications" | "settings";
type TaskFilter = "all" | "active" | "queued" | "completed" | "failed";
type SettingsCategory = "home" | "general" | "appearance" | "downloads" | "bittorrent" | "integration" | "network" | "advanced" | "about";
type ConnectionState = "disconnected" | "connecting" | "connected" | "error";
type NoticeTone = "success" | "error";
type CredentialScheme = "Bearer" | "ApiKey";

interface Notice {
  id: number;
  message: LocalizedMessage;
  tone: NoticeTone;
  createdAt: number;
  read: boolean;
  server: string;
  taskId?: string;
}

const NOTIFICATION_LIMIT = 30;
const SUCCESS_BANNER_DURATION_MS = 5_000;
const SCHEDULER_MESSAGES: Record<string, { named: MessageKey; generic: MessageKey; tone: NoticeTone }> = {
  "scheduler.completed": { named: "{name} completed", generic: "Task completed", tone: "success" },
  "scheduler.failed": { named: "{name} failed", generic: "Task failed", tone: "error" },
  "scheduler.retrying": { named: "{name} will retry", generic: "Task will retry", tone: "error" },
};

type LocalizedMessage = { key: MessageKey; values?: Record<string, string | number> } | { raw: string };

function renderMessage(message: LocalizedMessage, t: Translate): string {
  return "raw" in message ? message.raw : t(message.key, message.values);
}

function renderStatus(message: string | LocalizedMessage, t: Translate): string {
  return typeof message === "string" ? message : renderMessage(message, t);
}

function localMessage(key: MessageKey, values?: Record<string, string | number>): LocalizedMessage {
  return { key, values };
}

function taskActionMessage(key: MessageKey, name: string | null): LocalizedMessage {
  if (!name) return localMessage(key);
  const namedKeys: Partial<Record<MessageKey, MessageKey>> = {
    "Download started": "{name} started",
    "Download paused": "{name} paused",
    "Download resumed": "{name} resumed",
    "Task removed": "{name} removed",
    "Task queued": "{name} queued",
  };
  return localMessage(namedKeys[key] ?? key, { name });
}

interface ServerEvent {
  server: string;
  generation: number;
  sequence: number;
  event: string;
  data: Record<string, unknown>;
}

interface EventStreamStatus {
  server: string;
  generation: number;
  connected: boolean;
  error: string | null;
  resync_required: boolean;
}

const DEFAULT_SERVER = "127.0.0.1:39100";

const DOWNLOAD_FILTERS: Array<{ id: TaskFilter; label: MessageKey }> = [
  { id: "all", label: "All Downloads" },
  { id: "active", label: "Active" },
  { id: "queued", label: "Queued" },
  { id: "completed", label: "Completed" },
  { id: "failed", label: "Failed" },
];

const NAV_ITEMS: Array<{ id: Section; label: MessageKey; icon: IconName }> = [
  { id: "dashboard", label: "Dashboard", icon: "dashboard" },
  { id: "downloads", label: "Downloads", icon: "downloads" },
  { id: "trackers", label: "Trackers", icon: "trackers" },
  { id: "plugins", label: "Plugins", icon: "plugins" },
];

const SETTINGS_CARDS: Array<{
  id: Exclude<SettingsCategory, "home">;
  label: MessageKey;
  description: MessageKey;
  icon: IconName;
  available: boolean;
}> = [
  { id: "general", label: "General", description: "Startup, Server and system integration", icon: "general", available: true },
  { id: "appearance", label: "Appearance", description: "Theme and language preferences", icon: "appearance", available: true },
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

function formatBytesMaybe(bytes: number | null, t: Translate): string {
  return bytes === null ? t("Unknown size") : formatBytes(bytes);
}

function taskProgress(task: TaskItem): number | null {
  if (task.state === "Completed") return 100;
  if (task.total_bytes === null) return null;
  if (task.total_bytes === 0) return 0;
  return Math.max(0, Math.min(100, (task.downloaded_bytes / task.total_bytes) * 100));
}

function formatUpdatedAt(value: number | null, t: Translate): string {
  if (value === null) return t("Not updated yet");
  const seconds = Math.max(0, Math.round((Date.now() - value) / 1000));
  if (seconds < 5) return t("Updated just now");
  if (seconds < 60) return t("Updated {count}s ago", { count: seconds });
  return t("Updated {count}m ago", { count: Math.round(seconds / 60) });
}

function errorMessage(error: unknown, t: Translate): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : t("The operation failed");
}

function taskStateLabel(state: TaskState, t: Translate): string {
  return t(state);
}

function isLoopbackServerAddress(address: string): boolean {
  const match = /^(?:([^:[\]]+)|\[([^\]]+)\]):(\d+)$/.exec(address.trim());
  if (!match) return false;
  const port = Number(match[3]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return false;
  if (match[2]) {
    try {
      return new URL(`http://[${match[2]}]:${port}/`).hostname === "[::1]";
    } catch {
      return false;
    }
  }
  const host = match[1].toLowerCase();
  if (host === "localhost") return true;
  const octets = host.split(".");
  return octets.length === 4 && octets[0] === "127" && octets.every((octet) => /^(?:0|[1-9]\d{0,2})$/.test(octet) && Number(octet) <= 255);
}

function isLocalServerAddress(address: string): boolean {
  const value = address.trim();
  return isLoopbackServerAddress(value.startsWith("tls://") ? value.slice("tls://".length) : value);
}

function isAbsoluteServerFolder(directory: string): boolean {
  const value = directory.trim();
  return value.startsWith("/") || value.startsWith("\\\\") || /^[A-Za-z]:[\\/]/.test(value);
}

function serverDestination(directory: string, fileName: string): string | null {
  if (!isAbsoluteServerFolder(directory) || !isValidFileName(fileName)) return null;
  const folder = directory.trim();
  const windowsFolder = folder.startsWith("\\\\") || /^[A-Za-z]:[\\/]/.test(folder);
  const separator = windowsFolder && folder.includes("\\") ? "\\" : "/";
  return `${folder}${/[\\/]$/.test(folder) ? "" : separator}${fileName}`;
}

function isTlsServerAddress(address: string): boolean {
  const value = address.trim();
  if (!value.startsWith("tls://")) return false;
  const authority = value.slice("tls://".length);
  const match = /^(?:\[[^\]]+\]|[^:[\]/?]+):(\d+)$/.exec(authority);
  if (!match) return false;
  const port = Number(match[1]);
  return Number.isInteger(port) && port >= 1 && port <= 65535;
}

type IconName = "dashboard" | "downloads" | "trackers" | "plugins" | "notifications" | "settings" | "general" | "appearance" | "bittorrent" | "integration" | "network" | "advanced" | "about" | "search" | "info" | "list" | "more" | "plus";

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
    case "plus": return <svg {...common}><path d="M12 5v14M5 12h14" /></svg>;
  }
}

export default function App() {
  const [section, setSection] = useState<Section>("downloads");
  const sectionRef = useRef(section);
  const [filter, setFilter] = useState<TaskFilter>("all");
  const [settingsCategory, setSettingsCategory] = useState<SettingsCategory>("home");
  const [server, setServer] = useState(DEFAULT_SERVER);
  const [serverDraft, setServerDraft] = useState(DEFAULT_SERVER);
  const [refreshSeconds, setRefreshSeconds] = useState(5);
  const savedSettings = useRef({ server: DEFAULT_SERVER, refresh_interval_secs: 5 });
  const [language, setLanguage] = useState<LanguagePreference>("system");
  const savedLanguage = useRef<LanguagePreference>("system");
  const [settingsBusy, setSettingsBusy] = useState(false);
  const settingsSaveInFlight = useRef(false);
  const [systemLocale, setSystemLocale] = useState<Locale>(() => resolveLocale("system"));
  const effectiveLocale = language === "system" ? systemLocale : language;
  const t = useMemo<Translate>(() => (key, values) => translate(effectiveLocale, key, values), [effectiveLocale]);
  const tRef = useRef(t);
  tRef.current = t;
  const [settingsReady, setSettingsReady] = useState(false);
  const [credentialRevision, setCredentialRevision] = useState(0);
  const [taskSnapshot, setTaskSnapshot] = useState<ContextValue<{ items: TaskItem[]; updatedAt: number }> | null>(null);
  const [taskSelection, setTaskSelection] = useState<ContextValue<string | null> | null>(null);
  const [notificationTargetId, setNotificationTargetId] = useState<string | null>(null);
  const [connection, setConnection] = useState<ConnectionState>("disconnected");
  const [eventStreamConnected, setEventStreamConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [taskActions, setTaskActions] = useState<ContextValue<Record<string, TaskCommand | "start">> | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [error, setError] = useState<string | LocalizedMessage>("");
  const [notifications, setNotifications] = useState<Notice[]>([]);
  const nextNotificationId = useRef(0);
  const [showAdd, setShowAdd] = useState(false);
  const [showInspector, setShowInspector] = useState(true);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [newSource, setNewSource] = useState("");
  const [newDirectory, setNewDirectory] = useState("");
  const [newFileName, setNewFileName] = useState("");
  const [fileNameEdited, setFileNameEdited] = useState(false);
  const [nameProbePending, setNameProbePending] = useState(false);
  const nameProbeVersion = useRef(0);
  const [finalPath, setFinalPath] = useState("");
  const [addLoading, setAddLoading] = useState(false);
  const [destinationPicking, setDestinationPicking] = useState(false);
  const [addError, setAddError] = useState("");
  const localServer = isLocalServerAddress(server);
  const remoteFinalPath = localServer ? null : serverDestination(newDirectory, newFileName);
  const destinationPreview = localServer ? finalPath : remoteFinalPath;
  const addDialogRef = useRef<HTMLElement>(null);
  const addSourceRef = useRef<HTMLTextAreaElement>(null);
  const destinationButtonRef = useRef<HTMLButtonElement>(null);
  const addReturnFocus = useRef<HTMLElement | null>(null);
  const sidebarAddRef = useRef<HTMLButtonElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const searchToggleRef = useRef<HTMLButtonElement>(null);
  const searchReturnFocus = useRef<HTMLElement | null>(null);
  const eventRefreshTimer = useRef<number | null>(null);
  const streamTransition = useRef<Promise<void>>(Promise.resolve());
  const snapshotContext = useRef<TaskContext>({ server, credentialRevision });
  if (snapshotContext.current.server !== server || snapshotContext.current.credentialRevision !== credentialRevision) {
    snapshotContext.current = { server, credentialRevision };
  }
  const renderedContext = snapshotContext.current;
  const currentSnapshot = valueForContext(taskSnapshot, renderedContext);
  const tasks = currentSnapshot?.items ?? EMPTY_TASKS;
  const lastUpdatedAt = currentSnapshot?.updatedAt ?? null;
  const selectedId = valueForContext(taskSelection, renderedContext);
  const actionByTask = valueForContext(taskActions, renderedContext) ?? EMPTY_TASK_ACTIONS;
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;

  const queueStreamTransition = (operation: () => Promise<void>): Promise<void> => {
    const next = streamTransition.current.then(operation);
    streamTransition.current = next.catch(() => undefined);
    return next;
  };

  const pushNotification = (message: LocalizedMessage, tone: NoticeTone, taskId?: string): number => {
    const item: Notice = {
      id: ++nextNotificationId.current,
      message,
      tone,
      createdAt: Date.now(),
      read: sectionRef.current === "notifications",
      server,
      taskId,
    };
    setNotifications((current) => [item, ...current].slice(0, NOTIFICATION_LIMIT));
    if (tone === "success") {
      setNotice(item);
      setError("");
    } else {
      setError(message);
      setNotice(null);
    }
    return item.id;
  };

  const noticeId = notice?.id;
  useEffect(() => {
    if (noticeId === undefined) return undefined;
    const timer = window.setTimeout(() => {
      setNotice((current) => current?.id === noticeId ? null : current);
    }, SUCCESS_BANNER_DURATION_MS);
    return () => window.clearTimeout(timer);
  }, [noticeId]);

  const refreshTasks = async (address = server, options: { silent?: boolean } = {}): Promise<void> => {
    const context = snapshotContext.current;
    if (address !== context.server) return;
    const isCurrent = () => snapshotContext.current === context;
    const silent = options.silent ?? false;
    if (!address.trim()) {
      if (isCurrent()) {
        setConnection("error");
        pushNotification(localMessage("Enter a Server address first."), "error");
      }
      return;
    }
    if (!silent) {
      setLoading(true);
      setConnection("connecting");
    }
    try {
      const result = await invoke<TaskItem[]>("task_list", { server: address });
      if (!isCurrent()) return;
      if (!silent) setError("");
      setTaskSnapshot({ context, value: { items: result, updatedAt: Date.now() } });
      setTaskSelection((current) => {
        const previousId = valueForContext(current, context);
        return {
          context,
          value: previousId && result.some((task) => task.id === previousId) ? previousId : result[0]?.id ?? null,
        };
      });
      setConnection("connected");
    } catch (caught) {
      if (isCurrent()) {
        setConnection("error");
        setNotice(null);
        setError(errorMessage(caught, tRef.current));
      }
    } finally {
      if (!silent && isCurrent()) setLoading(false);
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
        savedSettings.current = { server: address, refresh_interval_secs: settings.refresh_interval_secs };
        savedLanguage.current = settings.language ?? "system";
        setLanguage(savedLanguage.current);
      })
      .catch((caught) => { if (mounted) setError(errorMessage(caught, tRef.current)); })
      .finally(() => { if (mounted) setSettingsReady(true); });
    return () => { mounted = false; };
  }, []);

  useLayoutEffect(() => {
    document.documentElement.lang = effectiveLocale;
  }, [effectiveLocale]);

  useEffect(() => {
    const updateSystemLanguage = () => setSystemLocale(resolveLocale("system"));
    window.addEventListener("languagechange", updateSystemLanguage);
    return () => window.removeEventListener("languagechange", updateSystemLanguage);
  }, []);

  useEffect(() => {
    if (!settingsReady || !server.trim()) return undefined;
    let active = true;
    let eventUnlisten: UnlistenFn | undefined;
    let statusUnlisten: UnlistenFn | undefined;
    let streamGeneration: number | undefined;
    const pendingStreamPayloads: Array<{ kind: "event"; payload: ServerEvent } | { kind: "status"; payload: EventStreamStatus }> = [];

    const scheduleRefresh = () => {
      if (!active || eventRefreshTimer.current !== null) return;
      eventRefreshTimer.current = window.setTimeout(() => {
        eventRefreshTimer.current = null;
        if (active) void refreshTasks(server, { silent: true });
      }, 120);
    };

    const handleServerEvent = (payload: ServerEvent) => {
      if (payload.server !== server || snapshotContext.current !== renderedContext) return;
      if (!payload.event.startsWith("task.") && !payload.event.startsWith("scheduler.")) return;
      scheduleRefresh();
      const eventMessage = SCHEDULER_MESSAGES[payload.event];
      if (!eventMessage) return;
      const taskId = typeof payload.data.task_id === "string" ? payload.data.task_id : null;
      const task = taskId ? tasksRef.current.find((item) => item.id === taskId) : null;
      const name = task ? taskDisplayName(task) : null;
      const message = name ? localMessage(eventMessage.named, { name }) : localMessage(eventMessage.generic);
      const noticeId = pushNotification(message, eventMessage.tone, taskId ?? undefined);
      if (!taskId || name) return;
      void invoke<TaskItem>("task_get", { server, taskId }).then((resolvedTask) => {
        if (!active || snapshotContext.current !== renderedContext || resolvedTask.id !== taskId) return;
        const resolvedName = taskDisplayName(resolvedTask);
        if (!resolvedName) return;
        const namedMessage = localMessage(eventMessage.named, { name: resolvedName });
        setNotifications((current) => current.map((item) => item.id === noticeId ? { ...item, message: namedMessage } : item));
        setNotice((current) => current?.id === noticeId ? { ...current, message: namedMessage } : current);
        if (eventMessage.tone === "error") setError((current) => current === message ? namedMessage : current);
      }).catch(() => undefined);
    };

    const handleEventStreamStatus = (payload: EventStreamStatus) => {
      if (!active || payload.server !== server || snapshotContext.current !== renderedContext) return;
      setEventStreamConnected(payload.connected);
      if (payload.connected || payload.resync_required) scheduleRefresh();
    };

    const handleStreamPayload = (item: { kind: "event"; payload: ServerEvent } | { kind: "status"; payload: EventStreamStatus }) => {
      if (!active || item.payload.server !== server) return;
      if (streamGeneration === undefined) {
        pendingStreamPayloads.push(item);
        return;
      }
      if (item.payload.generation !== streamGeneration) return;
      if (item.kind === "event") handleServerEvent(item.payload);
      else handleEventStreamStatus(item.payload);
    };

    void Promise.all([
      listen<ServerEvent>("server-event", ({ payload }) => handleStreamPayload({ kind: "event", payload })),
      listen<EventStreamStatus>("server-event-status", ({ payload }) => handleStreamPayload({ kind: "status", payload })),
    ])
      .then(([removeEvent, removeStatus]) => {
        if (!active) {
          removeEvent();
          removeStatus();
          return;
        }
        eventUnlisten = removeEvent;
        statusUnlisten = removeStatus;
        return queueStreamTransition(async () => {
          if (!active) return;
          const generation = await invoke<number>("start_event_stream", { server });
          streamGeneration = generation;
          if (active) {
            for (const item of pendingStreamPayloads.splice(0)) handleStreamPayload(item);
          }
        });
      })
      .catch((caught) => { if (active) { setEventStreamConnected(false); setError(errorMessage(caught, tRef.current)); } });

    return () => {
      active = false;
      if (eventRefreshTimer.current !== null) {
        window.clearTimeout(eventRefreshTimer.current);
        eventRefreshTimer.current = null;
      }
      eventUnlisten?.();
      statusUnlisten?.();
      pendingStreamPayloads.length = 0;
      setEventStreamConnected(false);
      void queueStreamTransition(async () => {
        if (streamGeneration !== undefined) await invoke("stop_event_stream", { generation: streamGeneration });
      }).catch(() => undefined);
    };
    // Event subscriptions follow the active server and its Keychain credential.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server, settingsReady, credentialRevision]);

  useEffect(() => {
    if (settingsReady) void refreshTasks();
    // Refresh the full snapshot when the active server or its credential changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server, settingsReady, credentialRevision]);

  const activeDownload = tasks.some((task) => task.state === "Downloading");
  const pollingSeconds = refreshSeconds > 0 && activeDownload ? 1 : refreshSeconds;

  useEffect(() => {
    if (!settingsReady || eventStreamConnected || pollingSeconds <= 0) return undefined;
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refreshTasks(server, { silent: true });
    }, pollingSeconds * 1000);
    return () => window.clearInterval(timer);
    // Keep fallback polling on the active server and credential revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connection, eventStreamConnected, pollingSeconds, server, settingsReady, credentialRevision]);

  const counts = useMemo(() => DOWNLOAD_FILTERS.reduce<Record<TaskFilter, number>>((result, item) => {
    result[item.id] = tasks.filter((task) => matchesFilter(task, item.id)).length;
    return result;
  }, { all: tasks.length, active: 0, queued: 0, completed: 0, failed: 0 }), [tasks]);
  const unreadNotificationCount = notifications.filter((item) => !item.read).length;

  const visibleTasks = useMemo(() => tasks.filter((task) => {
    if (!matchesFilter(task, filter)) return false;
    const query = searchQuery.trim().toLowerCase();
    return !query || [taskDisplayName(task) ?? t("Download"), task.source, task.destination, task.state, taskStateLabel(task.state, t)].some((value) => value.toLowerCase().includes(query));
  }), [tasks, filter, searchQuery, t]);
  const selectedTask = visibleTasks.find((task) => task.id === selectedId) ?? visibleTasks[0] ?? null;

  useLayoutEffect(() => {
    if (section !== "downloads" || !notificationTargetId) return;
    const row = Array.from(document.querySelectorAll<HTMLElement>(".task-row"))
      .find((element) => element.dataset.taskId === notificationTargetId);
    row?.scrollIntoView({ block: "nearest", behavior: "auto" });
    row?.querySelector<HTMLButtonElement>(".task-row-main")?.focus({ preventScroll: true });
    setNotificationTargetId(null);
  }, [section, notificationTargetId, visibleTasks]);

  useEffect(() => {
    setNewDirectory("");
    setFinalPath("");
  }, [server]);

  useEffect(() => {
    if (!showAdd || !localServer || newDirectory) return undefined;
    let active = true;
    void downloadDir()
      .then((directory) => {
        if (active) setNewDirectory((current) => current || directory);
      })
      .catch(() => {
        if (active) setAddError(tRef.current("Choose a download folder before adding a task."));
      });
    return () => { active = false; };
  }, [showAdd, localServer, newDirectory]);

  useEffect(() => {
    if (!localServer || !newDirectory || !isValidFileName(newFileName)) {
      setFinalPath("");
      return undefined;
    }
    let active = true;
    setFinalPath("");
    void join(newDirectory, newFileName)
      .then((path) => { if (active) setFinalPath(path); })
      .catch(() => { if (active) setFinalPath(""); });
    return () => { active = false; };
  }, [localServer, newDirectory, newFileName]);

  useEffect(() => {
    if (!showAdd || fileNameEdited || !isHttpSource(newSource)) return undefined;
    const version = nameProbeVersion.current;
    let active = true;
    const timer = window.setTimeout(() => {
      void invoke<string | null>("suggest_download_filename", { source: newSource.trim() })
        .then((name) => {
          if (active && version === nameProbeVersion.current && name && isValidFileName(name)) {
            setNewFileName(name);
          }
        })
        .catch(() => undefined)
        .finally(() => {
          if (active && version === nameProbeVersion.current) setNameProbePending(false);
        });
    }, 350);
    return () => { active = false; window.clearTimeout(timer); };
  }, [showAdd, newSource, fileNameEdited]);

  const changeSource = (source: string) => {
    nameProbeVersion.current += 1;
    setNewSource(source);
    if (!fileNameEdited) {
      setNewFileName(suggestedFileName(source));
      setNameProbePending(isHttpSource(source));
    }
  };

  const openAdd = (trigger?: HTMLElement) => {
    nameProbeVersion.current += 1;
    const active = document.activeElement;
    addReturnFocus.current = trigger ?? (active instanceof HTMLElement && active.tabIndex >= 0 ? active : sidebarAddRef.current);
    setAddError("");
    setNewSource("");
    setNewDirectory("");
    setNewFileName("");
    setFileNameEdited(false);
    setNameProbePending(false);
    setFinalPath("");
    setShowAdd(true);
  };

  const closeAdd = () => {
    if (!addLoading && !destinationPicking) {
      nameProbeVersion.current += 1;
      setNameProbePending(false);
      setShowAdd(false);
    }
  };

  const openSearch = (trigger?: HTMLElement) => {
    if (searchOpen) {
      searchInputRef.current?.focus();
      return;
    }
    const active = document.activeElement;
    searchReturnFocus.current = trigger ?? (active instanceof HTMLElement && active.tabIndex >= 0 ? active : searchToggleRef.current);
    setSearchOpen(true);
  };

  const closeSearch = () => {
    setSearchOpen(false);
    setSearchQuery("");
    const previous = searchReturnFocus.current;
    window.requestAnimationFrame(() => {
      (previous?.isConnected ? previous : searchToggleRef.current)?.focus();
    });
  };

  useEffect(() => {
    if (!showAdd) return undefined;
    addSourceRef.current?.focus();
    return () => {
      const previous = addReturnFocus.current;
      window.requestAnimationFrame(() => {
        (previous?.isConnected ? previous : sidebarAddRef.current)?.focus();
      });
    };
  }, [showAdd]);

  useEffect(() => {
    if (section === "downloads" && searchOpen) searchInputRef.current?.focus();
  }, [section, searchOpen]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || showAdd) return;
      if (event.metaKey && !event.altKey && !event.ctrlKey && !event.shiftKey && !event.repeat) {
        if (event.key.toLowerCase() === "n") {
          event.preventDefault();
          openAdd();
          return;
        }
        if (event.key.toLowerCase() === "f") {
          event.preventDefault();
          if (section !== "downloads") selectSection("downloads");
          openSearch();
          return;
        }
      }
      if (event.key === "Escape" && section === "downloads" && searchOpen) {
        event.preventDefault();
        closeSearch();
      }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
    // Shortcuts use the currently visible workspace and modal state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [section, searchOpen, showAdd]);

  const selectSection = (next: Section) => {
    sectionRef.current = next;
    setSection(next);
    if (next === "settings") setSettingsCategory("home");
    if (next === "downloads") setFilter("all");
    if (next === "notifications") setNotifications((current) => current.map((item) => item.read ? item : { ...item, read: true }));
  };

  const openNotificationTask = (item: Notice) => {
    if (snapshotContext.current !== renderedContext) return;
    if (!item.taskId || item.server !== server || !tasksRef.current.some((task) => task.id === item.taskId)) return;
    setSearchQuery("");
    setTaskSelection({ context: renderedContext, value: item.taskId });
    setNotificationTargetId(item.taskId);
    setShowInspector(true);
    selectSection("downloads");
  };

  const applyServer = async () => {
    if (settingsSaveInFlight.current || !settingsReady) return;
    const nextServer = serverDraft.trim();
    if (!nextServer) {
      pushNotification(localMessage("Enter a Server address first."), "error");
      return;
    }
    settingsSaveInFlight.current = true;
    setSettingsBusy(true);
    try {
      await invoke("save_settings", { settings: { server: nextServer, refresh_interval_secs: refreshSeconds, language: savedLanguage.current } });
      savedSettings.current = { server: nextServer, refresh_interval_secs: refreshSeconds };
      setServer(nextServer);
      pushNotification(localMessage("Settings saved"), "success");
    } catch (caught) {
      pushNotification({ raw: errorMessage(caught, t) }, "error");
    } finally {
      settingsSaveInFlight.current = false;
      setSettingsBusy(false);
    }
  };

  const changeLanguage = async (next: LanguagePreference): Promise<void> => {
    if (next === language || settingsSaveInFlight.current || !settingsReady) return;
    const previous = savedLanguage.current;
    settingsSaveInFlight.current = true;
    setSettingsBusy(true);
    setLanguage(next);
    try {
      await invoke("save_settings", { settings: { ...savedSettings.current, language: next } });
      savedLanguage.current = next;
    } catch (caught) {
      setLanguage(previous);
      pushNotification({ raw: errorMessage(caught, t) }, "error");
    } finally {
      settingsSaveInFlight.current = false;
      setSettingsBusy(false);
    }
  };

  const runTaskCommand = async (id: string, command: TaskCommand, successMessage: MessageKey): Promise<void> => {
    const context = renderedContext;
    if (snapshotContext.current !== context) return;
    const task = tasksRef.current.find((item) => item.id === id);
    if (!task) return;
    const name = taskDisplayName(task);
    setTaskActions((current) => ({ context, value: { ...(valueForContext(current, context) ?? EMPTY_TASK_ACTIONS), [id]: command } }));
    setNotice(null);
    setError("");
    try {
      await invokeTaskCommand(invoke, command, server, id);
      if (snapshotContext.current !== context) return;
      pushNotification(taskActionMessage(successMessage, name), "success", command === "task_remove" ? undefined : id);
      await refreshTasks();
    } catch (caught) {
      if (snapshotContext.current !== context) return;
      const detail = errorMessage(caught, t);
      pushNotification(name ? localMessage("{name}: {error}", { name, error: detail }) : { raw: detail }, "error", id);
    } finally {
      setTaskActions((current) => {
        if (current?.context !== context) return current;
        const next = { ...current.value }; delete next[id];
        return { context, value: next };
      });
    }
  };

  const startNextTask = async (taskId?: string): Promise<void> => {
    const context = renderedContext;
    if (snapshotContext.current !== context) return;
    if (taskId && !tasksRef.current.some((task) => task.id === taskId)) return;
    const actionKey = taskId ?? selectedId ?? "__next__";
    setTaskActions((current) => ({ context, value: { ...(valueForContext(current, context) ?? EMPTY_TASK_ACTIONS), [actionKey]: "start" } }));
    try {
      const startedId = await invoke<string>("task_start", { server });
      if (snapshotContext.current !== context) return;
      const startedTask = tasksRef.current.find((task) => task.id === startedId);
      const name = startedTask ? taskDisplayName(startedTask) : null;
      pushNotification(taskActionMessage("Download started", name), "success", startedId);
      await refreshTasks();
    } catch (caught) {
      if (snapshotContext.current !== context) return;
      pushNotification({ raw: errorMessage(caught, t) }, "error");
    } finally {
      setTaskActions((current) => {
        if (current?.context !== context) return current;
        const next = { ...current.value }; delete next[actionKey];
        return { context, value: next };
      });
    }
  };

  const chooseDestination = async (): Promise<void> => {
    if (!localServer) return;
    setDestinationPicking(true);
    setAddError("");
    try {
      const directory = await open({
        title: t("Choose download folder"),
        directory: true,
        multiple: false,
        defaultPath: newDirectory || undefined,
      });
      if (typeof directory === "string") setNewDirectory(directory);
    } catch (caught) {
      setAddError(errorMessage(caught, t));
    } finally {
      setDestinationPicking(false);
      window.requestAnimationFrame(() => destinationButtonRef.current?.focus());
    }
  };

  const createAndQueue = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    if (nameProbePending) return;
    if (hasHardLineBreak(newSource)) {
      setAddError(t("Enter one URL without line breaks."));
      return;
    }
    if (!isHttpSource(newSource)) {
      setAddError(t("Enter an HTTP or HTTPS URL."));
      return;
    }
    if (!isValidFileName(newFileName)) {
      setAddError(t("Enter a valid file name without slashes or control characters."));
      return;
    }
    if (!newDirectory.trim()) {
      setAddError(t(localServer ? "Choose a download folder before adding a task." : "Enter an absolute folder path on the Server host."));
      return;
    }
    if (!localServer && !isAbsoluteServerFolder(newDirectory)) {
      setAddError(t("Enter an absolute folder path on the Server host."));
      return;
    }
    setAddLoading(true);
    setAddError("");
    try {
      const destination = localServer ? await join(newDirectory, newFileName) : serverDestination(newDirectory, newFileName);
      if (!destination) {
        setAddError(t("Enter an absolute folder path on the Server host."));
        return;
      }
      if (localServer && await invoke<boolean>("destination_exists", { path: destination })) {
        setAddError(t("A file already exists at this destination. Choose another file name."));
        return;
      }
      const currentTasks = await invoke<TaskItem[]>("task_list", { server });
      if (currentTasks.some((task) => task.destination === destination)) {
        setAddError(t("A download task already uses this destination. Choose another file name."));
        return;
      }
      const result = await addDownload(invoke, server, newSource.trim(), destination);
      if (!result.queued) {
        setAddError(t("Task created, but queueing failed: {error}", { error: errorMessage(result.error, t) }));
        await refreshTasks();
        return;
      }
      nameProbeVersion.current += 1;
      setNewSource(""); setNewFileName(""); setFileNameEdited(false); setNameProbePending(false); setShowAdd(false);
      selectSection("downloads");
      pushNotification(localMessage("{name} added and queued", { name: newFileName }), "success", result.id);
      await refreshTasks();
    } catch (caught) {
      setAddError(errorMessage(caught, t));
    } finally {
      setAddLoading(false);
    }
  };

  const handleAddDialogKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeAdd();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(addDialogRef.current?.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
    ) ?? []);
    if (focusable.length === 0) {
      event.preventDefault();
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && (document.activeElement === first || !addDialogRef.current?.contains(document.activeElement))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (document.activeElement === last || !addDialogRef.current?.contains(document.activeElement))) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <TranslationProvider translateMessage={t}>
    <main className="app-shell">
      <aside className="sidebar" aria-label={t("Nexum navigation")} inert={showAdd}>
        <div className="sidebar-topbar">
          <span className="sidebar-brand">NEXUM</span>
          <div className="sidebar-top-actions">
            <button className="sidebar-icon-button" onClick={() => setShowInspector((current) => !current)} title={t("Toggle inspector")} aria-label={showInspector ? t("Hide task details") : t("Show task details")} aria-pressed={showInspector}><Icon name="list" size={18} /></button>
            <button ref={sidebarAddRef} className="sidebar-icon-button add-sidebar-button" onClick={(event) => openAdd(event.currentTarget)} title={t("Add download")} aria-label={t("Add download")}><Icon name="plus" size={18} /></button>
          </div>
        </div>
        <nav className="primary-nav">
          {NAV_ITEMS.map((item) => (
            <button className={`nav-item ${section === item.id ? "selected" : ""}`} key={item.id} onClick={() => selectSection(item.id)} aria-current={section === item.id ? "page" : undefined}>
              <Icon name={item.icon} size={20} /><span>{t(item.label)}</span>
              {item.id === "downloads" && counts.active > 0 && <span className="nav-count">{counts.active}</span>}
            </button>
          ))}
        </nav>
        <div className="sidebar-spacer" />
        <button className={`nav-item ${section === "notifications" ? "selected" : ""}`} onClick={() => selectSection("notifications")} aria-current={section === "notifications" ? "page" : undefined}>
          <Icon name="notifications" size={20} /><span>{t("Notifications")}</span>{unreadNotificationCount > 0 && <span className="nav-count" aria-label={t("{count} unread notifications", { count: unreadNotificationCount })}>{unreadNotificationCount}</span>}
        </button>
        <div className="sidebar-divider" />
        <button className={`nav-item ${section === "settings" ? "selected" : ""}`} onClick={() => selectSection("settings")} aria-current={section === "settings" ? "page" : undefined}><Icon name="settings" size={20} /><span>{t("Settings")}</span></button>
      </aside>

      <section className="workspace" inert={showAdd}>
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
            searchInputRef={searchInputRef}
            searchToggleRef={searchToggleRef}
            onFilterChange={setFilter}
            onSelect={(id) => setTaskSelection({ context: renderedContext, value: id })}
            onSearchOpen={(trigger) => { if (searchOpen) closeSearch(); else openSearch(trigger); }}
            onSearchQuery={setSearchQuery}
            onToggleInspector={() => setShowInspector((current) => !current)}
            onRefresh={() => void refreshTasks()}
            onDismissNotice={() => setNotice(null)}
            onDismissError={() => setError("")}
            onAdd={openAdd}
            onPause={(id) => void runTaskCommand(id, "task_pause", "Download paused")}
            onResume={(id) => void runTaskCommand(id, "task_resume", "Download resumed")}
            onRemove={(id) => void runTaskCommand(id, "task_remove", "Task removed")}
            onQueue={(id) => void runTaskCommand(id, "task_queue", "Task queued")}
            onStart={(id) => void startNextTask(id)}
          />
        )}
        {section === "dashboard" && <DashboardView tasks={tasks} connection={connection} onAdd={openAdd} onDownloads={() => selectSection("downloads")} />}
        {section === "trackers" && <UnavailableView icon="trackers" title={t("Trackers")} description={t("Tracker discovery and health checks will appear here when the BitTorrent engine is connected.")} />}
        {section === "plugins" && <UnavailableView icon="plugins" title={t("Plugins")} description={t("The plugin manager is currently a runtime foundation. Executable providers and a plugin catalog are planned.")} />}
        {section === "notifications" && <NotificationsView notifications={notifications} locale={effectiveLocale} server={server} tasks={tasks} onDownloads={() => selectSection("downloads")} onOpenTask={openNotificationTask} onClear={(id) => setNotifications((current) => current.filter((item) => item.id !== id))} onClearAll={() => setNotifications([])} />}
        {section === "settings" && <SettingsView category={settingsCategory} server={server} serverDraft={serverDraft} settingsReady={settingsReady} settingsBusy={settingsBusy} connection={connection} refreshSeconds={refreshSeconds} language={language} onCategory={setSettingsCategory} onServerChange={setServerDraft} onRefreshSecondsChange={setRefreshSeconds} onLanguageChange={(next) => void changeLanguage(next)} onApply={() => void applyServer()} onCredentialChanged={() => setCredentialRevision((current) => current + 1)} />}
      </section>

      <div className="sr-only" role="alert">{renderStatus(error, t)}</div>
      <div className="sr-only" role="status">{notice ? renderMessage(notice.message, t) : ""}</div>

      {showAdd && (
        <div className="modal-backdrop" role="presentation" onMouseDown={closeAdd}>
          <section ref={addDialogRef} className="modal" role="dialog" aria-modal="true" aria-labelledby="add-download-title" onKeyDown={handleAddDialogKeyDown} onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-header"><div><span className="eyebrow">{t("New task")}</span><h2 id="add-download-title">{t("Add Download")}</h2></div><button className="close-button" onClick={closeAdd} disabled={addLoading || destinationPicking} aria-label={t("Close Add Download")}>×</button></div>
            <form onSubmit={createAndQueue}>
              <label htmlFor="add-source">{t("Source URL")}</label>
              <textarea id="add-source" ref={addSourceRef} className="source-url-input" value={newSource} onChange={(event) => changeSource(event.target.value)} placeholder="https://example.com/file.zip" rows={3} wrap="soft" aria-describedby="add-source-help" required />
              <p id="add-source-help" className="add-field-help">{t("Long URLs wrap automatically. Enter one URL per download.")}</p>
              <label htmlFor="add-file-name">{t("File name")}</label>
              <input id="add-file-name" value={newFileName} onChange={(event) => { nameProbeVersion.current += 1; setNewFileName(event.target.value); setFileNameEdited(true); setNameProbePending(false); }} aria-describedby="add-name-help" aria-invalid={newFileName.length > 0 && !isValidFileName(newFileName)} placeholder={t("Enter a file name")} required />
              <p id="add-name-help" className="add-field-help" aria-live="polite">{t(nameProbePending ? "Checking file name from the response… You can edit it now." : "Suggested from the URL or response; you can edit it before adding the download.")}</p>
              <label htmlFor="add-directory">{t(localServer ? "Save folder" : "Save folder on Server")}</label>
              {localServer ? (
                <div className="input-with-action"><input id="add-directory" value={newDirectory} readOnly title={newDirectory} placeholder={t("Choose a download folder")} aria-describedby="add-directory-help add-final-path" /><button ref={destinationButtonRef} type="button" className="button chooser-button" onClick={() => void chooseDestination()} disabled={addLoading || destinationPicking}>{destinationPicking ? t("Choosing…") : t("Choose…")}</button></div>
              ) : (
                <input id="add-directory" className="server-directory-input" value={newDirectory} onChange={(event) => setNewDirectory(event.target.value)} placeholder={t("Enter an absolute folder path on the Server host.")} aria-describedby="add-directory-help add-final-path" aria-invalid={Boolean(newDirectory.trim()) && !isAbsoluteServerFolder(newDirectory)} autoComplete="off" spellCheck={false} required />
              )}
              <p id="add-directory-help" className="add-field-help">{localServer ? t("Defaults to this Mac's Downloads folder.") : t("The Server at {server} writes downloads to this folder on its own filesystem.", { server })}</p>
              <p id="add-final-path" className="add-final-path">{t("Destination")}: <output>{destinationPreview || t(localServer ? "Choose a folder and file name" : "Enter a Server folder and file name")}</output></p>
              {addError && <p className="form-error" role="alert">{addError}</p>}
              <div className="modal-actions"><button type="button" className="button" onClick={closeAdd} disabled={addLoading || destinationPicking}>{t("Cancel")}</button><button type="submit" className="button primary" disabled={addLoading || destinationPicking || nameProbePending || !newSource.trim() || !newDirectory || !newFileName}>{addLoading ? t("Adding…") : nameProbePending ? t("Checking name…") : t("Add and Queue")}</button></div>
            </form>
          </section>
        </div>
      )}
    </main>
    </TranslationProvider>
  );
}

function DownloadsView({
  tasks, visibleTasks, counts, filter, selectedTask, actionByTask, connection, eventStreamConnected, server, loading, error, notice, lastUpdatedAt, showInspector, searchOpen, searchQuery,
  searchInputRef, searchToggleRef, onFilterChange, onSelect, onSearchOpen, onSearchQuery, onToggleInspector, onRefresh, onDismissNotice, onDismissError, onAdd, onPause, onResume, onRemove, onQueue, onStart,
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
  error: string | LocalizedMessage;
  notice: Notice | null;
  lastUpdatedAt: number | null;
  showInspector: boolean;
  searchOpen: boolean;
  searchQuery: string;
  searchInputRef: RefObject<HTMLInputElement | null>;
  searchToggleRef: RefObject<HTMLButtonElement | null>;
  onFilterChange: (filter: TaskFilter) => void;
  onSelect: (id: string) => void;
  onSearchOpen: (trigger?: HTMLElement) => void;
  onSearchQuery: (query: string) => void;
  onToggleInspector: () => void;
  onRefresh: () => void;
  onDismissNotice: () => void;
  onDismissError: () => void;
  onAdd: (trigger?: HTMLElement) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onRemove: (id: string) => void;
  onQueue: (id: string) => void;
  onStart: (id: string) => void;
}) {
  const t = useTranslation();
  const title = DOWNLOAD_FILTERS.find((item) => item.id === filter)?.label ?? "All Downloads";
  return (
    <>
      <header className="content-header">
        <div className="content-title"><select value={filter} onChange={(event) => onFilterChange(event.target.value as TaskFilter)} aria-label={t("Download filter")}>{DOWNLOAD_FILTERS.map((item) => <option value={item.id} key={item.id}>{t(item.label)}</option>)}</select><span className="count-pill">{counts[filter]}/{tasks.length}</span></div>
        <div className="header-tools"><button className="round-tool" title={t("More actions (planned)")} aria-label={t("More actions (planned)")} disabled><Icon name="more" /></button><button className={`round-tool ${showInspector ? "active" : "muted"}`} onClick={onToggleInspector} title={t("Toggle task details")} aria-label={showInspector ? t("Hide task details") : t("Show task details")} aria-pressed={showInspector}><Icon name="info" /></button><button ref={searchToggleRef} className="round-tool" onClick={(event) => onSearchOpen(event.currentTarget)} title={t("Search downloads")} aria-label={t("Search downloads")} aria-expanded={searchOpen} aria-controls={searchOpen ? "download-search" : undefined}><Icon name="search" /></button></div>
      </header>
      {searchOpen && <div id="download-search" className="search-row"><Icon name="search" size={16} /><input ref={searchInputRef} value={searchQuery} onChange={(event) => onSearchQuery(event.target.value)} placeholder={t("Search downloads")} aria-label={t("Search downloads")} /></div>}
      {(error || notice) && <div className={`message-banner ${error ? "error" : "success"}`}><span>{error ? renderStatus(error, t) : notice && renderMessage(notice.message, t)}</span><div className="message-banner-actions">{error && <button type="button" onClick={onRefresh}>{t("Retry")}</button>}<button type="button" onClick={error ? onDismissError : onDismissNotice} aria-label={t("Dismiss message")}>{t("Dismiss")}</button></div></div>}
      <div className="download-content">
        {visibleTasks.length === 0 ? (
          <EmptyDownloads onAdd={onAdd} hasFilter={filter !== "all" || Boolean(searchQuery)} />
        ) : (
          <div className={`download-layout ${showInspector ? "with-inspector" : ""}`}>
            <section className="task-list" aria-label={t("Downloads")}>
              {visibleTasks.map((task) => (
                <Fragment key={task.id}>
                  <TaskRow task={task} selected={selectedTask?.id === task.id} action={actionByTask[task.id]} onSelect={() => onSelect(task.id)} onPause={() => onPause(task.id)} onResume={() => onResume(task.id)} onRemove={() => onRemove(task.id)} onQueue={() => onQueue(task.id)} onStart={() => onStart(task.id)} />
                  {showInspector && selectedTask?.id === task.id && <TaskInspector task={task} server={server} className="narrow-inspector" />}
                </Fragment>
              ))}
            </section>
            {showInspector && <TaskInspector task={selectedTask} server={server} className="desktop-inspector" />}
          </div>
        )}
      </div>
      <footer className="status-bar"><span className="status-mode">HTTP/HTTPS</span><span className="status-transfer"><span>↓ —</span><span>↑ —</span></span><span className="status-spacer" /><span className="status-item"><span className={`status-light ${connection}`} />{connection === "connected" ? t("Server connected") : t("Server unavailable")}</span><span className="status-item"><span className={`status-light ${eventStreamConnected ? "ready" : "connecting"}`} />{eventStreamConnected ? t("Live updates") : t("Polling fallback")}</span><span className="status-item"><span className="status-light ready" />{loading ? t("Syncing") : formatUpdatedAt(lastUpdatedAt, t)}</span></footer>
      <button className="floating-add" onClick={(event) => onAdd(event.currentTarget)} title={t("Add download")} aria-label={t("Add download")}><Icon name="plus" size={24} /></button>
      <span className="sr-only">{t(title)}</span>
    </>
  );
}

function TaskRow({ task, selected, action, onSelect, onPause, onResume, onRemove, onQueue, onStart }: { task: TaskItem; selected: boolean; action?: TaskCommand | "start"; onSelect: () => void; onPause: () => void; onResume: () => void; onRemove: () => void; onQueue: () => void; onStart: () => void }) {
  const t = useTranslation();
  const progress = taskProgress(task);
  const name = taskDisplayName(task) ?? t("Download");
  const selectionLabel = (progress === null
    ? t("Select {name}, {state}, progress unknown", { name, state: taskStateLabel(task.state, t) })
    : t("Select {name}, {state}, {percent} percent complete", { name, state: taskStateLabel(task.state, t), percent: Math.round(progress) }))
    + (task.error ? t(", error: {error}", { error: task.error }) : "");
  const handleSelectionKeyDown = (event: ReactKeyboardEvent<HTMLButtonElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const buttons = Array.from(event.currentTarget.closest(".task-list")?.querySelectorAll<HTMLButtonElement>(".task-row-main") ?? []);
    const index = buttons.indexOf(event.currentTarget);
    if (index < 0) return;
    event.preventDefault();
    const nextIndex = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, index + (event.key === "ArrowDown" ? 1 : -1)));
    buttons[nextIndex].focus();
    buttons[nextIndex].click();
  };
  return <article className={`task-row ${selected ? "selected" : ""}`} data-task-id={task.id}>
    <button type="button" className="task-row-main" onClick={onSelect} onKeyDown={handleSelectionKeyDown} aria-pressed={selected} aria-label={selectionLabel}>
      <span className="task-row-heading"><strong>{name}</strong><span className={`state-badge state-${task.state.toLowerCase()}`}>{taskStateLabel(task.state, t)}</span></span><span className="task-source" title={task.source}>{task.source}</span><span className="progress-track" aria-hidden="true"><span className={progress === null ? (task.state === "Downloading" ? "indeterminate active" : "indeterminate") : ""} style={progress === null ? undefined : { width: `${progress}%` }} /></span>
    </button>
    <div className="task-row-footer">
      <div className="task-row-meta"><span>{formatBytes(task.downloaded_bytes)} / {formatBytesMaybe(task.total_bytes, t)}</span><span className="task-destination" title={task.destination}>{task.destination}</span></div>
      <div className="task-row-actions">{task.state === "Downloading" && <button className="row-action" disabled={Boolean(action)} onClick={onPause} aria-label={t("Pause {name}", { name })}>{action === "task_pause" ? t("Pausing…") : t("Pause")}</button>}{task.state === "Paused" && <button className="row-action" disabled={Boolean(action)} onClick={onResume} aria-label={t("Resume {name}", { name })}>{action === "task_resume" ? t("Resuming…") : t("Resume")}</button>}{task.state === "Queued" && <button className="row-action" disabled={Boolean(action)} onClick={onStart} aria-label={t("Start next queued task")}>{action === "start" ? t("Starting…") : t("Start")}</button>}{task.state === "Created" && <button className="row-action" disabled={Boolean(action)} onClick={onQueue} aria-label={t("Queue {name}", { name })}>{action === "task_queue" ? t("Queueing…") : t("Queue")}</button>}<button className="row-action danger-text" disabled={Boolean(action)} onClick={onRemove} aria-label={t("Remove {name}", { name })}>{action === "task_remove" ? t("Removing…") : t("Remove")}</button></div>
    </div>
    {task.error && <span className="task-error">{task.error}</span>}
  </article>;
}

function TaskInspector({ task, server, className = "" }: { task: TaskItem | null; server: string; className?: string }) {
  const t = useTranslation();
  if (!task) return <aside className={`inspector inspector-empty ${className}`}><span className="inspector-icon"><Icon name="info" size={24} /></span><p>{t("Select a download to inspect it.")}</p></aside>;
  const progress = taskProgress(task);
  const name = taskDisplayName(task) ?? t("Download");
  return <aside className={`inspector ${className}`} aria-label={t("Task details")}><span className="eyebrow">{t("Task details")}</span><h2>{name}</h2><span className={`state-badge state-${task.state.toLowerCase()}`}>{taskStateLabel(task.state, t)}</span><div className="inspector-progress"><div className="progress-track" role="progressbar" aria-label={t("{name} download progress", { name })} aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress === null ? undefined : Math.round(progress)} aria-valuetext={task.total_bytes === null ? `${taskStateLabel(task.state, t)}; ${t("{bytes} downloaded; total size unknown", { bytes: formatBytes(task.downloaded_bytes) })}` : undefined}><span className={progress === null ? (task.state === "Downloading" ? "indeterminate active" : "indeterminate") : ""} style={progress === null ? undefined : { width: `${progress}%` }} /></div><strong>{formatBytes(task.downloaded_bytes)} / {formatBytesMaybe(task.total_bytes, t)}</strong></div><dl className="detail-list"><div><dt>{t("Source")}</dt><dd title={task.source}>{task.source}</dd></div><div><dt>{t("Destination")}</dt><dd title={task.destination}>{task.destination}</dd></div><div><dt>{t("Server")}</dt><dd>{server}</dd></div></dl>{task.error && <div className="inspector-error"><strong>{t("Latest error")}</strong><p>{task.error}</p><button className="text-button" onClick={() => void navigator.clipboard?.writeText(task.error ?? "")}>{t("Copy error")}</button></div>}</aside>;
}

function EmptyDownloads({ onAdd, hasFilter }: { onAdd: (trigger?: HTMLElement) => void; hasFilter: boolean }) {
  const t = useTranslation();
  return <div className="empty-state"><div className="empty-art" aria-hidden="true"><div className="art-block art-one" /><div className="art-block art-two" /><div className="art-block art-three" /><div className="art-block art-four" /></div><h2>{hasFilter ? t("No matching downloads") : t("No downloads yet")}</h2><p>{hasFilter ? t("Try another filter or search term.") : t("Press + to add one")}</p>{hasFilter && <button className="button" onClick={(event) => onAdd(event.currentTarget)}>{t("Add Download")}</button>}</div>;
}

function DashboardView({ tasks, connection, onAdd, onDownloads }: { tasks: TaskItem[]; connection: ConnectionState; onAdd: (trigger?: HTMLElement) => void; onDownloads: () => void }) {
  const t = useTranslation();
  const active = tasks.filter((task) => task.state === "Downloading").length;
  const queued = tasks.filter((task) => task.state === "Queued" || task.state === "Retrying").length;
  const completed = tasks.filter((task) => task.state === "Completed").length;
  return <div className="dashboard-page"><div className="page-heading"><div><span className="eyebrow">{t("Overview")}</span><h1>{t("Dashboard")}</h1><p>{t("Current activity from your Nexum Server.")}</p></div><button className="button primary dashboard-add" onClick={(event) => onAdd(event.currentTarget)}><Icon name="plus" size={16} />{t("Add Download")}</button></div><div className="metric-grid"><MetricCard label={t("All downloads")} value={tasks.length} icon="downloads" /><MetricCard label={t("Active")} value={active} icon="trackers" /><MetricCard label={t("Queued")} value={queued} icon="list" /><MetricCard label={t("Completed")} value={completed} icon="general" /></div><section className="dashboard-card"><div className="dashboard-card-heading"><div><span className="eyebrow">{t("Recent activity")}</span><h2>{tasks.length ? t("Latest downloads") : t("Nothing here yet")}</h2></div><button className="text-button" onClick={onDownloads}>{t("View downloads")}</button></div>{tasks.length ? tasks.slice(0, 5).map((task) => <div className="activity-row" key={task.id}><span className={`status-light ${task.state === "Completed" ? "ready" : "connected"}`} /><strong>{taskDisplayName(task) ?? t("Download")}</strong><span>{taskStateLabel(task.state, t)}</span></div>) : <p className="dashboard-empty">{t("Add an HTTP or HTTPS URL to see it here.")}</p>}</section><div className="dashboard-connection"><span className={`status-light ${connection}`} />{connection === "connected" ? t("Server connected") : t("Start nexum-server to connect")}</div></div>;
}

function MetricCard({ label, value, icon }: { label: string; value: number; icon: IconName }) {
  return <div className="metric-card"><div className="metric-icon"><Icon name={icon} size={20} /></div><span>{label}</span><strong>{value}</strong></div>;
}

function UnavailableView({ icon, title, description }: { icon: IconName; title: string; description: string }) {
  const t = useTranslation();
  return <div className="unavailable-page"><div className="unavailable-icon"><Icon name={icon} size={34} /></div><span className="eyebrow">{t("Planned capability")}</span><h1>{title}</h1><p>{description}</p><span className="planned-badge">{t("Not connected yet")}</span></div>;
}

function NotificationsView({ notifications, locale, server, tasks, onDownloads, onOpenTask, onClear, onClearAll }: {
  notifications: Notice[];
  locale: Locale;
  server: string;
  tasks: TaskItem[];
  onDownloads: () => void;
  onOpenTask: (item: Notice) => void;
  onClear: (id: number) => void;
  onClearAll: () => void;
}) {
  const t = useTranslation();
  const taskIds = new Set(tasks.map((task) => task.id));
  return <div className="notifications-page">
    <div className="page-heading">
      <div><span className="eyebrow">{t("Activity center")}</span><h1>{t("Notifications")}</h1><p>{t("Task and connection events from this Desktop session.")}</p></div>
      <div className="notification-heading-actions"><button className="button" onClick={onDownloads}>{t("View downloads")}</button>{notifications.length > 0 && <button className="button" onClick={onClearAll}>{t("Clear all")}</button>}</div>
    </div>
    {notifications.length === 0 ? <div className="notifications-empty"><Icon name="notifications" size={32} /><h2>{t("No notifications")}</h2><p>{t("Completion and failure events will appear here.")}</p></div> : <div className="notification-list">{notifications.map((item) => {
      const message = renderMessage(item.message, t);
      const canOpenTask = Boolean(item.taskId && item.server === server && taskIds.has(item.taskId));
      return <div className={`notification-row ${item.tone} ${item.read ? "" : "unread"}`} key={item.id}>
        <span className={`status-light ${item.tone === "success" ? "ready" : "error"}`} aria-hidden="true" />
        <div className="notification-body">
          {canOpenTask ? <button type="button" className="notification-open" onClick={() => onOpenTask(item)} aria-label={t("Open {message} in Downloads", { message })}><strong>{message}</strong></button> : <strong>{message}</strong>}
          <span className="notification-time">{new Date(item.createdAt).toLocaleTimeString(locale)}</span>
        </div>
        <button type="button" className="notification-dismiss" onClick={() => onClear(item.id)} aria-label={t("Clear notification: {message}", { message })}>{t("Clear")}</button>
      </div>;
    })}</div>}
  </div>;
}

function SettingsView({ category, server, serverDraft, settingsReady, settingsBusy, connection, refreshSeconds, language, onCategory, onServerChange, onRefreshSecondsChange, onLanguageChange, onApply, onCredentialChanged }: {
  category: SettingsCategory;
  server: string;
  serverDraft: string;
  settingsReady: boolean;
  settingsBusy: boolean;
  connection: ConnectionState;
  refreshSeconds: number;
  language: LanguagePreference;
  onCategory: (category: SettingsCategory) => void;
  onServerChange: (value: string) => void;
  onRefreshSecondsChange: (value: number) => void;
  onLanguageChange: (value: LanguagePreference) => void;
  onApply: () => void;
  onCredentialChanged: () => void;
}) {
  const t = useTranslation();
  const categoryButtons = useRef<Partial<Record<SettingsCategory, HTMLButtonElement | null>>>({});
  const returnFocusCategory = useRef<SettingsCategory | null>(null);

  useEffect(() => {
    if (category !== "home" || !returnFocusCategory.current) return;
    const previous = returnFocusCategory.current;
    returnFocusCategory.current = null;
    categoryButtons.current[previous]?.focus();
  }, [category]);

  const goBack = () => {
    returnFocusCategory.current = category;
    onCategory("home");
  };

  if (category !== "home") {
    const card = SETTINGS_CARDS.find((item) => item.id === category);
    if (category === "general") return <SettingsDetail title={t(card?.label ?? "General")} icon="general" onBack={goBack}>
      <div className="settings-card">
        <div className="settings-card-heading"><div><span className="eyebrow">{t("Connection")}</span><h2>{t("Server")}</h2></div><span className={`status-pill ${connection}`}>{t(connection)}</span></div>
        <label>{t("Server address")}<input value={serverDraft} onChange={(event) => onServerChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") onApply(); }} placeholder={DEFAULT_SERVER} disabled={settingsBusy} /></label>
        <p className="field-help">{t("The Server runs as a separate process. The saved address is kept in the Mac application support directory.")}</p>
        <button className="button primary" onClick={onApply} disabled={settingsBusy || !settingsReady}>{t("Save and test connection")}</button>
      </div>
      <CredentialSettings key={server} server={server} serverDraft={serverDraft} settingsReady={settingsReady} onCredentialChanged={onCredentialChanged} />
    </SettingsDetail>;
    if (category === "downloads") return <SettingsDetail title={t(card?.label ?? "Downloads")} icon="downloads" onBack={goBack}>
      <div className="settings-card"><span className="eyebrow">{t("Updates")}</span><h2>{t("Refresh policy")}</h2><p className="field-help">{t("Idle tasks refresh every five seconds. Active downloads refresh every second.")}</p><select value={refreshSeconds} onChange={(event) => onRefreshSecondsChange(Number(event.target.value))} aria-label={t("Refresh policy")} disabled={settingsBusy}><option value={0}>{t("Manual")}</option><option value={5}>{t("Automatic")}</option></select><button className="button primary settings-save" onClick={onApply} disabled={settingsBusy || !settingsReady}>{t("Save settings")}</button></div>
    </SettingsDetail>;
    if (category === "appearance") return <SettingsDetail title={t(card?.label ?? "Appearance")} icon="appearance" onBack={goBack}>
      <div className="settings-card"><span className="eyebrow">{t("Appearance")}</span><h2>{t("System appearance and language")}</h2><p className="field-help">{t("Appearance follows the Mac system setting.")}</p><label>{t("Language")}<select value={language} onChange={(event) => onLanguageChange(event.target.value as LanguagePreference)} disabled={settingsBusy || !settingsReady}><option value="system">{t("Follow system")}</option><option value="en">{t("English")}</option><option value="zh-CN">{t("Simplified Chinese")}</option></select></label><p className="field-help">{t("Language changes are saved immediately.")}</p></div>
    </SettingsDetail>;
    return <SettingsDetail title={t(card?.label ?? "Settings")} icon={card?.icon ?? "settings"} onBack={goBack}>
      <div className="settings-card settings-planned"><span className="eyebrow">{t("Planned capability")}</span><h2>{t(card?.label ?? "Settings")}</h2><p>{t("{description}. This page is reserved so the navigation can remain stable while the underlying engine and protocol are implemented.", { description: t(card?.description ?? "Planned capability") })}</p><span className="planned-badge">{t("Coming later")}</span></div>
    </SettingsDetail>;
  }
  return <div className="settings-page"><div className="page-heading"><div><span className="eyebrow">{t("Client preferences")}</span><h1>{t("Settings")}</h1><p>{t("Configure Nexum without mixing connection options into the download list.")}</p></div></div><div className="settings-grid">{SETTINGS_CARDS.map((card) => <button ref={(element) => { categoryButtons.current[card.id] = element; }} className={`settings-card-tile ${card.available ? "" : "planned"}`} key={card.id} onClick={() => onCategory(card.id)}><span className={`settings-tile-icon icon-${card.id}`}><Icon name={card.icon} size={28} /></span><strong>{t(card.label)}</strong><p>{t(card.description)}</p>{!card.available && <span className="planned-badge">{t("Planned")}</span>}</button>)}</div></div>;
}

function CredentialSettings({ server, serverDraft, settingsReady, onCredentialChanged }: { server: string; serverDraft: string; settingsReady: boolean; onCredentialChanged: () => void }) {
  const t = useTranslation();
  const tRef = useRef(t);
  tRef.current = t;
  const secretInput = useRef<HTMLInputElement>(null);
  const currentServer = useRef(server);
  currentServer.current = server;
  const [scheme, setScheme] = useState<CredentialScheme>("Bearer");
  const [configuredScheme, setConfiguredScheme] = useState<CredentialScheme | null>(null);
  const [statusFailed, setStatusFailed] = useState(false);
  const [statusLoading, setStatusLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [hasSecret, setHasSecret] = useState(false);
  const [feedback, setFeedback] = useState<{ tone: NoticeTone; message: LocalizedMessage } | null>(null);

  const loopback = isLoopbackServerAddress(server);
  const tls = isTlsServerAddress(server);
  const draftSaved = serverDraft.trim() === server;
  const canConfigure = settingsReady && draftSaved && (loopback || tls);

  const clearSecretInput = () => {
    if (secretInput.current) secretInput.current.value = "";
    setHasSecret(false);
  };

  useEffect(() => {
    clearSecretInput();
    setFeedback(null);
    // Changing the address draft must not carry an unsubmitted secret to another Server.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverDraft]);

  useEffect(() => {
    clearSecretInput();
    setFeedback(null);
    setConfiguredScheme(null);
    setStatusFailed(false);
    setScheme("Bearer");
    if (!settingsReady || (!loopback && !tls)) {
      setStatusLoading(false);
      return undefined;
    }

    let active = true;
    setStatusLoading(true);
    void invoke<string | null>("credential_status", { server })
      .then((savedScheme) => {
        if (!active) return;
        if (savedScheme !== null && savedScheme !== "Bearer" && savedScheme !== "ApiKey") {
          setStatusFailed(true);
          setFeedback({ tone: "error", message: localMessage("The saved credential has an unsupported scheme.") });
          return;
        }
        setConfiguredScheme(savedScheme);
        setStatusFailed(false);
        if (savedScheme) setScheme(savedScheme);
      })
      .catch((caught) => {
        if (active) {
          setStatusFailed(true);
          setFeedback({ tone: "error", message: { raw: errorMessage(caught, tRef.current) } });
        }
      })
      .finally(() => { if (active) setStatusLoading(false); });
    return () => { active = false; };
    // The status lookup follows only the saved Server, not edits to its draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server, settingsReady, loopback, tls]);

  const saveCredential = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    if (!canConfigure || statusLoading || busy) return;
    const secret = secretInput.current?.value ?? "";
    if (!secret) {
      setFeedback({ tone: "error", message: localMessage("Enter a secret first.") });
      return;
    }

    setBusy(true);
    setFeedback(null);
    try {
      await invoke<void>("save_credential", { server, scheme, secret });
      if (currentServer.current === server) {
        setConfiguredScheme(scheme);
        setStatusFailed(false);
        setFeedback({ tone: "success", message: localMessage("Credential saved in macOS Keychain.") });
        onCredentialChanged();
      }
    } catch (caught) {
      if (currentServer.current === server) setFeedback({ tone: "error", message: { raw: errorMessage(caught, t) } });
    } finally {
      clearSecretInput();
      setBusy(false);
    }
  };

  const clearCredential = async (): Promise<void> => {
    if (!canConfigure || statusLoading || busy || (!configuredScheme && !statusFailed)) return;
    setBusy(true);
    setFeedback(null);
    clearSecretInput();
    try {
      await invoke<void>("clear_credential", { server });
      if (currentServer.current === server) {
        setConfiguredScheme(null);
        setStatusFailed(false);
        setFeedback({ tone: "success", message: localMessage("Credential removed from macOS Keychain.") });
        onCredentialChanged();
      }
    } catch (caught) {
      if (currentServer.current === server) setFeedback({ tone: "error", message: { raw: errorMessage(caught, t) } });
    } finally {
      setBusy(false);
    }
  };

  const statusText = !loopback && !tls ? t("Loopback or TLS required") : statusLoading ? t("Checking…") : statusFailed ? t("Status unavailable") : configuredScheme ? t("{scheme} configured", { scheme: configuredScheme }) : t("Not configured");
  return <div className="settings-card credential-card">
    <div className="settings-card-heading"><div><span className="eyebrow">{t("Authentication")}</span><h2>{t("Server credential")}</h2></div><span className={`status-pill ${statusFailed ? "error" : configuredScheme ? "connected" : "disconnected"}`}>{statusText}</span></div>
    <p className="field-help credential-help">{t("For the saved Server ")}<strong>{server}</strong>{t(". The secret is stored in macOS Keychain and is never displayed after saving.")}</p>
    {!draftSaved && <p className="credential-guidance" role="status">{t("Save and test the Server address before configuring its credential.")}</p>}
    {!loopback && !tls && <p className="credential-guidance" role="status">{t("Credential storage and sending require a loopback address or an explicit tls:// address.")}</p>}
    <form onSubmit={(event) => void saveCredential(event)}>
      <label>{t("Scheme")}<select value={scheme} onChange={(event) => setScheme(event.target.value as CredentialScheme)} disabled={!canConfigure || statusLoading || busy}><option value="Bearer">Bearer</option><option value="ApiKey">ApiKey</option></select></label>
      <label>{t("Secret")}<input ref={secretInput} type="password" autoComplete="off" autoCapitalize="off" spellCheck={false} placeholder={t("Enter a new secret")} disabled={!canConfigure || statusLoading || busy} onChange={(event) => setHasSecret(Boolean(event.target.value))} /></label>
      {feedback && <p className={`credential-feedback ${feedback.tone}`} role={feedback.tone === "error" ? "alert" : "status"}>{renderMessage(feedback.message, t)}</p>}
      <div className="credential-actions"><button className="button primary" type="submit" disabled={!canConfigure || statusLoading || busy || !hasSecret}>{busy ? t("Working…") : t("Save credential")}</button><button className="button" type="button" onClick={() => void clearCredential()} disabled={!canConfigure || statusLoading || busy || (!configuredScheme && !statusFailed)}>{t("Clear credential")}</button></div>
    </form>
  </div>;
}

function SettingsDetail({ title, icon, onBack, children }: { title: string; icon: IconName; onBack: () => void; children: ReactNode }) {
  const t = useTranslation();
  const backButton = useRef<HTMLButtonElement>(null);
  useEffect(() => { backButton.current?.focus(); }, []);
  return <div className="settings-page"><button ref={backButton} className="back-button" onClick={onBack}>{t("‹ Settings")}</button><div className="settings-detail-heading"><span className="settings-tile-icon"><Icon name={icon} size={27} /></span><div><span className="eyebrow">{t("Settings")}</span><h1>{title}</h1></div></div>{children}</div>;
}
