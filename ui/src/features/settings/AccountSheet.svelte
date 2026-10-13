<script lang="ts">
  import type { Account } from "../../generated/Account";
  import type { PlatformKind } from "../../generated/PlatformKind";
  import { accounts } from "../../state/accounts.svelte";
  import { hasKeychain } from "../../state/host";
  import { cliName, GITHUB_HOST } from "./accountRows";
  import Sheet from "./Sheet.svelte";

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
  // An Azure DevOps account is of one organization, which the host names.
  const PLATFORMS: Record<Platform, { label: string; host: string; example: string; kind: PlatformKind }> = {
    github: { label: "GitHub", host: GITHUB_HOST, example: "github.com", kind: "github" },
    ghe: { label: "GitHub Enterprise", host: "", example: "git.example.com", kind: "github" },
    azureDevOps: {
      label: "Azure DevOps",
      host: "dev.azure.com/",
      example: "dev.azure.com/contoso",
      kind: "azureDevOps",
    },
  };

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
  const cliAccount = $derived<Account | undefined>(
    accounts.accounts.find((a) => a.source === "cli" && a.host === host.trim().toLowerCase()),
  );
  const cli = $derived(cliAccount?.state.type === "unusable" ? undefined : cliAccount);
  const tokenOnly = $derived(replace !== undefined || !cli);
  const useCli = $derived(!tokenOnly && (chosen ?? "cli") === "cli");

  function pick(id: Platform) {
    platform = id;
    host = PLATFORMS[id].host;
  }

  async function submit() {
    if (useCli && cli) return ondone(`${cli.host}/cli`);
    busy = true;
    const result = await accounts.addToken(host, token);
    busy = false;
    if (result.ok) ondone(`${result.host}/pat`, result.warning);
    else error = result.error;
  }
</script>

<Sheet title={replace ? "Replace token" : "Add account"} {busy} {onclose} onsubmit={() => void submit()}>
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
      placeholder={PLATFORMS[platform].example}
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
          <span class="hint">{cli ? (cli.user ?? "") : cliAccount ? "unusable" : "not signed in"}</span>
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
  {#if !useCli && !hasKeychain}
    <p class="hint">This window has no keychain, so it is kept until the backend stops.</p>
  {/if}
  {#if error}<p class="problem" role="alert">{error}</p>{/if}
  {#snippet actions()}
    <button class="ctl" type="button" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="ctl primary" type="submit" disabled={busy || !host.trim() || (!useCli && !token.trim())}
      >{busy ? "Checking…" : replace ? "Replace" : "Add"}</button
    >
  {/snippet}
</Sheet>

<style>
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
</style>
