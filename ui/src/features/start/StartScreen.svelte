<script lang="ts">
  import Icon from "../../lib/ui/Icon.svelte";
  import { repoName } from "../../state/host";

  let {
    recent,
    failed,
    canPick,
    onpick,
    onopen,
    onforget,
  }: {
    /** Backend-side paths, most recent first. */
    recent: string[];
    /** The last open attempt that failed; `path` is empty when the folder dialog itself refused. */
    failed?: { path: string; message: string };
    /** A native folder dialog exists; otherwise the path is typed. */
    canPick: boolean;
    onpick: () => void;
    onopen: (path: string) => void;
    onforget: (path: string) => void;
  } = $props();

  let typed = $state("");
</script>

<main>
  <div class="column">
    <h1>Vet</h1>
    <p class="lead">Open a repository to review its working tree.</p>

    {#if canPick}
      <button class="ctl open" type="button" onclick={onpick}>
        <Icon name="files" size={14} />Open repository…
      </button>
    {:else}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          if (typed.trim()) onopen(typed.trim());
        }}
      >
        <input type="text" bind:value={typed} placeholder="/path/to/repository" aria-label="Repository path" />
        <button class="ctl" type="submit">Open</button>
      </form>
    {/if}

    {#if failed}
      <div class="error" role="alert">
        <span class="selectable">{failed.message}</span>
        {#if recent.includes(failed.path)}
          <button class="ctl" type="button" onclick={() => onforget(failed.path)}>Remove from recent</button>
        {/if}
      </div>
    {/if}

    {#if recent.length}
      <h2>Recent</h2>
      <ul>
        {#each recent as path (path)}
          <li>
            <button class="flat row" type="button" title={path} onclick={() => onopen(path)}>
              <span class="name">{repoName(path)}</span>
              <span class="mono path">{path}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</main>

<style>
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: center;
    overflow: auto;
    background: var(--bg-editor);
  }
  .column {
    width: min(560px, 100%);
    box-sizing: border-box;
    padding: 64px 24px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }
  h2 {
    margin: 20px 0 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
  }
  .lead {
    margin: 0 0 8px;
    color: var(--text-2);
  }
  .open {
    height: 30px;
    padding: 0 12px;
  }
  form {
    display: flex;
    gap: 8px;
    width: 100%;
  }
  input {
    flex: 1;
    height: var(--h-ctl);
    box-sizing: border-box;
    padding: 0 8px;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--bg-editor);
    color: var(--text);
    font: inherit;
    font-family: var(--font-mono);
    font-size: var(--fs-mono);
  }
  .error {
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--del-text);
    border-radius: var(--radius);
    background: var(--del-bg);
    font-size: var(--fs-ctl);
  }
  ul {
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    width: 100%;
    height: 34px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    padding: 0 8px;
    border-radius: var(--radius);
    text-align: left;
    overflow: hidden;
  }
  .name {
    font-size: var(--fs-ctl);
  }
  .path {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-mono-s);
    color: var(--text-3);
  }
</style>
