# Nexum macOS Desktop UI Design

Status: implemented macOS Desktop UI baseline; remaining items are called out as planned.

This document defines the macOS-first information architecture and interaction model for the Tauri desktop client. The current `apps/desktop/src/App.tsx` and `App.css` implement the sidebar, Downloads surface, Inspector, Settings cards, Add Download sheet, status bar, and planned-capability placeholders described here. Native destination selection, server events, keyboard shortcuts, and localization remain later slices.

For the Chinese version, see [DESKTOP_UI_DESIGN.zh-CN.md](DESKTOP_UI_DESIGN.zh-CN.md).

## 1. Product goal

The desktop app should make the common local workflow understandable at a glance:

1. Connect to a Nexum Server.
2. Add an HTTP/HTTPS download.
3. Queue it and see progress without manually refreshing.
4. Pause, resume, retry, or remove it safely.
5. Inspect the destination and the latest error when a task needs attention.

The first desktop release is a client for a separately running Server. It does not start a Server process automatically and it does not present unsupported Magnet or local-file sources as if they were ready.

## 2. Design decisions

### 2.1 NavigationSplitView-style shell

Use a macOS-style sidebar and a single main task surface rather than tabs across the top.

```text
┌────────────────────────────────────────────────────────────────┐
│ Sidebar       │ Downloads: filter  search  Inspector           │
│ Dashboard     ├────────────────────────────────────────────────┤
│ Downloads     │ Task list                         Inspector     │
│ Trackers      │ name | status | progress | destination          │
│ Plugins       │                                                │
│               │                                                │
│ Notifications │                                                │
│ Settings      │                                                │
└───────────────┴────────────────────────────────────────────────┘
```

- Minimum window: 960×640; preferred window: 1120×720.
- Sidebar width: 208–240 px and collapsible.
- Main content has a list-first layout. A selected task opens an inspector on the right at widths above 1100 px; below that width it opens as a sheet or stacked detail panel.
- Use the system font stack, system accent color, light/dark system appearance, and native focus rings.

### 2.2 Sidebar sections

The sidebar follows the Motrix-style top-level navigation. Task filters stay inside `Downloads`; Server connection details belong inside Settings:

| Section | Purpose | Data source |
| --- | --- | --- |
| Dashboard | Overview and recent activity | `task.list` snapshot |
| Downloads | All, Active, Queued, Completed, and Failed filters | client-side filter over `task.list` |
| Trackers | Reserved for BitTorrent tracker health | planned engine capability |
| Plugins | Reserved for plugin catalog and runtime | planned plugin capability |
| Notifications | Session task and connection events | Desktop session state |
| Settings | Server connection, appearance, refresh, and future client options | Desktop settings |

Each filter shows a count when the data is fresh. Counts are hidden while the connection is unavailable so stale values are not presented as current.

### 2.3 Toolbar

The Downloads header has a filter selector and compact controls:

- **Add**: opens the Add Download sheet from the sidebar or floating action button.
- **Filter**: selects All, Active, Queued, Completed, or Failed downloads.
- **Inspector**: toggles the right-hand task details panel.
- **Search**: filters task ID, source, destination, and state locally.
- Row actions pause, resume, queue, start, or remove a task when its state permits it.
- The footer shows HTTP/HTTPS, connection state, and the latest refresh time.

Actions are disabled per row while their request is in flight. Keyboard shortcuts and native context menus remain planned.

## 3. Task list and inspector

### 3.1 Row content

Every row shows only values supplied by the server:

- Task name: task ID for now; derive a filename label later when the protocol exposes one.
- State badge: Created, Queued, Downloading, Paused, Completed, or Failed.
- Progress bar: determinate when `total_bytes` is known, indeterminate otherwise.
- Byte text: `downloaded_bytes / total_bytes` with a human-readable unit.
- Destination path, truncated with a tooltip.
- Latest error beneath the row when `error` is present.

Speed, ETA, retry count, and validator are not shown until the protocol exposes those values. The UI must not estimate them from sparse polling data and imply precision that the server does not provide.

### 3.2 Inspector

The current Inspector presents the selected task in this order:

1. State and progress.
2. Source URL.
3. Destination path.
4. Server address.
5. Last error with a copy action.

The Inspector is read-only for task metadata. Row actions currently provide the task commands; native reveal and destructive confirmation are planned.

### 3.3 Empty, loading, and error states

- **No connection:** keep the task surface available, show the unavailable state in the footer/banner, and direct the user to Settings for the Server address.
- **Connected with no tasks:** show Add Download as the primary action.
- **Loading:** keep the existing list visible and show a small progress indicator in the toolbar.
- **Request failed:** keep the last successful list, mark it stale, and show a recoverable error banner with Retry. Do not replace a useful list with an empty state after a transient error.
- **Unsupported source:** validate HTTP/HTTPS before submission and explain that Magnet/local-file transfers are not available yet.

## 4. Add Download sheet

The sheet has one focused flow:

1. Source URL field.
2. Destination field (a native folder/file chooser is planned).
3. Explicit Task ID field required by the current Server contract.
4. Advanced disclosure for future headers, priority, and bandwidth policy; hidden until those server features exist.
5. Cancel and Add Download buttons.

Submit creates the task and immediately queues it. The sheet closes only after both RPC calls succeed. If creation succeeds but queueing fails, keep the form open and identify the task so the user can retry queueing without creating a duplicate.

The current server requires an explicit task ID and destination, so the first implementation keeps those fields visible. A later protocol can make the ID optional without changing the layout.

## 5. Settings

Settings is a dedicated page with grouped sections. It is not a second task workflow:

- **Server:** address (default `127.0.0.1:39100`), Connect/Test, last connection result, and protocol version.
- **Updates:** refresh policy (Automatic, Manual, or a configured interval while polling is enabled).
- **Appearance:** follow the system appearance; reserve language and accent choices for the client settings model.
- **Notifications:** reserve this section for task completion and failure notifications after the notification contract is defined.
- A note that the Server is a separate process in the first release.

Persist the address and refresh policy in the macOS application support directory through a Tauri command. Do not rely only on React state or browser storage. Credentials should not be displayed in this page until server-side authentication is enforced.

## 6. State model and data freshness

The React layer should consume a small client state model:

```text
ConnectionState = disconnected | connecting | connected | error
TaskData = { items, selectedId, lastUpdatedAt }
OperationState = { [taskId]: idle | running }
```

Until the server publishes events, use adaptive polling:

- 1 second while any task is Downloading or a request is active.
- 5 seconds while connected and idle.
- Stop polling when the window is hidden or disconnected.
- Keep `lastUpdatedAt` and show “Updated just now / X ago” in the toolbar.

When a server event stream exists, replace active polling with event updates and retain manual Refresh as a recovery action.

The RPC boundary stays in Rust/Tauri commands. React owns presentation state and never opens a TCP socket or interprets JSON-RPC errors directly.

## 7. macOS interaction and accessibility

- Support keyboard navigation through the sidebar, task list, inspector, and sheets.
- Provide accessible labels for state badges, progress, and destructive buttons.
- Use `aria-live="polite"` for connection and operation results; do not announce every progress tick.
- Respect reduced motion and system color scheme.
- Use context menus for task actions in addition to toolbar buttons.
- Keep destructive confirmation wording specific: identify the task and say that an existing destination is preserved by the server.
- Add English and Simplified Chinese strings through a small typed message catalog before the final UI is shipped.

## 8. Implementation slices

### Slice A — shell and reliable state (implemented)

- Replace top tabs with sidebar, toolbar, list, and inspector. [x]
- Introduce typed task/connection/operation state in React. [x]
- Fix per-action loading and preserve the last successful list on errors. [x]
- Keep current TCP RPC commands unchanged. [x]

### Slice B — Add sheet and native settings (partially implemented)

- Add URL validation, task ID derivation, and native destination chooser. [ ]
- Add Tauri commands for loading and saving desktop settings. [x]
- Queue a newly created task as part of the Add flow. [x]

### Slice C — live task surface (partially implemented)

- Add adaptive polling and refresh timestamps. [x]
- Add progress bars, state badges, and inspector details. [x]
- Add stale-data indicators, keyboard shortcuts, accessibility labels, and context menus. [ ]

### Slice D — macOS release polish

- Add light/dark screenshots and visual regression checks.
- Build the frontend and Tauri app on macOS.
- Produce a signed/notarized `.app`/`.dmg` when release credentials and identity are available.

## 9. Acceptance criteria

The current implementation satisfies these baseline behaviors:

- A user can identify connection state and task state without opening a detail view.
- Adding a download requires no manual JSON-RPC or CLI step.
- Active progress updates without pressing Refresh.
- A failed request does not erase the last useful task list.
- Pause, resume, and remove actions are available only in valid states and report their result.
- Restarting the app preserves the Server address and refresh policy.
- The UI never claims Magnet/local-file support before the transfer engines exist.
- The same flows render in the dark Motrix-style shell; keyboard navigation, system appearance, and localization remain release polish.
