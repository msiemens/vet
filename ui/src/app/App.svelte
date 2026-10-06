<script lang="ts">
  import { onMount } from "svelte";
  import DevTools from "../dev/DevTools.svelte";
  import { hudListeners } from "../dev/hud.svelte";
  import { AboutDialog, Header, StatusFooter, TOOLS, TitleBar, ToolRail } from "../features/chrome";
  import { FilterPopover } from "../features/filter";
  import { type Mode, ReviewScreen, type ReviewController } from "../features/review";
  import { RunButton } from "../features/run";
  import { SettingsDialog } from "../features/settings";
  import { StartScreen } from "../features/start";
  import { SymbolPanel } from "../features/symbols";
  import { matches } from "../lib/keymap";
  import { type Theme, applyTheme, storedTheme } from "../lib/theme/theme";
  import Toast from "../lib/ui/Toast.svelte";
  import { changeset } from "../state/changeset.svelte";
  import { canPickFolder, canQuit, isMac, quit, repoName, showWindow } from "../state/host";
  import type { Command } from "../state/mainMenu";
  import { mainMenu } from "../state/mainMenu.svelte";
  import { recent } from "../state/recent.svelte";

  let controller = $state.raw<ReviewController>();
  // The selected tool of each rail, and whether its sidebar is shown.
  const left = $state({ tool: "files", open: true });
  const right = $state({ tool: "symbol", open: true });
  let tools: DevTools;
  let devOpen = $state(false);
  let status = $state("");
  let theme = $state<Theme>(storedTheme());
  let mode = $state<Mode>("unified");
  let dialog = $state<"settings" | "about">();

  const completion = $derived.by(() => {
    const c = changeset.manifest?.completion;
    if (!c) return { seenPct: 0, oldPct: 0 };
    const total = c.lines + c.items;
    const pct = (n: number) => (total ? (n / total) * 100 : 0);
    return {
      seenPct: pct(c.seenLines + c.acknowledgedItems),
      oldPct: pct(c.oldBaseLines),
      title: `${c.seenLines} of ${c.lines} lines · ${c.acknowledgedItems} of ${c.items} other changes · ${c.oldBaseLines} lines against an old base`,
    };
  });
  const excluded = $derived({
    lines: changeset.manifest?.excludedLines ?? 0,
    items: changeset.manifest?.excludedItems ?? 0,
  });
  const footer = $derived({ snapshot: `#${changeset.manifest?.revision ?? 0}` });

  // The controller belongs to the repository's screen.
  $effect(() => {
    if (!changeset.repo) controller = undefined;
  });

  // The main menu follows the state it shows.
  $effect(() => {
    const shown = (side: typeof left, id: string) => side.open && side.tool === id;
    mainMenu.update({
      mac: isMac,
      repoOpen: !!changeset.repo,
      canQuit,
      recent: recent.list.map((path) => ({ path, name: repoName(path) })),
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
    showWindow().then(
      (frame) => {
        if (frame === "native") console.warn("app-drawn title bar unavailable; the host log says why");
      },
      (e: unknown) => {
        console.error(e);
      },
    );
    void changeset.connect();
    return mainMenu.listen(execute);
  });

  function execute(command: Command) {
    switch (command.type) {
      case "open":
        void changeset.pickRepo();
        break;
      case "openRecent":
        void changeset.openRepo(command.path);
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
      case "tool":
        toggleTool(command.id);
        break;
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

  /** Like the rail's icon: the selected tool's window hides or shows, another tool is selected and shown. */
  function toggleTool(id: string) {
    const side = TOOLS.left.some((t) => t.id === id) ? left : right;
    if (side.tool === id) side.open = !side.open;
    else [side.tool, side.open] = [id, true];
  }

  function ready(c: ReviewController, firstPaintMs: number) {
    controller = c;
    status = `${changeset.manifest?.name}: ${changeset.files.length} files, ${c.changes.totalHunks} hunks; first hunk painted in ${firstPaintMs.toFixed(0)} ms`;
    void tools.runIfRequested();
  }
</script>

<svelte:window
  onkeydowncapture={(e) => {
    if (!matches("Ctrl-Alt-s", e)) return;
    e.preventDefault();
    dialog = "settings";
  }}
/>

{#if changeset.backend && changeset.repo}
  <Header
    info={{
      project: repoName(changeset.repo),
      changeset: "Working tree",
      target: `#${changeset.manifest?.revision ?? 0}`,
    }}
    base={changeset.manifest?.base}
    loadBase={() => changeset.baseCandidates()}
    onbase={(choice) => void changeset.setBase(choice)}
    {completion}
    onopenrepo={() => void changeset.pickRepo()}
  >
    {#snippet filter()}
      {#if changeset.manifest}
        <FilterPopover filter={changeset.manifest.filter} {excluded} onsave={(f) => changeset.setFilter(f)} />
      {/if}
    {/snippet}
    {#snippet run()}<RunButton />{/snippet}
  </Header>

  <div class="body">
    <ToolRail bind:active={left.tool} bind:open={left.open} />
    <ReviewScreen
      backend={changeset.backend}
      hud={hudListeners}
      leftTool={left.tool}
      leftOpen={left.open}
      rightTool={right.tool}
      rightOpen={right.open}
      {mode}
      onmode={setMode}
      onready={ready}
    >
      {#snippet side(tool)}{#if tool === "symbol"}<SymbolPanel />{/if}{/snippet}
    </ReviewScreen>
    <ToolRail side="right" bind:active={right.tool} bind:open={right.open} />
  </div>
{:else if changeset.backend}
  <TitleBar />
  <div class="body">
    <StartScreen
      recent={recent.list}
      failed={changeset.openError}
      canPick={canPickFolder}
      onpick={() => void changeset.pickRepo()}
      onopen={(path) => void changeset.openRepo(path)}
      onforget={(path) => {
        void recent.forget(path);
        changeset.openError = undefined;
      }}
    />
  </div>
{:else}
  <TitleBar />
  <p class="message">{changeset.status}</p>
{/if}

{#if changeset.notice}
  {#key changeset.notice}
    <Toast message={changeset.notice} onclose={() => (changeset.notice = undefined)} />
  {/key}
{/if}

{#if dialog === "settings"}
  <SettingsDialog {theme} ontheme={(t) => applyTheme((theme = t))} onclose={() => (dialog = undefined)} />
{:else if dialog === "about"}
  <AboutDialog onclose={() => (dialog = undefined)} />
{/if}

<DevTools bind:this={tools} {controller} bind:open={devOpen} bind:status />

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
