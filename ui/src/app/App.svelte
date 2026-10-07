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
    TitleBar,
    ToolRail,
    toggleTool,
  } from "../features/chrome";
  import { ReviewScreen, type ReviewController } from "../features/review";
  import { SettingsDialog } from "../features/settings";
  import { StartScreen } from "../features/start";
  import { type OpenLocation, SymbolPanel } from "../features/symbols";
  import { KEYS, matches } from "../lib/keymap";
  import { repoName } from "../lib/paths";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import Toast from "../lib/ui/Toast.svelte";
  import type { Mode } from "../lib/viewMode";
  import { changeset } from "../state/changeset.svelte";
  import { canQuit, hasNativeDialogs, hasNativeMenu, quit, showWindow } from "../state/host";
  import type { Command } from "../state/mainMenu";
  import { mainMenu } from "../state/mainMenu.svelte";
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
  const reviewOpen = $derived(changeset.manifest?.review.state === "open");

  const semanticSummary = $derived(semantic.summary(controller?.activePath));
  const footer = $derived({
    languageServer: semanticSummary.label,
    languageServerTitle: semanticSummary.title,
    diagnostics: semanticSummary.diagnostics,
  });

  const openLocation: OpenLocation = (path, line, side) => void controller?.openLocation(path, line, side);

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: typeof left, id: string) => side.open && side.tool === id;
    mainMenu.update({
      native: hasNativeMenu,
      repoOpen: !!session.repo,
      reviewOpen,
      canQuit,
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
    session.onConnect(reveal);
    void session.connect().then(reveal);
    return mainMenu.listen(execute);
  });

  function execute(command: Command) {
    switch (command.type) {
      case "open":
        void session.pickRepo();
        break;
      case "openRecent":
        void session.openRepo(command.path);
        break;
      case "finishReview":
        if (reviewOpen) dialog = "finish";
        break;
      case "settings":
      case "about":
        dialog = command.type;
        break;
      case "quit":
        void quit();
        break;
      case "mode":
        setMode(command.mode);
        break;
      case "tool": {
        const side = TOOLS.left.some((t) => t.id === command.id) ? left : right;
        Object.assign(side, toggleTool(side, command.id));
        break;
      }
      case "theme":
        applyTheme((theme = command.theme));
        break;
      case "devPanel":
        devOpen = !devOpen;
        break;
    }
  }

  function setMode(next: Mode) {
    mode = next;
    controller?.setMode(next);
  }
</script>

<svelte:window
  onkeydowncapture={(e) => {
    if (!matches(KEYS.settings.spec, e)) return;
    e.preventDefault();
    dialog = "settings";
  }}
/>

{#if session.backend && session.repo}
  <Header
    project={repoName(session.repo)}
    base={changeset.manifest?.base}
    target={changeset.manifest?.target}
    review={changeset.manifest?.review}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
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
  <TitleBar />
  <div class="body">
    <StartScreen
      recent={recent.list.map((p) => p.path)}
      failed={session.openError}
      canPick={hasNativeDialogs}
      onpick={() => void session.pickRepo()}
      onopen={(path) => void session.openRepo(path)}
      onforget={(path) => void session.forget(path)}
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

<DevTools bind:this={tools} {controller} bind:open={devOpen} />

<StatusFooter info={footer}>
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
