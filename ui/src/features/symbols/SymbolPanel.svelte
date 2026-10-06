<script module lang="ts">
  export interface SymbolInfo {
    name: string;
    qualified: string;
    signature: string;
    doc?: string;
    callers: { name: string; at: string }[];
    /** Entry point first, the symbol last. */
    reachedFrom: string[];
    /** Call sites of a removed symbol, classified in the target (SPEC §7.3). */
    removed?: {
      name: string;
      sites: { at: string; verdict: "unresolved" | "binding changed" | "gone" | "unknown"; detail?: string }[];
    };
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../../lib/ui/Icon.svelte";
  import IconButton from "../../lib/ui/IconButton.svelte";
  import { mockSymbol } from "./mock";

  let { symbol = mockSymbol }: { symbol?: SymbolInfo } = $props();

  const VERDICT = {
    unresolved: "var(--delt)",
    "binding changed": "var(--old)",
    gone: "var(--tx3)",
    unknown: "var(--tx3)",
  } as const;
</script>

{#snippet section(title: string, count: number | undefined, derived: boolean, body: Snippet)}
  <div class="sh">
    <Icon name="open" size={10} />{title}
    {#if count !== undefined}<span class="mono n">{count}</span>{/if}
    <span class="grow"></span>
    {#if derived}<IconButton icon="derived" label="Derived — show inputs" size={20} iconSize={13} />{/if}
  </div>
  {@render body()}
{/snippet}

{#snippet signature()}
  <div class="block">
    <code class="mono selectable">{symbol.signature}</code>
    {#if symbol.doc}<span class="doc selectable">{symbol.doc}</span>{/if}
  </div>
{/snippet}

{#snippet callers()}
  <ul>
    {#each symbol.callers as c (c.name + c.at)}<li class="tw">
        <span class="mono s">{c.name}</span><span class="grow"></span><span class="mono at">{c.at}</span>
      </li>{/each}
  </ul>
{/snippet}

{#snippet reached()}
  <ol>
    {#each symbol.reachedFrom as step, i (i)}
      <li class="tw mono s" style:padding-left="{26 + i * 10}px" class:last={i === symbol.reachedFrom.length - 1}>
        {#if i > 0}<span class="at">└</span>{/if}{step}
      </li>
    {/each}
  </ol>
{/snippet}

{#snippet removed()}
  <ul>
    {#each symbol.removed?.sites ?? [] as site (site.at)}
      <li class="tw">
        <span class="mono s">{site.at}</span><span class="grow"></span>
        <span class="verdict" style:color={VERDICT[site.verdict]}>{site.verdict}</span>
        {#if site.detail}<span class="mono at">{site.detail}</span>{/if}
      </li>
    {/each}
  </ul>
{/snippet}

<aside aria-label="Symbol">
  <div class="head">
    <span class="mono name selectable">{symbol.name}</span>
    <span class="mono qualified selectable">{symbol.qualified}</span>
  </div>
  {@render section("Signature", undefined, false, signature)}
  {@render section("Callers", symbol.callers.length, true, callers)}
  {@render section("Reached from", undefined, true, reached)}
  {#if symbol.removed}
    {@render section(symbol.removed.name, symbol.removed.sites.length, true, removed)}
  {/if}
</aside>

<style>
  aside {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden auto;
  }
  .head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--ln2);
  }
  .name {
    font-size: var(--fs-ctl);
    font-weight: 500;
  }
  .qualified {
    font-size: var(--fs-mono-s);
    color: var(--tx2);
  }
  .sh {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-ctl);
    padding: 0 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--tx2);
    flex: none;
  }
  .n {
    font-weight: 400;
    color: var(--tx3);
  }
  .grow {
    flex: 1;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 10px 10px 26px;
    border-bottom: 1px solid var(--ln2);
  }
  code {
    font-size: var(--fs-mono-s);
    white-space: pre-wrap;
  }
  .doc {
    font-size: var(--fs-ctl);
    color: var(--tx2);
  }
  ul,
  ol {
    margin: 0;
    padding: 0 0 6px;
    list-style: none;
    border-bottom: 1px solid var(--ln2);
  }
  .tw {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--h-row);
    padding: 0 10px 0 26px;
    white-space: nowrap;
    font-size: var(--fs-ctl);
  }
  .s {
    font-size: var(--fs-mono-s);
  }
  .at {
    font-size: 11px;
    color: var(--tx3);
  }
  .last {
    font-weight: 500;
  }
  .verdict {
    font-size: 12px;
  }
</style>
