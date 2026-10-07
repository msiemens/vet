<script lang="ts">
  import { type ComponentProps, onMount } from "svelte";
  import DevTools from "../dev/DevTools.svelte";
  import { probe } from "../dev/probe.svelte";
  import {
    AboutDialog,
    Banners,
    FinishDialog,
    Header,
    StatusFooter,
    TOOLS,
    TaskProgress,
    TitleBar,
    ToolRail,
    type Screen,
    toggleTool,
  } from "../features/chrome";
  import { ChangesetSwitcher, OpenPullRequestDialog, QueueScreen, targetLabel } from "../features/queue";
  import { ReviewScreen, type ReviewController, summarize } from "../features/review";
  import { SettingsDialog } from "../features/settings";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { type RangeFile, RangeChips, TimelinePanel } from "../features/timeline";
  import { ProjectSwitcher, WelcomeScreen } from "../features/welcome";
  import type { Thread } from "../generated/Thread";
  import { repoName } from "../lib/paths";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Toast from "../lib/ui/Toast.svelte";
  import type { Mode } from "../lib/viewMode";
  import { changeset } from "../state/changeset.svelte";
  import {
    appVersion,
    canQuit,
    hasNativeDialogs,
    hasNativeMenu,
    hostLabel,
    inTauri,
    quit,
    showWindow,
  } from "../state/host";
  import { appCommands } from "../state/appCommands";
  import { commands } from "../state/commands";
  import { contextMenus } from "../state/contextMenu.svelte";
  import { keymap } from "../state/keymap.svelte";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { progress as backgroundTasks } from "../state/progress.svelte";
  import { queue } from "../state/queue.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { session } from "../state/session.svelte";
  import { defaultRange } from "../state/timelineRange";
  import { versions } from "../state/versions.svelte";

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "files", open: true });
  const right = $state({ tool: "symbol", open: true });
  let tools: DevTools;
  let devOpen = $state(false);
  let timelineOpen = $state(false);
  let theme = $state<Theme>(storedTheme());
  let mode = $state<Mode>("unified");
  let dialog = $state<"settings" | "about" | "finish" | "pullRequest">();
  let settingsPage = $state<ComponentProps<typeof SettingsDialog>["page"]>("appearance");
  let switcherOpen = $state(false);
  // The queue is the project's home; the review screen stays mounted behind it, so its views survive.
  let screen = $state<Screen>("queue");
  // Only the host knows these; a plain browser shows none.
  let version = $state<string>();
  let host = $state<string>();
  const reviewOpen = $derived(changeset.manifest?.review.state === "open");

  const semanticSummary = $derived(semantic.summary(controller?.activePath));
  const range = $derived(changeset.range);
  const rangeFiles = $derived<RangeFile[]>(
    range ? changeset.files.map((f) => ({ path: f.path, delta: f.delta, mode: summarize(f).mode })) : [],
  );
  const footer = $derived({
    range: range && `v${range.from} → v${range.to}`,
    languageServer: semanticSummary.label,
    languageServerTitle: semanticSummary.title,
    diagnostics: semanticSummary.diagnostics,
  });

  const openLocation: OpenLocation = (path, line, side) => void controller?.openLocation(path, line, side);

  // Registered before the first render, so the menu below is built from them.
  const stopCommands = commands.register(
    ...appCommands({
      pickProject: () => void pickProject(),
      openProject,
      finishReview: () => (dialog = "finish"),
      changesSinceReview: () => void showChangesSinceReview(),
      fetch: () => void changeset.fetch(),
      openChangeset: (key) => void changeset.openChangeset(key),
      showQueue: () => (screen = "queue"),
      showChangesets: () => {
        screen = "review";
        switcherOpen = true;
      },
      openPullRequest: () => void openPullRequest(),
      showDialog: (d) => showDialog(d),
      quit: () => void quit(),
      setMode,
      toggleTool: (id) => {
        const side = TOOLS.left.some((t) => t.id === id) ? left : right;
        Object.assign(side, toggleTool(side, id));
      },
      setTheme: (t) => applyTheme((theme = t)),
      toggleDevPanel: () => (devOpen = !devOpen),
    }),
  );
  keymap.load();
  session.onOpen(() => (screen = "queue"));
  const stopContext = commands.provide(() => ({ repoOpen: !!session.repo, reviewOpen, canQuit }));

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: typeof left, id: string) => side.open && side.tool === id;
    mainMenu.update({
      native: hasNativeMenu,
      recent: recent.list.map(({ path }) => ({ path, name: repoName(path) })),
      mode,
      theme,
      tools: [
        ...TOOLS.left.map((t) => ({ id: t.id, label: t.label, shown: shown(left, t.id) })),
        ...TOOLS.right.map((t) => ({ id: t.id, label: t.label, shown: shown(right, t.id) })),
      ],
      devPanel: devOpen,
    });
  });

  onMount(() => {
    // Capturing, so a command's key reaches it before any control or editor handles the key.
    const onkeydown = (e: KeyboardEvent) => {
      if (commands.dispatch(e) || !probe.key(e)) return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onkeydown, true);
    // The webview's own menu stays on text fields; elsewhere a handler opened the app's menu, or there is none.
    const onContextMenu = (e: MouseEvent) => {
      if (!(e.target instanceof Element && e.target.closest("input, textarea"))) e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    applyTheme(theme);
    // The splash covers the start until the backend is connected (or the connection failed, which
    // the window then shows), so the window never shows empty.
    let shown: Promise<void> | undefined;
    const reveal = () => {
      shown ??= showWindow().then(
        (frame) => {
          if (frame === "native") console.warn("app-drawn title bar unavailable; the host log says why");
        },
        (e: unknown) => {
          console.error(e);
        },
      );
    };
    if (inTauri) void appVersion().then((v) => (version = v));
    void hostLabel().then((label) => (host = label));
    session.onConnect(reveal);
    void session.connect().then(reveal);
    const stopMenu = mainMenu.listen();
    return () => {
      stopMenu();
      window.removeEventListener("keydown", onkeydown, true);
      window.removeEventListener("contextmenu", onContextMenu);
      stopContext();
      stopCommands();
    };
  });

  function showDialog(d: "settings" | "about", page: typeof settingsPage = "appearance") {
    settingsPage = page;
    dialog = d;
  }

  /** Asks for a pull request by its number, in the repository the project has on GitHub. */
  async function openPullRequest() {
    if (!queue.sources.length) await queue.loadPullRequests();
    if (queue.sources.length) dialog = "pullRequest";
    else session.notice = "This project has no GitHub remote to open a pull request from";
  }

  /** Every way of opening a project (recent list, menu, folder dialog, typed path) ends here. */
  function openProject(path: string) {
    void session.openRepo(path);
  }

  async function pickProject() {
    const path = await session.pickRepo();
    if (path) openProject(path);
  }

  /** The review view compares the last reviewed version with the latest; with nothing after it, the changeset shows. */
  async function showChangesSinceReview() {
    const since = defaultRange(versions.items);
    if (!since) session.notice = "Nothing has changed since your last review";
    await changeset.setRange(since);
  }

  /** Opening the timeline shows the changes since the last review. */
  function toggleTimeline() {
    timelineOpen = !timelineOpen;
    if (timelineOpen) void changeset.setRange(defaultRange(versions.items));
  }

  /** Shows the review at a file. */
  function openFile(path: string) {
    screen = "review";
    void controller?.revealInChanges(path);
  }

  function openThread(thread: Thread) {
    screen = "review";
    void controller?.goToThread(thread);
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }
</script>

{#if session.backend && session.repo}
  {#snippet rangeChips()}
    {#if range}
      <RangeChips {range} versions={versions.items} onrange={(r) => void changeset.setRange(r)} />
    {/if}
  {/snippet}
  {#snippet backToChangeset()}
    {@const base = changeset.manifest?.base.name}
    {@const latest = versions.items.at(-1)?.n}
    <button
      class="ctl back"
      type="button"
      title="Show the whole changeset again"
      onclick={() => void changeset.setRange(undefined)}
      >Back to {base ?? "…"}{latest === undefined ? "" : ` → v${latest}`}</button
    >
  {/snippet}
  {#snippet projectSwitcher()}
    {#if session.repo}
      <ProjectSwitcher
        name={repoName(session.repo)}
        path={session.repo}
        recent={recent.list}
        {host}
        canPick={hasNativeDialogs}
        onopen={openProject}
        onpick={() => void pickProject()}
      />
    {/if}
  {/snippet}
  {#snippet changesetSwitcher()}
    {@const target = changeset.manifest?.target}
    <ChangesetSwitcher
      label={target && targetLabel(target)}
      draft={target?.draft}
      title={target?.branch && `On ${target.branch}`}
      onpick={(key) => void changeset.openChangeset(key)}
      onqueue={() => (screen = "queue")}
      onopenpullrequest={() => void openPullRequest()}
      bind:open={switcherOpen}
    />
  {/snippet}
  <Header
    {screen}
    onscreen={(s) => void (screen = s)}
    project={projectSwitcher}
    changesets={changesetSwitcher}
    chips={range ? rangeChips : undefined}
    back={backToChangeset}
    base={changeset.manifest?.base}
    target={changeset.manifest?.target}
    review={changeset.manifest?.review}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
    completion={changeset.manifest?.completion}
    onfinish={() => (dialog = "finish")}
    refreshing={queue.refreshing}
    onrefresh={() => void queue.refresh()}
  />

  <Banners degraded={changeset.manifest?.degraded} failures={changeset.restoreFailures} />

  <div class="body">
    {#if screen === "queue"}
      <QueueScreen
        onopen={() => (screen = "review")}
        onopenfile={openFile}
        onopenthread={openThread}
        onsignin={() => showDialog("settings", "accounts")}
      />
    {/if}
    <div class="review" class:away={screen !== "review"} inert={screen !== "review"}>
      <ToolRail bind:active={left.tool} bind:open={left.open} timeline={timelineOpen} ontimeline={toggleTimeline} />
      <ReviewScreen
        backend={session.backend}
        {changeset}
        {probe}
        leftTool={left.tool}
        leftOpen={left.open}
        rightTool={right.tool}
        rightOpen={right.open}
        {mode}
        onmode={setMode}
        onready={(c, firstPaintMs) => {
          controller = c;
          void tools.ready(firstPaintMs);
        }}
        onclose={() => (controller = undefined)}
      >
        {#snippet side(tool)}{#if tool === "symbol"}<SymbolPanel onopen={openLocation} />{/if}{/snippet}
        {#snippet bottom({ selected, select })}
          {#if timelineOpen}
            <TimelinePanel
              versions={versions.items}
              {range}
              files={rangeFiles}
              {selected}
              onrange={(r) => void changeset.setRange(r)}
              onselect={select}
              onclose={() => (timelineOpen = false)}
            />
          {/if}
        {/snippet}
      </ReviewScreen>
      <ToolRail side="right" bind:active={right.tool} bind:open={right.open} />
    </div>
  </div>
{:else if session.backend}
  <div class="body">
    <WelcomeScreen
      recent={recent.list}
      failed={session.openError}
      canPick={hasNativeDialogs}
      {version}
      onpick={() => void pickProject()}
      onopen={openProject}
      onforget={(path) => void session.forget(path)}
      onsettings={() => showDialog("settings")}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{session.status}</p>
{/if}

{#if session.notice}
  {#key session.notice}
    <Toast message={session.notice} onclose={() => (session.notice = undefined)} />
  {/key}
{/if}

{#if dialog === "settings"}
  <SettingsDialog
    bind:page={settingsPage}
    {theme}
    ontheme={(t) => applyTheme((theme = t))}
    onclose={() => (dialog = undefined)}
    project={session.repo ? changeset.manifest : undefined}
    onfilter={(f) => changeset.setFilter(f)}
  />
{:else if dialog === "about"}
  <AboutDialog onclose={() => (dialog = undefined)} />
{:else if dialog === "finish"}
  <FinishDialog
    onclose={() => (dialog = undefined)}
    onfinish={() => {
      dialog = undefined;
      void changeset.finishReview();
    }}
  />
{:else if dialog === "pullRequest"}
  <OpenPullRequestDialog
    remotes={queue.sources.map((s) => s.remote)}
    onclose={() => (dialog = undefined)}
    onopen={(remote, number) => {
      dialog = undefined;
      void changeset.openChangeset({ type: "pullRequest", remote, number });
    }}
  />
{/if}

{#if contextMenus.drawn}
  {@const request = contextMenus.drawn}
  <ContextMenu
    x={request.x}
    y={request.y}
    items={request.items}
    onselect={(id) => request.onselect(id)}
    onclose={() => contextMenus.close()}
  />
{/if}

<DevTools bind:this={tools} {controller} bind:open={devOpen} />

<StatusFooter info={footer}>
  {#snippet progress()}<TaskProgress tasks={backgroundTasks.tasks} />{/snippet}
  {#snippet end()}<button class="flat dev" type="button" aria-pressed={devOpen} onclick={() => (devOpen = !devOpen)}
      >Dev</button
    >{/snippet}
</StatusFooter>

<style>
  .body {
    flex: 1;
    display: flex;
    position: relative;
    min-height: 0;
  }
  .review {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  /* Laid out but not seen, so the editors keep their size and scroll positions behind the queue. */
  .review.away {
    position: absolute;
    inset: 0;
    visibility: hidden;
  }
  .message {
    flex: 1;
    margin: 16px;
    color: var(--text-2);
  }
  .dev {
    height: 18px;
    padding: 0 6px;
    border-radius: var(--radius);
    color: var(--text-3);
    font-size: 11px;
  }
</style>
