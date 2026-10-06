<script lang="ts">
  import type { Theme } from "../../lib/theme/theme";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";
  import type { Backend } from "../../state/backend";
  import ServersPage from "./ServersPage.svelte";

  let {
    theme,
    ontheme,
    onclose,
    backend,
  }: {
    theme: Theme;
    ontheme: (theme: Theme) => void;
    onclose: () => void;
    /** The language servers belong to the open repository's session, if there is one. */
    backend?: Backend;
  } = $props();

  type Page = "general" | "servers";
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
        { id: "servers", label: "Language servers" },
      ]}
    />
  </div>
  {#if page === "servers"}
    <ServersPage {backend} />
  {:else}
    <section>
      <h3>Appearance</h3>
      <div class="row">
        <div class="text">
          <span>Theme</span>
          <span class="hint">System follows the operating system's light or dark setting.</span>
        </div>
        <Segmented
          label="Theme"
          value={theme}
          onchange={ontheme}
          options={[
            { id: "system", label: "System" },
            { id: "light", label: "Light" },
            { id: "dark", label: "Dark" },
          ]}
        />
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
