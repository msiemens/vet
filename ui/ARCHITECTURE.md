# UI architecture

Svelte 5 + CodeMirror 6, organised by feature under `src/`. What the UI does is in `SPEC.md` §10; this file is how it is built.

| Folder                    | Holds                                                                                                                                                                                                                                                                                                 |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app/`                    | `App.svelte`: composition only. Wires features together, applies the theme, owns which tool of each rail is selected and whether its sidebar is shown.                                                                                                                                                |
| `lib/theme/`              | `tokens.css` (the only place colours and dimensions are defined, light and dark via `light-dark()`), `global.css` (control states, text selection), `theme.ts`.                                                                                                                                       |
| `lib/ui/`                 | Domain-free primitives (buttons, chips, menus, popovers, dialogs, sidebars, virtual rows). Check here before adding one. `lib/keymap.ts` matches key events to bindings.                                                                                                                              |
| `features/<domain>/`      | One folder per domain: components, its own logic, and an `index.ts` as the public entry.                                                                                                                                                                                                              |
| `features/review/`        | The review screen: file tree, tab strip and panes, `ReviewController` (builds a view per tab), `Tabs.svelte.ts` (open tabs).                                                                                                                                                                          |
| `features/review/editor/` | CodeMirror integration: framework-agnostic TS, no Svelte imports. `ChangesetView` (a column of file sections, virtualized), `mount` (one file's editor), `sectionLayout` (section heights from the manifest), hunk bars, cards and review decorations.                                                |
| `state/`                  | `connection.ts` (transport), `backend.ts` (typed API over it), `host.ts` (the Tauri host's commands), `session.svelte.ts` (the connection, the open repository, notices) and view-model stores. No product logic (SPEC I5): review state, completion and names are rendered as the backend sent them. |
| `dev/`                    | Benchmark, metrics, HUD and a gallery on fixtures, behind the "Dev" toggle in the footer.                                                                                                                                                                                                             |

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

Controls take one of two classes from `global.css`, which applies the state tokens (`--hov`, `--press`, `--tog`, `--focus`): `.flat` (borderless: icon buttons, rows, menu items) and `.ctl` (bordered: chips, text buttons; `.ctl-group` joins several). Toggled is `aria-pressed="true"` or `.on`. A new control takes one of the classes and does not restyle its states; editor widgets use the same classes.

Chrome is not selectable (`user-select: none` on `body`); content meant to be copied opts in with `.selectable`.

## Main menu

`state/mainMenu.ts` defines the menu once (`buildMenu`) from the current state; `App` feeds it through `mainMenu.update` and executes the commands (`parseCommand`) it reports. On macOS the host draws it natively: `state/mainMenu.svelte.ts` sends the definition to the host's `set_menu`, and a chosen item comes back as the `vet-menu` event. Elsewhere the `TitleBar` draws the same items with `Menu`.

## Host

The host passes the repository to open (command line or `VET_REPO`; in a plain browser `#repo=`). The folder dialog and the file picker are Tauri commands behind `state/host.ts`; the recent list is the backend's (`state/recent.svelte.ts`); paths stay backend-side strings (in the WSL split the host turns `\\wsl.localhost\<distro>\…` into the Linux path).

## Views

Each tab's pane hands its scroll root to `ReviewController.attach`, which builds a `ChangesetView` in it (a read-only `TextView` for an unchanged file). Panes stay laid out when inactive (`visibility: hidden`), so scroll positions and editors survive switching. A new manifest revision rebuilds every view.

- Heights come from the manifest, not from CodeMirror, which estimates what is off screen. `editor/sectionLayout.ts` mirrors `collapseUnchanged` to compute a section's height, including change cards (fixed heights, nothing measured) and thread cards (measured by one `ResizeObserver` per view, estimated until then). Expanding a region grows its section by exactly the lines it hid; unmounting a section resets it to the estimate.
- The UI computes no diff of a changeset file: `FileContents` carries the changed ranges (`changes`, down to the words that differ), and `editor/backendDiff.ts` hands them to `@codemirror/merge` as its `diffConfig.override`. CodeMirror diffs only the slice around a local edit.
- The UI parses no code: syntax highlights come with `FileContents`, `FileText` and hover code blocks, and `editor/highlights.ts` turns them into classes.
- Layout reads happen in CodeMirror's measure phase (`requestMeasure`), never on the mount path. A plugin asks for a measure only on changes that can alter what it reads (document, viewport, its own effects), never on a height-only update: those come from the measure phase itself, and asking from one restarts CodeMirror's measure loop. `gutterWidth.ts` publishes the gutters' width once per editor as `--vet-gutters`.
- Lists (the file tree's scopes) render through `lib/ui/VirtualRows`: fixed row height from `--h-row`, only the visible rows plus a margin in the DOM. Directory listings carry the revision they belong to; a reply of another revision is dropped.

## Threads

`state/threads.svelte.ts` holds the backend's threads, resolved at one revision, and reloads on `threadsChanged` and `snapshotSettled`. `ReviewController` hands them to every view whose revision matches. In a view, `editor/threadLayer.ts` owns them: range threads are block widgets (`threadWidgets.ts`) below the target line the backend sends with each thread; file and changeset threads are blocks outside the editor.

## Semantics

Hover, go to definition and diagnostics reach every editor through `editor/semantic.ts`, which asks its owner through `SemanticHooks`; `bindRevision` fixes the revision of a view's requests, since the backend rejects a stale one. `state/semantic.svelte.ts` drops a stale answer.

- Positions are 0-based lines and UTF-16 columns of the side asked about. In a unified view the deleted lines are widgets outside the document, so `editor/deletedLines.ts` maps the pointer to a base position from the DOM.
- Hover text comes from an external process and is untrusted: `editor/hoverMarkdown.ts` renders it with raw HTML escaped, no images and no link targets.
- Unavailable (SPEC I6) is never shown as nothing: `state/semanticSummary.ts` turns it into the footer's language-server text.

## Where things go

- New screen: `features/<name>/` with an `index.ts`; mount it from `app/App.svelte`.
- New backend request: a method in `state/backend.ts`, then a store field if the UI displays it. A request that can fail in a way the viewer should hear about goes through the store, which sets `session.notice` (shown as a `Toast` in `App`).
- Data the protocol does not carry yet is an optional prop whose absence renders a neutral empty state; fixtures for it live in `dev/` and never reach a production code path.
- Colour or size: `lib/theme/tokens.css` (and a `dev` check in both themes).
- New sidebar tool: an entry in `chrome/ToolRail`, its content as a snippet from `App`.
- New tab kind: an id in `Tabs.svelte.ts`, a pane in `ReviewScreen`, and a case in `ReviewController.build`.
