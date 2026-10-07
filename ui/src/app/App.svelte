<script lang="ts">
  import { onMount } from "svelte";
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
    toggleTool,
  } from "../features/chrome";
  import { ReviewScreen, type ReviewController } from "../features/review";
  import { SettingsDialog } from "../features/settings";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { WelcomeScreen } from "../features/welcome";
  import { repoName } from "../lib/paths";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import ContextMenu from "../lib/ui/ContextMenu.svelte";
  import Toast from "../lib/ui/Toast.svelte";
  import type { Mode } from "../lib/viewMode";
  import { changeset } from "../state/changeset.svelte";
  import { appVersion, canQuit, hasNativeDialogs, hasNativeMenu, inTauri, quit, showWindow } from "../state/host";
  import { appCommands } from "../state/appCommands";
  import { commands } from "../state/commands";
  import { contextMenus } from "../state/contextMenu.svelte";
  import { keymap } from "../state/keymap.svelte";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { progress as backgroundTasks } from "../state/progress.svelte";
  import { recent } from "../state/recent.svelte";
  import { semantic } from "../state/semantic.svelte";
  import { session } from "../state/session.svelte";

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "files", open: true });
  const right = $state({ tool: "symbol", open: true });
  let tools: DevTools;
  let devOpen = $state(false);
  let theme = $state<Theme>(storedTheme());
  let mode = $state<Mode>("unified");
  let dialog = $state<"settings" | "about" | "finish">();
  let switcherOpen = $state(false);
  // Only the host knows it; a plain browser shows none.
  let version = $state<string>();
  const reviewOpen = $derived(changeset.manifest?.review.state === "open");

  const semanticSummary = $derived(semantic.summary(controller?.activePath));
  const footer = $derived({
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
      fetch: () => void changeset.fetch(),
      openChangeset: (key) => void changeset.openChangeset(key),
      showChangesets: () => (switcherOpen = true),
      showDialog: (d) => (dialog = d),
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

  /** Every way of opening a project (recent list, menu, folder dialog, typed path) ends here. */
  function openProject(path: string) {
    void session.openRepo(path);
  }

  async function pickProject() {
    const path = await session.pickRepo();
    if (path) openProject(path);
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }
</script>

{#if session.backend && session.repo}
  <Header
    project={repoName(session.repo)}
    base={changeset.manifest?.base}
    target={changeset.manifest?.target}
    review={changeset.manifest?.review}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
    loadChangesets={() => changeset.listChangesets()}
    onchangeset={(key) => void changeset.openChangeset(key)}
    bind:switcherOpen
    completion={changeset.manifest?.completion}
    onfinish={() => (dialog = "finish")}
  />

  <Banners degraded={changeset.manifest?.degraded} failures={changeset.restoreFailures} />

  <div class="body">
    <ToolRail bind:active={left.tool} bind:open={left.open} />
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
    </ReviewScreen>
    <ToolRail side="right" bind:active={right.tool} bind:open={right.open} />
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
      onsettings={() => (dialog = "settings")}
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
    min-height: 0;
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
