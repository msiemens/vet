# UI architecture

Svelte 5 + CodeMirror 6, organised by feature under `src/`.

| Folder                    | Holds                                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/`                    | `App.svelte`: composition only. Wires features together, applies the theme, owns which tool of each rail is selected and whether its sidebar is shown.                                                                                                                                                                                                                      |
| `lib/theme/`              | `tokens.css` (the only place colours and dimensions are defined, light and dark via `light-dark()`), `global.css` (control states, text selection), `theme.ts`.                                                                                                                                                                                                             |
| `lib/ui/`                 | Domain-free primitives: Icon, IconButton, Chip, Meter, DiffBar (a file's added/removed ratio, turning blue as it is reviewed), Popover, Menu (with submenus), SplitButton, SideBar, ContextMenu, Tab, Toast, Banner (a dismissible notice under the header), Dialog (modal sheet), Segmented. Check here before adding one. `lib/keymap.ts` matches key events to bindings. |
| `features/<domain>/`      | One folder per domain: components, its own logic, and an `index.ts` as the public entry.                                                                                                                                                                                                                                                                                    |
| `features/review/`        | The review screen: file tree, tab strip and panes, `ReviewController` (builds a view per tab), `Tabs.svelte.ts` (open tabs), `tree.ts` (tree rows).                                                                                                                                                                                                                         |
| `features/review/editor/` | CodeMirror integration: framework-agnostic TS, no Svelte imports. `ChangesetView` (a column of file sections, virtualized), `mount` (one file's editor), `sectionLayout` (section heights from the manifest), hunk bars and review decorations.                                                                                                                             |
| `state/`                  | `connection.ts` (transport), `backend.ts` (typed API over it), `host.ts` (the Tauri host's folder dialog, recent-repository and menu commands), `mainMenu.ts` (the menu definition) and view-model stores (`changeset`, `recent`, `mainMenu`). No product logic (SPEC I5): review state and completion are rendered as the backend sent them.                               |
| `dev/`                    | Spike benchmark, metrics, HUD and a change-card gallery on fixtures, behind the "Dev" toggle in the footer.                                                                                                                                                                                                                                                                 |

`generated/` (ts-rs output of `vet-protocol`) stays at the top level; never edit it by hand.

## Rules

- `features` import `lib/*` and `state`; `lib/*` imports nothing from `features`, `state` or `app`.
- A feature imports another only through its `index.ts`. `app` composes features through snippets and props.
- Components get data from props or stores. Only `state/backend.ts` talks to the connection.
- Components use tokens (`var(--seen)`, `var(--h-row)`), never literal colours or sizes. The editor reads row heights from the tokens at construction.
- `eslint.config.js` enforces the import rules and the editor's no-Svelte rule (`pnpm lint`), plus import cycles.
- Events: `state/changeset.svelte.ts` subscribes to the backend's events (refetch on `snapshotSettled`, update one file on `reviewChanged`, set `manifest.review` on `reviewStarted`); views listen to the store, not the connection.
- One component per file, named by what it is. No `utils`/`helpers`; put a function next to its user, or in `lib/` if shared.

## Reactivity

- Backend data lives in `state/` as immutable snapshots (`$state.raw`), one reactive cell per file (`SvelteMap<path, FileChange>`), never deep `$state`: proxies over thousands of files and hunks cost memory and every read. `changeset.manifest` is the manifest of the last refresh with the changeset completion kept current; `changeset.file(path)` is a file's current state.
- No full rebuilds. A `$derived` over a large collection is keyed or incremental: one `ReviewChanged` or `SetSeen` reply invalidates only that file's cell, and with it the tree row and tab that show it. The file tree's structure depends on `changeset.paths`, which keeps its identity while only review state changes, so it is built once per revision; a row reads its own file.
- The editor and virtualized views get explicit calls (`applyReview(path, review)`, `goToHunk`, `setMode`); they never subscribe to stores, and editor state is never mirrored into Svelte state.
- Nothing reactive per frame: no `$effect` or reactive state on what changes with a scroll position, a pointer move or a keystroke. Mounting editors while scrolling is imperative, and spends at most a few milliseconds per frame (the rest follows on the next frames).
- Small UI state is plain `$state`: tool windows, widths, theme, tabs, menus, toasts.

## Controls and selection

Controls behave like a desktop app (JetBrains-like): `cursor: default`, no transitions or shadows, a flat background on hover and while pressed, a distinct toggled state, and a focus ring only for keyboard focus (`:focus-visible`). The colours are tokens (`--hov`, `--press`, `--tog`, `--tog-hov`, `--tog-press`, `--focus`); `global.css` applies them to two classes:

- `.flat`: borderless (icon buttons, rows, menu items).
- `.ctl`: bordered (chips, text buttons). `.ctl-group` joins several; the right part owns the one border between parts.

Toggled is `aria-pressed="true"` or `.on`. A new control takes one of the classes; it does not restyle hover or pressed itself. Editor widgets (hunk bars) use the same classes.

Chrome is not selectable (`user-select: none` on `body`). Content meant to be copied opts in with `.selectable`: CodeMirror editors and their tooltips, comment bodies (when they exist), the Symbol panel's code and signature, the dev results table. Paths are not selectable text; tree rows and file headers have a "Copy path" context menu.

## Main menu and settings

`state/mainMenu.ts` defines the menu once (`buildMenu`: items with id, label, shortcut, enabled, checked, children), from the current state: recent repositories, view mode, theme, which tool windows are shown. `App` feeds it through `mainMenu.update` and executes the commands (`parseCommand`) the menu reports; a command is one of open, openRecent, settings, about, quit, mode, tool, theme, devPanel. Where it is drawn depends on the platform:

- macOS (Tauri): the native menu bar. `state/mainMenu.svelte.ts` sends the definition to the host's `set_menu` command, which builds it with Tauri's menu API (and adds an Edit menu, without which the webview gets no ⌘C/⌘V). A chosen item comes back as the `vet-menu` event carrying its id, forwarded by `mainMenu.run`. Settings and About move into the app menu there.
- Windows, Linux and a plain browser: a ☰ `IconButton` at the left of the `TitleBar` opens the same items with `Menu`. Alt pressed and released on its own focuses it; the arrow keys then navigate.

Settings (`features/settings`) is a `Dialog`: Theme first, persisted by `lib/theme/theme.ts`. Keys: Ctrl+Alt+S opens it (App), Space marks the current hunk (or change card) seen when the focus is on the page and not in an editor, field or control; the section header's Seen button marks the whole file.

The dialog has three pages. "Review filter" (`ProjectPage`) edits the open repository's filter, which the manifest carries; saving goes through `changeset.setFilter` and the new manifest comes back with `snapshotSettled`. "Language servers" (`ServersPage`, rows from `serverRows.ts`) shows one row per language from `ListServers`: state, origin, path, version, "Install…" with its streamed output (`ServerInstallOutput`) in a collapsible log, "Custom path…" (text field, plus the host's `pick_file` dialog in Tauri) and "Reset" (`SetServerPath` with no path). `ServersChanged` reloads the list. These requests are per session, so `App` passes the backend only while a repository is open; otherwise the page says to open one. "Installing" and "failed" are the page's own state; the protocol only knows found and missing.

## Opening a repository

The host passes the repository from its command line or `VET_REPO` (a plain browser: `#repo=`); `changeset.connect()` opens it. Without one, `features/start` shows the start screen (to be replaced by M1's queue home): "Open repository…" and the recent list. "Open repository…" in the header's project menu does the same from a open repository: `changeset.openRepo` closes the session (`CloseSession`) and opens the chosen one in the same window. The folder dialog and the recent list (a JSON file in the host's app config dir, most recent first, at most 10, backend-side path strings) are Tauri commands behind `state/host.ts`; in the WSL split the host turns `\\wsl.localhost\<distro>\…` into the Linux path and refuses anything else with a message. A failed open shows the backend's message on the start screen, with "Remove from recent" if the entry is in the list.

## Layout

`App` has a tool rail on each side (`chrome/ToolRail`, `side="left" | "right"`). Clicking the selected tool's icon hides its sidebar and keeps the tool selected, so a second click restores it; another icon selects that tool and shows its sidebar. Each sidebar is a `lib/ui/SideBar`: resizable by dragging its splitter (limits `--w-side-min`/`--w-side-max`), reset by double-click, with the width stored per viewer in `localStorage` (every access in try/catch).

## Tabs

The Changes tab is fixed and cannot be closed: the whole changeset as one column of file sections (hunks plus context, unchanged regions collapsed). A file tab shows one file's full diff, without collapsing. A file opens from the tree (double click or Enter; a single click only scrolls the Changes tab to the file), from a section header's file name, from a hunk bar's "Open file", and with F4 (Jump to Source, bound in `ReviewController`) at the caret of the Changes stream, scrolled to that line; opening an open file focuses its tab. Both tab titles show `+N −M`, computed by `editor/lineDelta.ts` over one file or all files. `Tabs.open` is the display order, so drag-to-reorder only has to move an entry.

The Files tool window's header has two scope tabs. "Changes" is the changed-files tree. "All files" lists the target snapshot lazily, one `ListDirectory` per expanded directory (`AllFiles.svelte.ts`, rows in `allFilesRows.ts`); changed entries keep their status and coverage, entries the review filter excludes are dimmed, and a directory with changes below it has a dot. Both scopes render through `lib/ui/VirtualRows` (window maths in `rowWindow.ts`): rows have a fixed height read from `--h-row`, only the rows in the viewport plus 8 on each side are in the DOM between two spacers, and a file selected elsewhere is scrolled into view. A directory whose only child is a directory shows as one row in both scopes (in "All files" once the child is listed). Listings carry the manifest revision they belong to: a reply of another revision is dropped, and a new revision reloads the root and the expanded directories. An unchanged file (double click or Enter) opens a read-only text tab (`editor/TextView.ts`, `FileText`, syntax highlighting, no diff); a file without text (binary, over the capture cap, symlink, submodule) shows a short placeholder. A changed file opens its diff tab as before.

Each tab's pane hands its scroll root to `ReviewController.attach`, which builds a `ChangesetView` in it. Panes stay laid out when inactive (`visibility: hidden`), so scroll positions and editors survive switching. A new manifest revision rebuilds every view and keeps the scroll position and the nearest hunk of the same file as current; file tabs of files that left the changeset close.

In the Changes tab unchanged regions collapse with `@codemirror/merge`'s `collapseUnchanged` (3 lines of context) in both modes. `editor/sectionLayout.ts` mirrors that rule to estimate a section's height from the manifest, including the `--h-section-gap` that closes every section (the editor is clipped above it, and has no padding of its own). CodeMirror's own height is not a reliable measure (it estimates what is off screen), so expanding a region grows the section by exactly the lines the bar hid, compensating the scroll position, and unmounting a section resets it to the estimate (the benchmark reports the estimate error of mounted sections).

The UI computes no diff of a changeset file: `FileContents` carries the changed ranges (`changes`, down to the words that differ), and `editor/backendDiff.ts` hands them to `@codemirror/merge` as its `diffConfig.override`. CodeMirror diffs only the slice around a local edit.

## Change cards

A file's non-text items (mode, symlink target, submodule commit, binary, content unavailable, rename) are change cards inside its section in every `ChangesetView` (the Changes stream and file tabs), between the section header and the first hunk. A card is plain DOM from `editor/changeCard.ts` with a fixed height (a hunk-style bar, one or two rows, and a preview strip for an image), so `sectionLayout` adds exactly that height (the strip's images come from `ItemBlob`, fetched for sections near the viewport and dropped, object URLs revoked, when far away or on destroy; an SVG is only an `<img>` source) and the section's body is offset by it (`--vet-cards`); nothing is measured and no editor is needed for a card to show. Hunk navigation (Ctrl+Alt+Down/Up, the strip's up/down arrows) stops on every card and every hunk, a file's cards first. Space and the card's check button acknowledge it (`Acknowledge` through the store, the new state coming back through `applyReview`); the section header's Seen button toggles hunks and cards together. `ChangeCard.svelte` wraps the same DOM for the dev gallery.

## Comment threads

`state/threads.svelte.ts` holds the backend's threads (`Threads`, resolved at one revision), reloading on `threadsChanged` and `snapshotSettled`; replies to its own requests patch the list. `ReviewController` hands them to every view (`ChangesetView.setThreads`, only if the revision matches the view's) and rebuilt views get them at construction.

A range thread is a card (plain DOM from `editor/threadCard.ts`) in a CodeMirror block widget (`threadWidgets.ts`) below the last line of its range; `threadPlacement.ts` maps a base-side range to the target line it faces. File threads sit in a block between the change cards and the first hunk (`--vet-threads` shortens the body), the changeset's above the first file of the Changes stream. `editor/threadLayer.ts`, owned by `ChangesetView`, holds this state. A thread card's height depends on its text, so unlike a change card it is measured: one `ResizeObserver` per view, a section counts every card (measured, else cached, else estimated by `cardHeight`) and grows by the difference when one reports. `sectionLayout` counts an inline card only where `shownLines` has a row for it, not inside a collapsed region. A card is as wide as the view (`cqw`, sticky), not as the longest line.

A new thread starts from a hunk bar's comment button, a section header's comment button, or Ctrl+Alt+C (the selection of the focused editor, else the current hunk); its form is a card too. The Comments tool window (`features/comments`) lists detached (ambiguous or stale: "Attach to N", "Discard"), moved ("Confirm") and the remaining threads by file (`groups.ts`); a click goes to the thread in the Changes tab.

## Semantic layer in the editor

Hover, go to definition and diagnostics reach every editor through `editor/semantic.ts` (`semanticExtensions`), which asks its owner through `SemanticHooks`; `bindRevision` fixes the revision of a view's requests, as the backend rejects a stale one. Requests go through `state/semantic.svelte.ts`, which drops a stale hover or diagnostics answer quietly (the views are rebuilt on `snapshotSettled`) and says "Changes arrived — try again" for a definition.

- Sides. Positions are 0-based lines and UTF-16 columns of the `side` asked about. The target editor asks `target`, the base editor of a side-by-side view `base`. In a unified view the deleted lines are block widgets outside the document, so `editor/deletedLines.ts` maps the pointer to a base line and column from the DOM.
- Go to definition: Cmd+click (Ctrl+click off macOS) and Cmd+B (Ctrl+B); F4 stays Jump to Source. `ReviewController.goToDefinition` takes the first location inside the repository (`pickLocation`) and opens the diff tab of a changed file, else the read-only text tab. A base-side location lands in base content: the base editor side by side, or in a unified view the target line the diff shows it at (`baseLineToTarget`). A location outside the repository only shows a notice.
- Diagnostics are target-side, asked when an editor mounts (cached per file and revision) and again on `serversChanged`; `@codemirror/lint` draws them. The footer counts them (hints excluded) for the focused file tab, or for every file asked about in the Changes tab.
- Hover content is Markdown, rendered by `editor/hoverContent.ts` for both the CodeMirror tooltip and the deleted lines' floating one (and, as the `codeBlock` action, the symbol panel's signature). `hoverMarkdown.ts` is markdown-it with `html: false` (the text is from an external process; raw HTML is escaped), images off and links shown as their text only (nothing opens URLs externally). Code is highlighted with the editor's Lezer parsers and `highlightStyle` via `highlightCode`; `fenceLanguage.ts` maps the fence's info string to a language, an unlabelled fence takes the hovered file's, an unknown one stays plain.
- Unavailable (SPEC I6) is never shown as "nothing": the footer's language-server slot names it, with the backend's message as its tooltip (`state/semanticSummary.ts`).

## Running

`features/run` over `state/runs.svelte.ts`. The Run split button (`RunButton`) lists the targets of `ListRunTargets` (reloaded when the manifest revision moves) with their manifest (`source`, null for a custom target) as detail; the main button runs the last-used target (remembered per repository in `localStorage`) or the first. "Edit targets…" opens `TargetsDialog` (command as one line, working directory, `NAME=value` lines; `SaveRunTarget`, `ResetRunTarget`, which also deletes a custom target).

`Run` answers with an approval or a start. `runs.approval` holds the answer; `ApprovalDialog` (`RunDialogs`, mounted by `App`) shows why, the changed files and each file's hunks with two lines of context (`approvalRows.ts`; a rename shows "old → new"). Run approves with the hash, Cancel drops it.

A run's state is created when its id is first seen, by output or by the `Run` answer, which attaches the target, snapshot and "N files changed since review". The store keeps every running run and the latest run of each target. `RunPanel` (the `bottom` snippet of `ReviewScreen`) shows one run: stdout and stderr, exit code or "Stopped", the backend's dropped-bytes notice, Stop. Output is decoded per stream as UTF-8 with replacement into a `RunLog` (`state/runLog.ts`, not reactive, 1 MB / 20,000 pieces of scrollback) and drawn by `LogView`, which appends to the DOM once per frame; a chunk never invalidates Svelte state.

## Where things go

- New screen: `features/<name>/` with an `index.ts`; mount it from `app/App.svelte`.
- New backend request: a method in `state/backend.ts`, then a store field if the UI displays it. A request that can fail in a way the viewer should hear about goes through the store, which sets `notice` (shown as a `Toast` in `App`).
- Data the protocol does not carry yet (symbols, footer details) is an optional prop whose absence renders a neutral empty state; fixtures for it live in `dev/` (the gallery) and never reach a production code path.
- Colour or size: `lib/theme/tokens.css` (and a `dev` check in both themes).
- New sidebar tool: an entry in `chrome/ToolRail`, its content as a snippet from `App`.
- New tab kind: an id in `Tabs.svelte.ts`, a pane in `ReviewScreen`, and a case in `ReviewController.build`.
