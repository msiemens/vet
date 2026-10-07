<script lang="ts">
  import type { Manifest } from "../../generated/Manifest";
  import type { ReviewFilter } from "../../generated/ReviewFilter";
  import { THEMES, type Theme } from "../../lib/theme/theme";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import ProjectPage from "./ProjectPage.svelte";
  import ServersPage from "./ServersPage.svelte";

  let {
    theme,
    ontheme,
    onclose,
    project,
    onfilter,
  }: {
    theme: Theme;
    ontheme: (theme: Theme) => void;
    onclose: () => void;
    /** The open repository's current manifest, which carries its review filter. */
    project?: Manifest;
    /** Saves the repository's review filter; resolves with the backend's message if it refused it. */
    onfilter: (filter: ReviewFilter) => Promise<string | undefined>;
  } = $props();

  type Page = "general" | "project" | "servers";
  let page = $state<Page>("general");
</script>

<Dialog title="Settings" {onclose} width="640px">
  <div class="pages">
    <Segmented
      label="Settings page"
      value={page}
      onchange={(p: Page) => {
        page = p;
      }}
      options={[
        { id: "general", label: "General" },
        { id: "project", label: "Review filter" },
        { id: "servers", label: "Language servers" },
      ]}
    />
  </div>
  {#if page === "project"}
    <ProjectPage
      filter={project?.filter}
      excluded={{ lines: project?.excludedLines ?? 0, items: project?.excludedItems ?? 0 }}
      onsave={onfilter}
    />
  {:else if page === "servers"}
    <ServersPage />
  {:else}
    <section>
      <h3>Appearance</h3>
      <div class="row">
        <div class="text">
          <span>Theme</span>
          <span class="hint">System follows the operating system's light or dark setting.</span>
        </div>
        <Segmented label="Theme" value={theme} onchange={ontheme} options={THEMES} />
      </div>
    </section>
  {/if}
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Close</button>
  {/snippet}
</Dialog>

<style>
  .pages {
    margin-bottom: 14px;
  }
  h3 {
    margin: 0 0 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
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
