<script lang="ts">
  import { onMount } from "svelte";
  import type { Account } from "../../generated/Account";
  import type { PlatformKind } from "../../generated/PlatformKind";
  import { accounts } from "../../state/accounts.svelte";
  import { hasKeychain } from "../../state/host";
  import { cliName, GITHUB_HOST } from "./accountRows";

  let {
    replace,
    onclose,
    ondone,
  }: {
    /** The host whose token is replaced; without it the sheet adds an account. */
    replace?: string;
    onclose: () => void;
    /** The account's row key and the keychain's complaint, if it would not keep the token. */
    ondone: (key: string, warning?: string) => void;
  } = $props();

  type Platform = "github" | "ghe" | "azureDevOps";
  const PLATFORMS: Record<Platform, { label: string; host: string; kind: PlatformKind }> = {
    github: { label: "GitHub", host: GITHUB_HOST, kind: "github" },
    ghe: { label: "GitHub Enterprise", host: "", kind: "github" },
    azureDevOps: { label: "Azure DevOps", host: "dev.azure.com", kind: "azureDevOps" },
  };

  let sheet: HTMLDialogElement;
  let platform = $state<Platform>("github");
  // svelte-ignore state_referenced_locally
  let host = $state(replace ?? GITHUB_HOST);
  /** What the viewer picked; until they do, the CLI if there is one. */
  let chosen = $state<"cli" | "token">();
  let token = $state("");
  let busy = $state(false);
  let error = $state<string>();

  const kind = $derived(PLATFORMS[platform].kind);
  /** The CLI's credential for the typed host, if the backend found a signed-in CLI for it. */
  const cli = $derived<Account | undefined>(
    accounts.accounts.find((a) => a.source === "cli" && a.host === host.trim().toLowerCase()),
  );
  const tokenOnly = $derived(replace !== undefined || !cli);
  const useCli = $derived(!tokenOnly && (chosen ?? "cli") === "cli");

  onMount(() => {
    sheet.showModal();
    sheet.querySelector<HTMLElement>("input:not([type=radio]):not([disabled]), select")?.focus();
  });

  function pick(id: Platform) {
    platform = id;
    host = PLATFORMS[id].host;
  }

  async function submit() {
    if (useCli && cli) return ondone(`${cli.host}/cli`);
    busy = true;
    const result = await accounts.addToken(host, token);
    busy = false;
    if (result.ok) ondone(`${host.trim().toLowerCase()}/pat`, result.warning);
    else error = result.error;
  }
</script>

<!-- A modal of its own, so that Escape and Tab belong to the sheet and not to the dialog under it. -->
<dialog
  class="surface"
  aria-label={replace ? "Replace token" : "Add account"}
  bind:this={sheet}
  oncancel={(e) => {
    e.preventDefault();
    if (!busy) onclose();
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <h3>{replace ? "Replace token" : "Add account"}</h3>
    {#if !replace}
      <label>
        <span>Platform</span>
        <select class="field" value={platform} onchange={(e) => pick(e.currentTarget.value as Platform)}>
          {#each Object.entries(PLATFORMS) as [id, p] (id)}<option value={id}>{p.label}</option>{/each}
        </select>
      </label>
    {/if}
    <label>
      <span>Host</span>
      <input
        class="field mono"
        type="text"
        placeholder="git.example.com"
        autocomplete="off"
        spellcheck="false"
        disabled={replace !== undefined}
        bind:value={host}
      />
    </label>
    <fieldset>
      <legend>{replace ? "New token" : "Sign in with"}</legend>
      <div class="choices">
        {#if !replace}
          <label class="choice" class:off={!cli}>
            <input type="radio" name="method" disabled={!cli} checked={useCli} onchange={() => (chosen = "cli")} />
            <span>{cliName(kind)}</span>
            <span class="hint">{cli ? (cli.user ?? "") : "not signed in"}</span>
          </label>
          <label class="choice">
            <input type="radio" name="method" checked={!useCli} onchange={() => (chosen = "token")} />
            <span>Token</span>
          </label>
        {/if}
        <input
          class="field mono"
          type="password"
          placeholder="Personal access token"
          aria-label="Personal access token"
          autocomplete="off"
          spellcheck="false"
          disabled={useCli}
          bind:value={token}
        />
      </div>
    </fieldset>
    {#if !useCli}
      <p class="hint">
        The platform checks the token before Vet keeps it. {hasKeychain
          ? "It is kept in the system keychain, never in Vet's own data."
          : "This window has no keychain, so it is kept until the backend stops."}
      </p>
    {/if}
    {#if error}<p class="problem" role="alert">{error}</p>{/if}
    <div class="actions">
      <button class="ctl" type="button" disabled={busy} onclick={onclose}>Cancel</button>
      <button class="ctl primary" type="submit" disabled={busy || !host.trim() || (!useCli && !token.trim())}
        >{busy ? "Checking…" : replace ? "Replace" : "Add"}</button
      >
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 420px;
    max-width: calc(100vw - 32px);
    margin: 6vh auto auto;
    padding: 16px;
    color: inherit;
    font-size: var(--fs-ctl);
  }
  dialog::backdrop {
    background: transparent;
  }
  dialog:focus-visible {
    outline: none;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: var(--fs);
    font-weight: 600;
  }
  label,
  fieldset {
    display: grid;
    grid-template-columns: 92px 1fr;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 0;
    border: 0;
    color: var(--text-2);
  }
  fieldset {
    align-items: start;
  }
  legend {
    float: left;
    width: 92px;
    padding: 0;
    padding-top: 2px;
  }
  .choices {
    grid-column: 2;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .choice.off {
    color: var(--text-3);
  }
  .hint {
    color: var(--text-3);
    font-size: 12px;
  }
  p {
    margin: 0;
  }
  .problem {
    color: var(--del-text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
