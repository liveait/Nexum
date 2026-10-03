# Nexum macOS Desktop UI Design

Status: implemented macOS Desktop UI baseline; remaining items are called out as planned.

This document defines the macOS-first information architecture and interaction model for the Tauri desktop client. The current `apps/desktop/src/App.tsx` and `App.css` implement the sidebar, Downloads surface, Inspector, Settings cards with macOS Keychain credential controls, Add Download sheet with a native destination selector, status bar, event-driven refresh, keyboard interaction, accessibility labels, focus management, reduced-motion behavior, system light/dark palettes, English/Simplified Chinese UI strings, and planned-capability placeholders described here.

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
- Use the system font stack, Nexum's purple accent, light/dark system appearance, and visible keyboard focus rings. A system accent-color option remains planned.

### 2.1.1 Appearance and language implementation decision

- The Desktop shell follows macOS light and dark appearance through `prefers-color-scheme`, with two readable palettes and the existing reduced-motion behavior. A manual theme or accent-color selector remains planned.
- Language defaults to **System**, resolving a Chinese system language to Simplified Chinese and other languages to English. Settings can override it with **English** or **简体中文**; the choice is persisted alongside the Server address and refresh policy, with older settings defaulting to **System**.
- A central typed message catalog supplies visible UI text, tooltips, accessible names, task-state labels, validation, and client-generated status messages. The language choice changes presentation only: protocol values and Server requests stay unchanged. Raw diagnostics returned by the Server or operating system remain verbatim so technical details are not lost.

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

Actions are disabled per row while their request is in flight. Keyboard shortcuts are available for common flows; native context menus remain planned.

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
2. Destination field with a native save dialog provided by the Tauri dialog plugin.
3. Explicit Task ID field required by the current Server contract.
4. Advanced disclosure for future headers, priority, and bandwidth policy; hidden until those server features exist.
5. Cancel and Add Download buttons.

Submit creates the task and immediately queues it. The sheet closes only after both RPC calls succeed. Retrying queueing without creating a duplicate after a partial failure remains planned.

The current server requires an explicit task ID and destination, so the first implementation keeps those fields visible. A later protocol can make the ID optional without changing the layout.

## 5. Settings

Settings is a dedicated page with grouped sections. It is not a second task workflow:

- **Server:** address (default `127.0.0.1:39100`), Connect/Test, last connection result, and protocol version.
- **Authentication:** in General settings for the active saved Server, show a `Bearer`/`ApiKey` scheme selector, write-only secret field, configured-scheme status, and Save/Clear actions. Save the scheme and secret together in macOS Keychain under that Server address; never read the secret back into React. Saving, clearing, or changing the active Server restarts the event subscription and refreshes the task snapshot. Enable credential storage for loopback plaintext addresses and explicit `tls://` addresses. Plaintext credentials remain loopback-only; the TLS client validates the server name and system root chain before sending a credential.
- **Updates:** Automatic uses event refreshes and a five-second idle or one-second active polling fallback when the stream is disconnected; Manual disables that polling fallback.
- **Appearance:** the shell follows the system light/dark appearance. A System/English/Simplified Chinese language selector saves immediately and persists with the Server address and refresh policy. Manual theme and accent-color choices remain planned.
- **Notifications:** show task completion, failure, and retry events in the session Activity Center; reserve delivery preferences and mute controls for a later slice.
- A note that the Server is a separate process in the first release.

Persist the address, refresh policy, and language preference in the macOS application support directory through Tauri commands. Keep credentials only in macOS Keychain. The Tauri backend attaches the credential to ordinary RPC calls and the dedicated `events.subscribe` request, and reports Keychain failures without revealing the secret. Desktop on other platforms can connect without authentication, but credential Save and Clear are unsupported.

## 6. State model and data freshness

The React layer should consume a small client state model:

```text
ConnectionState = disconnected | connecting | connected | error
TaskData = { items, selectedId, lastUpdatedAt }
OperationState = { [taskId]: idle | running }
```

The Desktop opens a dedicated `events.subscribe` TCP connection through a Tauri background thread. The Server sends `events.event` JSON-RPC notifications with incremental Task or Scheduler data, plus a heartbeat every 15 seconds to keep the connection alive. Tauri ignores heartbeat updates, tracks the sequence within each connection, and treats duplicate or stale notifications as harmless. A forward sequence gap closes the stream so reconnect logic can trigger a full snapshot; React debounces task and scheduler notifications and refreshes the full task snapshot because an update does not contain a complete `TaskView`.

While the event stream is connected, use event-driven refreshes:

- Show a live-updates status when the subscription is active.
- Refresh after a short debounce so progress bursts do not create overlapping `task.list` calls.
- Keep manual Refresh as a recovery action.

When the stream is disconnected, use adaptive polling as a fallback:

- 1 second while any task is Downloading or a request is active.
- 5 seconds while connected and idle.
- Stop polling when the window is hidden.
- Keep `lastUpdatedAt` and show “Updated just now / X ago” in the toolbar.

The stream has no replay buffer. A reconnect, dropped subscription, or detected sequence gap starts with a full task snapshot, and the UI continues to keep the last successful snapshot when a refresh fails.

The RPC boundary stays in Rust/Tauri commands. React owns presentation state and never opens a TCP socket or interprets JSON-RPC errors directly.

## 7. macOS interaction and accessibility

- `⌘N` opens Add Download; `⌘F` opens and focuses Downloads search. Escape closes the open sheet or search field. Sidebar and action controls remain in the normal Tab order.
- Task selection is a button: Enter or Space selects it, Up/Down moves between visible tasks, and Home/End moves to the first or last visible task. Row actions are separate buttons.
- The Add sheet focuses Source URL, keeps Tab and Shift+Tab inside the dialog, and returns focus to the invoking control when closed. The native destination picker returns focus to Choose after it closes.
- Navigation uses `aria-current`; task selection announces task state and progress, Inspector progress has progress-bar semantics, and task actions have accessible names. Error and operation messages use alert/status semantics without announcing every progress tick.
- CSS respects `prefers-reduced-motion` for transitions, hover movement, and indeterminate progress animation, and `prefers-color-scheme` for light/dark palettes.
- A typed English/Simplified Chinese message catalog covers Desktop-owned visible and accessibility-facing strings. Native context menus for task actions and destructive confirmation wording remain planned.

## 8. Implementation slices

### Slice A — shell and reliable state (implemented)

- Replace top tabs with sidebar, toolbar, list, and inspector. [x]
- Introduce typed task/connection/operation state in React. [x]
- Fix per-action loading and preserve the last successful list on errors. [x]
- Keep current TCP RPC commands unchanged. [x]

### Slice B — Add sheet and native settings (partially implemented)

- Add URL validation and task ID derivation. [ ]
- Add the native destination save dialog through the Tauri dialog plugin. [x]
- Add Tauri commands for loading and saving desktop settings. [x]
- Add per-Server macOS Keychain credential Save/Clear controls and attach credentials to RPC and event subscriptions. [x]
- Queue a newly created task as part of the Add flow. [x]
- Retry queueing after a partial Add failure without creating a duplicate task. [ ]

### Slice C — live task surface (partially implemented)

- Add adaptive polling and refresh timestamps. [x]
- Add progress bars, state badges, and inspector details. [x]
- Add the dedicated Server event subscription, Tauri event bridge, and debounced task refresh. [x]
- Add explicit stale-data indicators for retained snapshots after request errors. [ ]
- Add keyboard navigation, shortcuts, focus management, and accessibility labels. [x]
- Respect reduced-motion preferences for desktop transitions. [x]
- Add native context menus for task actions. [ ]

### Slice D — macOS release polish

- Follow the system light/dark appearance and add English/Simplified Chinese UI strings. [x]
- Add light/dark screenshots and visual regression checks.
- Build the frontend and Tauri app on macOS.
- Produce a signed/notarized `.app`/`.dmg` when release credentials and identity are available.

## 9. Acceptance criteria

The current implementation satisfies these baseline behaviors:

- A user can identify connection state and task state without opening a detail view.
- Adding a download requires no manual JSON-RPC or CLI step.
- Active progress updates without pressing Refresh when the event stream is connected, with polling fallback when it is not.
- A failed request does not erase the last useful task list.
- Pause, resume, and remove actions are available only in valid states and report their result.
- Restarting the app preserves the Server address and refresh policy.
- The UI never claims Magnet/local-file support before the transfer engines exist.
- The same flows render with system light/dark palettes, English/Simplified Chinese UI strings, keyboard navigation, accessible controls, managed focus, and reduced-motion behavior. Native macOS visual and VoiceOver release checks remain pending.
