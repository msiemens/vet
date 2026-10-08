<script lang="ts">
  import Dialog from "../../lib/ui/Dialog.svelte";
  import { pullRequestNumber } from "./queueRows";

  let {
    remotes,
    onopen,
    onclose,
  }: {
    /** The hosted repositories of the project, as `github.com/acme/app` or `dev.azure.com/org/project/repo`. */
    remotes: string[];
    onopen: (remote: string, number: number) => void;
    onclose: () => void;
  } = $props();

  let typed = $state("");
  /** The repository picked; the first until one is. */
  let picked = $state<string>();
  const remote = $derived(picked ?? remotes.at(0) ?? "");
  const number = $derived(pullRequestNumber(typed));
</script>

<Dialog title="Open pull request" {onclose} width="380px">
  <form
    id="open-pull-request"
    onsubmit={(e) => {
      e.preventDefault();
      if (number !== undefined && remote) onopen(remote, number);
    }}
  >
    {#if remotes.length > 1}
      <label>
        <span>Repository</span>
        <select class="field mono" bind:value={picked}>
          {#each remotes as r (r)}<option value={r}>{r}</option>{/each}
        </select>
      </label>
    {/if}
    <label>
      <span>Number{remotes.length === 1 ? ` in ${remotes[0]}` : ""}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="field mono"
        type="text"
        inputmode="numeric"
        placeholder="482"
        autocomplete="off"
        spellcheck="false"
        autofocus
        bind:value={typed}
      />
    </label>
  </form>
  {#snippet actions()}
    <button class="ctl" type="button" onclick={onclose}>Cancel</button>
    <button class="ctl" type="submit" form="open-pull-request" disabled={number === undefined || !remote}>Open</button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    font-size: var(--fs-ctl);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  span {
    color: var(--text-2);
  }
</style>
