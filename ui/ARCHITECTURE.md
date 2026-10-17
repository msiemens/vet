# UI architecture

Svelte 5 + CodeMirror 6, organised by feature under `src/`. What Vet does is in `SPEC.md`, which leaves out how the window looks; this file is how the window is built.

## What must always hold

A rule that carries out an invariant of `SPEC.md` §2 names it; the others are architecture rules, the first of them shared with `ARCHITECTURE.md`.

- **The window is a thin client:** it renders and sends, and review state, completion and names are the backend's. Components get data from props or stores, only `state/backend.ts` talks to the connection, and views listen to the stores, not the connection.
- **The window holds only what the backend sent.** A store holds and distributes it and applies its events; what it derives is presentation. An answer or event is applied only if it is newer than what is held, and an action in a view being replaced never lands in the next one.
- **A Find in Files search runs through `SearchRun`** (`state/searchRun.ts`), one per dialog and per Find set; nothing else calls `backend.search`.
- **The window asks content-anchored questions** (I2): a position and a view, never a commit hash. An operation a server does not offer is shown as not offered, never as nothing (I3); caching (I1) and trust are the backend's.
- **Git is read-only**: every editor is read-only and draws the backend's diff.
- **The dependency direction is one-way**: `features` import `lib/*` and `state`; `lib/*` imports nothing from `features`, `state` or `app`; a feature imports another only through its `index.ts`; `app` composes through snippets and props. One component per file; no `utils` or `helpers`. `eslint.config.js` enforces all of it, plus import cycles and the editor's no-Svelte rule (`pnpm lint`).

## Building blocks

`src/` has five groups.

- `generated/` — `ts-rs` output of `vet-protocol`; never edited by hand.
- `state/` — the connection (`connection.ts`), the typed backend client (`backend.ts`), the host's commands (`host.ts`), the session (`session.svelte.ts`) and the view-model stores. A store follows the events of what it shows and never relies on another store's request.
- `lib/` — domain-free primitives and shared logic: `lib/theme/` (the colour and dimension tokens; light and dark via `light-dark()`), `lib/ui/` (buttons, menus, popovers, dialogs, sidebars, splitters, panel tabs, scroll areas, tables, icons, virtual rows, tree rows, speed search …), the shared list rule, `lib/search/query.ts`, `lib/keymap.ts`, `lib/commands.ts`, `lib/commandMenu.ts`, `lib/speedSearch.ts`.
- `features/` — one folder per domain: its components, its own logic, and an `index.ts` as the public entry.
- `app/` — the window: `App.svelte` composes the features and shows the screen, dialogs and `ConnectionNotice`; `appState.svelte.ts` holds the dialog and the review controller, `commands.ts`/`actions.ts` the window's commands, `window.ts` the key listener and native-menu wiring, `appPreferences.svelte.ts` the theme and soft wrap.

Features, one line each:

- `welcome` — recent projects and Open… while no project is open; the project switcher.
- `chrome` — title bar and main-menu button, the header (screen toggle, switchers, base note, scope chip, progress, Finish review, checks, outgoing comments), banners, tool rails, status footer, About and Finish dialogs.
- `queue` — the project's home: the changeset table and the panel under it; the changeset switcher.
- `timeline` — the versions panel under the editors.
- `comments` — the Comments tool: the pull request block, the Review/Conversation tabs, threads and posting.
- `checks` — the head commit's checks in the review toolbar.
- `platform` — the platform footer item, notification cards and the bell.
- `settings` — the pages (General, Appearance, Keymap, Review, Language servers, Accounts) and the `Sheet` form.
- `symbols` — the Symbol tool: one symbol's signature, callers, callees, usages, reachability.
- `structure` — the Structure tool and File Structure popup: the file's outline.
- `locations` — the Go to Implementation / Show Usages picker.
- `palette` — Search Everywhere.
- `search` — Find in Files.
- `problems` — the Problems panel: the changeset's diagnostics, new ones first.
- `pipelines` — the Pipelines panel: runs, jobs, steps, re-run and cancel.
- `find` — the Find tool: the result sets of Find in Files and Find usages as tabs.
- `review` — the review screen: file tree, tab strip, panes, find bar, and `review/editor/`, the framework-agnostic CodeMirror layer with no Svelte imports.
- `splash` — the startup window.

### The host

The Tauri host (`ui/src-tauri/`) is the window's process. At launch it opens a splash and starts the backend on a thread, reporting each step; the main window loads hidden and `App` shows it once the connection is up, or a failed start keeps the splash with Retry. One window shows the welcome screen or a project and keeps a position and size per mode.

The host passes the repository to open (command line, `VET_REPO`, or `#repo=` in a browser), answers the folder and file dialogs and the recent list, and keeps the app version. It owns the operating-system keychain: the backend keeps tokens in memory only, and the window hands it every stored token when the connection is up and when it asks (`credentialsNeeded`). Personal access tokens are the host's; a web address (`openUrl`) is opened only when it is `http(s)`. While a comment box holds text, the page arms a close guard (`close.rs`): a window close or an exit without a code is held and handed to the page, which asks and then calls `quit`; a page load disarms it.

## How it runs

### Backend events into stores into views

1. Every event a request causes reaches every subscriber, the window that asked included: only the store that asked reads the answer, and every other store of that window learns of the change from the event alone.
2. A store follows the events of what it shows, so it never relies on another store's request; a reload that the requester's own answer makes redundant is harmless. The queue follows the changeset on view for as long as a project is open. Events sent while the connection was down are lost, and the stores catch up on resync.
3. The backend's own events (`credentialsNeeded`, `accountsChanged`, `recentProjectsChanged`, `settingsChanged`, `preferenceChanged`) reach every subscribed window; `onEvent` hands over only the open project's (`state/projectEvents.ts`), so no listener checks the project itself. `vet-backend/src/contract.rs` says which request sends which event, and its test checks it.

Each store follows the events of what it shows:

- `state/backend.ts` — `snapshotMoved`, `filterChanged`, `reviewFinished`, `changesetOpened`: aborts the semantic requests of older revisions.
- `opened.svelte.ts` — `changesetOpened`: ends the session on view and makes the announced changeset's (`openedProtocol.ts`); every other event goes to the session on view.
- `changesetSession.svelte.ts` — decides the reads for every event in one place: `sessionReads.ts` maps an event and the session's state to the reads it needs, and `readFlush.ts` runs them on the next task, once per burst. A pass awaits the manifest, then the whole changeset unless the manifest fed it, then starts threads, the pull request info, versions, changes since and commits without waiting, and has the file tabs' scopes read. What the event itself carries is applied at once: `lensChanged` the lens's view, `reviewChanged` one file or the totals in a commit, `reviewStarted` and `restoreFailed` the facts' review and failures. The reads per event:
  - `snapshotMoved`: the manifest, the whole changeset, threads, the scopes, and versions and changes since when it names a `version` (a review is open).
  - `filterChanged`: the manifest, the whole changeset, changes since, the scopes. No snapshot moved, so the threads held are carried to the new revision instead of read.
  - `reviewFinished`: the manifest, the whole changeset, threads, versions, changes since, the scopes.
  - `targetChanged`: the manifest, the whole changeset, the scopes.
  - `lensChanged`: the manifest, unless a range stays on view.
  - `threadsChanged`: threads and the pull request info.
  - `platformStatus`: the pull request info.
  - `reviewStarted`: versions, changes since.
  - `reviewChanged`: versions and changes since only when its `reviewedAt` says the mark moved where the review last was (the first seen-mark of a version), and the commits when they show its progress.
- `queue.svelte.ts` — `changesetOpened`, `reviewStarted`, `changesetsChanged`, `reviewFinished`: lists the local changesets again; `threadsChanged`: lists them without recounting; `pullRequestListChanged`: reads the pull requests again; `pullRequestsChanged`: patches the rows it names; `changesetLinesChanged`: the counts pushed so far; `snapshotMoved`, and `reviewChanged` with `reviewedAt`: follows the row on view; `filterChanged`: nothing, as a row counts lines between commits and the row on view shows its progress live.
- `versions.svelte.ts` (a session's versions, while the review screen is shown), `reviewChanges.svelte.ts` (a session's changes since a version, for its whole life, as the queue panel shows them), `pullRequest.svelte.ts` (only a pull request's session has one) and `threads.svelte.ts` (the changeset's threads and, while the review screen shows a range, the range's) — read when the session's flush says so, and when the session starts or resyncs.
- `pipelines.svelte.ts` (a session's chosen run, job and folds) — `accountsChanged`: forgets which accounts were refused; while the panel shows, a running chosen job's steps are read again for every new record of it the checks bring; the steps drop when the review screen hides.
- `checks.svelte.ts` (a session's checks) — `checksChanged`: takes the checks.
- `semantic.svelte.ts` (starts over for each session) — `fileDiagnostics`: a file's pushed diagnostics; `serversChanged`: drops the answers and tells the views.
- `symbols.svelte.ts` (a session's changed symbols and the symbol on show, while the review screen is shown) — `fileSymbols`: a file's pushed symbols and the progress; `serversChanged`: asks again about the symbol on show.
- `structure.svelte.ts` (a session's outline and selection, while the review screen is shown; sort, changed-only and the popup are the window's) — `serversChanged`, the session resyncing: reads the outline again.
- `pendingEdits.svelte.ts` — `editsPending`: whether edits wait to settle.
- `platform.svelte.ts` — `platformStatus`: the status, which a `PullRequestInfo` answer also sets; `notification`: the bell's list and its card.
- `progress.svelte.ts` — `task`: the running tasks, and the last failed sync and checks refresh until one of their kind succeeds or another changeset comes on view, and the last failed fetch of the remotes (`remoteFetch`) until one succeeds, across changesets.
- `serverList.svelte.ts` — `serversChanged`: reads the servers again; `serverRuntimes`: what the servers do and offer, without reading the list; `serverInstallOutput`: an install's output.
- `servedBy.svelte.ts` — `serversChanged`: which server serves each file.
- `ReviewController.ts` — `reviewChanged`: hands the update to the scope that made it. The tabs' scopes read again when the session's flush calls them, after the manifest.
- `accounts.svelte.ts` — `credentialsNeeded`, `accountsChanged`: hands over the keychain's token, and reads the accounts again.
- `recent.svelte.ts` — `recentProjectsChanged`: reloads the recent list.

#### Reactivity

Backend data lives in `state/` as immutable snapshots (`$state.raw`), one cell per file, never deep `$state`: proxies over thousands of files cost every read. Each opened changeset is a `ChangesetSession` held by `opened`: it lives until another changeset is opened or the project closes, and every continuation checks its `Lifetime`, so a late answer touches nothing in the next one. A session keeps apart what a manifest says (`layout`) from the changeset's `facts` and a cell per path holding its review state, and holds the whole changeset (`whole`) and its commits by revision (`CommitList`), its versions, its changes since a version, the Pipelines panel's choice, its changed symbols and outline, its checks and, for a pull request, what the platform says of it, the Find tool's result sets, Find in Files' place and chosen result, and the text typed into comment boxes and not added (`commentDrafts.ts`); what is read only for the review screen is read while it is shown (`whileReviewShown`). The session holds one view choice (range, commit, base-changed limit); `viewChoice.ts` decides which view is asked and how a manifest read settles on one (a gone range or commit gives way to the changeset, at most three re-asks), and only the latest read may correct the choice. The generated manifest shapes are read only by the session, the whole view and the client (an eslint rule). Requests about the project rather than a changeset are plain functions in `projectRequests.ts`. Ending a session loses its comment text, so switching changesets, closing or opening a project and quitting go through `whenDiscarded` (`discard.ts`), which asks with one dialog while a box holds text.

There are no full rebuilds: a keyed or incremental `$derived`, so one change invalidates only the cell, tree row and tab that show it, and the file tree is rebuilt once per revision. The editor and virtualized views are not reactive: they get explicit calls, subscribe to no store, and hold no review state (an eslint rule). Review state reaches them by notify-then-pull through the view's review cells, and each section redraws when the version it remembers differs, so a missed notification is corrected at the next redraw. Nothing reactive runs per frame; small UI state is plain `$state`.

#### The review screen

`App` shows the queue or the review under the header; the review is mounted only while it shows, and comes back from the per-changeset views it saved (open tabs, active tab, scroll positions, current file); its first frame comes from the changeset store. A queue action that needs the review leaves an intent it takes once it is up (`appState.whenReview`). Once it is shown, the Changes tab loads the files in view and the margin around them, and any file a command goes to; the rest loads a few at a time while the screen is idle, pausing while the viewer scrolls or acts. The base note appears only for a pull request into a non-default branch or a `HEAD` off its base, and the scope chip names the view on show.

- `ReviewScreen`'s shell (sidebars, dock, tool panels) belongs to the window; its `ReviewController`, the tabs and the panes belong to one session and are made again when another changeset is opened. What a changeset had open and where comes back from `reviewMemory`, which outlives the controllers. The session tells the controller what changed (`onViewChange`): a newer revision keeps the Changes tab's place, another range, commit or filter starts it at the top.
- A pane hands its scroll root to `ReviewController`, which builds a view for the tab; the view is not torn down when the state it reads changes, only remade by a new manifest.
- Heights come from the manifest, not from CodeMirror, which estimates off-screen sections; layout reads happen in CodeMirror's measure phase, never on the mount path. Wrapping is the viewer's setting, and only prose wraps by default.
- The UI computes no diff and parses no code: the backend's changed ranges go to `@codemirror/merge`, and highlights come with the file contents. Lists render through `VirtualRows`.
- A file has one tab per file and view: one on the whole changeset, or one pinned to a commit, which stays at that commit and offers no comments. A tab reads its own `ViewSource` (the session, its whole-changeset view, or a `ViewScope` created for its commit's tabs), each a `HeldView`, which is how they share the review cells and mark requests; an action names its tab by id and its view by origin, never by path and commit.
- A range of versions is a mode of the review: the View every file request carries; it never grows and is read-only unless it ends at the latest. By commit, the `manifest.lens` names one commit and `view` is that commit's view. Hunks seen against an older base draw amber and count as unseen until re-confirmed.
- A session holds the threads per view: the changeset's always, a range's while the review screen shows it, each at its revision. A view draws the threads of its own view only when their revision is its own; a commit's tab draws none; the queue panel lists the changeset's. A pull request's threads carry the backend's sync state, including a two-sided edit the card shows as two texts and a Keep choice.
- Posting has one code path for the tool window and the header button, following the platform's traits.
- The find bar searches the tab's own diff or text, never an editor, since an unmounted section has none.
- Hover, go-to-definition and diagnostics reach the editor bound to its revision; hover text is untrusted and rendered with HTML escaped and no links, and an unavailable answer (I3) is never shown as nothing.

### A user action out to the backend

1. A key, a menu or a tooltip names a command in one registry (`state/commands.ts`): an id, a label, a default binding (data, so the Keymap page overrides it), an enabled-when over the context, and what it runs. App-wide commands are in `state/appCommands.ts`, the review's in `features/review/reviewCommands.ts`, registered by `app/commands.ts`; tool windows are listed once in `TOOL_WINDOWS` with `view.tool.<id>`.
2. `App` installs the capturing `keydown` listener that every command goes through. Text input never matches; a command that cannot run leaves its key to the page. Menus are command-id descriptions resolved by `menuFor`; macOS draws them natively, elsewhere the title bar does.
3. A request that can change review state carries `Shown` (the revision and view it was made for); the view source's `admit` refuses a stale one, so an action in a view being replaced never lands in the next. A failure the viewer should hear about goes to `notices` (`notices.svelte.ts`, a Toast).
4. Controls take `.flat` or `.ctl` and use tokens for colours and sizes; the editor reads its row heights from the tokens. Lists share one rule: a click runs the row's single action, a double click its open action, arrows move the highlight, Space acts and Enter opens, and `ClickGate` holds a click for the system's double-click time. Chrome is not selectable; content opts in.

### Reconnect and resync

1. The host supervises the backend and starts another on a new port if it exits, unless that would be its fourth exit within a minute.
2. `state/connection.ts` reconnects on a drop: requests on the dropped socket fail and are not resent, a request made while reconnecting waits up to 10 s, retries are 0.2/0.5/1/2 s then every 5 s, and the link is `stalled` after 30 s. A window that stops reading its outbox is disconnected, and reconnects the same way.
3. On reconnect the session opens the repository again, subscribes, reloads the recent list and calls the `onResync` listeners: every store that holds backend data reads it again, and the backend re-runs the symbols, problems and checks analyses once the review screen is shown again.

## Shared concerns

### Reactivity and rendering budgets

The rules above are also performance rules, and `ui/test/harness/` checks the stores against the backend: scripted sessions act against a real backend, and after each step the stores are compared with fresh reads through a second connection. Known differences are listed by bug; any other, or a listed one that does not occur, fails. A second tier checks what the editor views drew against the backend.

### The connection and the protocol

One protocol over one loopback WebSocket; the messages are `vet-protocol`'s request, response and event shapes, whose bindings the window reads from `ui/src/generated`. The outbox bound and `WaitForQuiet` are as in `ARCHITECTURE.md`. Because nothing is replayed, the resync path must be complete.

### Storage and preferences

The window keeps its own state in the browser storage. App-wide: theme, soft wrap, key bindings, panel and dialog sizes, table columns and sort, folded sections, the queue's and the switcher's tab, Search Everywhere's options, the structure tool's sort and filter, and when each pull request's Comments tool was last open. Per project: its open tabs per changeset and the results chosen last in Search Everywhere. The backend's own settings (semantic cache budget, finished-review retention) are read through `backendSettings.svelte.ts` without an open repository. Secrets are the host's keychain.

### Errors and notices

A failed request answers with an `ErrorKind` and a message fit to show; the window decides from the kind and shows the message, never reading it to tell kinds apart. A failure the viewer should know about becomes a notice or a `Notification` card. Uncaught errors, unhandled rejections and failed requests go to the backend as `ClientMessage::Log`.

### Logging

The window logs through the backend: `ClientMessage::Log` under the target `ui`. `LogDirectory` points at the files for Help > Show Log.

### Browser development

`pnpm dev` serves the UI with no backend; the page talks to a real `vet-backend` started by hand, addressed by `#port=&token=&repo=` in the URL, and the dialer throws when `port` or `token` is missing. A plain browser is development only: the host's commands fall back or are off, and the data directory is the backend's own.

### Theming

Light and dark come from `lib/theme/`. A colour or size belongs in `tokens.css`, not in a component.

## Where new code goes

- New screen: `features/<name>/` with an `index.ts`, mounted from `app/App.svelte`.
- New backend request: a method in `state/backend.ts`, then a store field if the UI displays it; a request that can fail in a way the viewer should hear about goes through the store, which tells `notices`.
- Data the protocol does not carry: an optional prop whose absence renders a neutral empty state.
- New sidebar tool: an entry in `TOOL_WINDOWS` (`state/appCommands.ts`), an icon in `chrome/ToolRail`, and its content in `ReviewScreen` or `App`. A bottom tool's panel is a `<section aria-label>` with its `TOOL_WINDOWS` label, which `chrome/focus.ts` focuses (its `[data-list]` first).
- New settings page: a component in `features/settings/` and an entry in `SettingsDialog`'s page list.
- New changeset kind in the queue: the backend's `ChangesetKey` variant and its tab in `queueRows.categoryOf`.
- New tab kind: an id in `Tabs.svelte.ts`, a pane in `ReviewScreen`, and a case in `ReviewViews.build`.
- Anything that differs by platform: a trait the backend sends in `PlatformTraits`, read in the component; never the platform's name.
- A new kind of Search Everywhere result: a candidate in `palette/paletteRows.ts`, its source in `paletteSources.ts`, and its case in `openChoice.ts`.
- New action with a key or a menu entry: a command registered with `commands`, named by id in the menu descriptions that offer it.
