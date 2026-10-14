<script lang="ts">
  import { errorText } from "../../lib/errorText";
  import { hasNativeDialogs, pickFile } from "../../state/host";
  import { servers } from "../../state/servers.svelte";
  import Sheet from "./Sheet.svelte";

  let {
    language,
    title,
    server,
    current,
    onclose,
  }: {
    language: string;
    /** The language's name and its server's, for the sheet's text. */
    title: string;
    server: string;
    /** The path the field starts with. */
    current?: string;
    onclose: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let path = $state(current ?? "");
  let busy = $state(false);
  let error = $state<string>();

  async function browse() {
    try {
      const picked = await pickFile();
      if (picked) path = picked;
    } catch (e) {
      error = errorText(e);
    }
  }

  async function submit() {
    busy = true;
    error = await servers.setPath(language, path.trim());
    busy = false;
    if (error === undefined) onclose();
  }
</script>

<Sheet title="Custom server path" {busy} {onclose} onsubmit={() => void submit()}>
  <div class="field-row">
    <span>Language</span>
    <span class="value">{title} · {server}</span>
  </div>
  <label>
    <span>Path</span>
    <span class="path">
      <input
        class="field mono"
        type="text"
        placeholder="Path to the server executable"
        autocomplete="off"
        spellcheck="false"
        bind:value={path}
      />
      {#if hasNativeDialogs}<button class="ctl" type="button" disabled={busy} onclick={() => void browse()}
          >Browse…</button
        >{/if}
    </span>
  </label>
  {#if error}<p class="settings-problem" role="alert">{error}</p>{/if}
  {#snippet actions()}
    <button class="ctl" type="button" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="ctl primary" type="submit" disabled={busy || !path.trim()}>Set</button>
  {/snippet}
</Sheet>

<style>
  .value {
    color: var(--text);
  }
  .path {
    display: flex;
    gap: 6px;
    min-width: 0;
  }
  .path input {
    flex: 1;
    min-width: 0;
  }
</style>
