<script lang="ts">
  import type { Theme } from "../../lib/theme/theme";
  import Dialog from "../../lib/ui/Dialog.svelte";
  import Segmented from "../../lib/ui/Segmented.svelte";

  let {
    theme,
    ontheme,
    onclose,
  }: {
    theme: Theme;
    ontheme: (theme: Theme) => void;
    onclose: () => void;
  } = $props();
</script>

<Dialog title="Settings" {onclose} width="480px">
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
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Close</button>
  {/snippet}
</Dialog>

<style>
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
