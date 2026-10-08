<script lang="ts">
  import type { Manifest } from "../../generated/Manifest";
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import type { SoftWrap } from "../../lib/softWrap";
  import { THEMES, type Theme } from "../../lib/theme/theme";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import ScrollArea from "../../lib/ui/ScrollArea.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import AccountsPage from "./AccountsPage.svelte";
  import GeneralPage from "./GeneralPage.svelte";
  import KeymapPage from "./KeymapPage.svelte";
  import ProjectPage from "./ProjectPage.svelte";
  import ServersPage from "./ServersPage.svelte";

  let {
    theme,
    ontheme,
    softWrap,
    onsoftwrap,
    onclose,
    project,
    onfilter,
    page = $bindable("appearance"),
  }: {
    /** The page shown; the viewer's choice of another is passed back. */
    page?: Page;
    theme: Theme;
    ontheme: (theme: Theme) => void;
    softWrap: SoftWrap;
    onsoftwrap: (wrap: SoftWrap) => void;
    onclose: () => void;
    /** The open repository's current manifest, which carries its review filter. */
    project?: Manifest;
    /** Saves the repository's review filter; resolves with the backend's message if it refused it. */
    onfilter: (filter: ReviewFilter) => Promise<string | undefined>;
  } = $props();

  // A Backends page is not listed: SPEC names no setting of its own for it.
  const PAGES = [
    { id: "general", label: "General" },
    { id: "appearance", label: "Appearance" },
    { id: "keymap", label: "Keymap" },
    { id: "review", label: "Review" },
    { id: "servers", label: "Language servers" },
    { id: "accounts", label: "Accounts" },
  ] as const;
  type Page = (typeof PAGES)[number]["id"];
</script>

<Dialog title="Settings" {onclose} width="780px" height="min(520px, 76vh)" resizable="settings" padded={false}>
  <div class="layout">
    <nav aria-label="Settings pages">
      {#each PAGES as p (p.id)}
        <button type="button" aria-current={page === p.id ? "page" : undefined} onclick={() => (page = p.id)}
          >{p.label}</button
        >
      {/each}
    </nav>
    <ScrollArea>
      <div class="page">
        {#if page === "general"}
          <GeneralPage {softWrap} {onsoftwrap} />
        {:else if page === "keymap"}
          <KeymapPage />
        {:else if page === "review"}
          <ProjectPage
            filter={project?.filter}
            excluded={{ lines: project?.excludedLines ?? 0, items: project?.excludedItems ?? 0 }}
            onsave={onfilter}
          />
        {:else if page === "servers"}
          <ServersPage />
        {:else if page === "accounts"}
          <AccountsPage />
        {:else}
          <div class="row">
            <div class="text">
              <span>Theme</span>
              <span class="hint">System follows the operating system's light or dark setting.</span>
            </div>
            <Segmented label="Theme" value={theme} onchange={ontheme} options={THEMES} />
          </div>
        {/if}
      </div>
    </ScrollArea>
  </div>
</Dialog>

<style>
  .layout {
    display: flex;
    height: 100%;
  }
  nav {
    flex: none;
    width: 190px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px;
    overflow: auto;
    border-right: 1px solid var(--line-soft);
    background: var(--bg-panel);
  }
  nav button {
    height: 26px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-ctl);
    text-align: left;
  }
  nav button:hover {
    background: var(--hover);
  }
  nav button[aria-current="page"] {
    background: var(--toggle);
  }
  /* Less on the right, where the scroll bar's gutter adds its own. */
  .page {
    padding: 16px calc(16px - var(--w-scrollbar)) 16px 16px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-ctl);
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
  }
</style>
