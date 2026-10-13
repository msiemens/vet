<script lang="ts">
  // What is known about the symbol on show: it need not be a changed one. Every location opens its
  // file. The signature and hover are the language server's own answers, the rest is derived
  // (SPEC §7.3, §7.4).
  import type { Side } from "../../generated/Side";
  import { codeBlock } from "../../lib/codeBlock";
  import { splitPath } from "../../lib/paths";
  import { nameOf } from "../../lib/symbolLabel";
  import Icon from "../../lib/ui/Icon.svelte";
  import { findSets } from "../../state/findSets.svelte";
  import type { Shown } from "../../state/symbolLoad";
  import { symbols } from "../../state/symbols.svelte";
  import SymbolGlyph from "./SymbolGlyph.svelte";
  import SymbolRoot from "./SymbolRoot.svelte";
  import { headOf, type OpenLocation } from "./symbolDetail";
  import {
    affectedBody,
    calleesBody,
    callersBody,
    reachBody,
    removalBody,
    type Target,
    usagesBody,
  } from "./symbolTree";

  let { shown, onopen }: { shown: Shown; onopen: OpenLocation } = $props();

  const id = $derived(shown.symbol.id);
  const removed = $derived(shown.change === "removed");
  const where = $derived(headOf(shown.symbol));
  const file = $derived(splitPath(id.path));
  const side = $derived<Side>(removed ? "base" : "target");

  const CALLERS_HINT =
    "Call hierarchy cannot see calls made through interfaces or virtual methods, callbacks, reflection or generated code, so callers and callees may be missing.";
  const AFFECTED_HINT = "Unchanged direct callers; their behaviour may differ now.";
  const FIND = {
    label: "Open all usages in the Find window",
    onclick: () =>
      findSets.findUsages({
        type: "usages",
        symbol: nameOf(id),
        side: "target",
        path: id.path,
        line: shown.symbol.at.line,
        col: shown.symbol.at.col,
      }),
  };

  function go(target: Target) {
    if (target.type === "open") {
      onopen(target.path, target.line, target.side);
      return;
    }
    // A changed symbol listed in the panel: shown in the tree and opened where it is.
    const { symbol, change } = target;
    symbols.choose(symbol, change, true);
    onopen(symbol.id.path, symbol.at.line, change === "removed" ? "base" : "target");
  }
</script>

<div class="head">
  <div class="nm">
    <SymbolGlyph kind={shown.symbol.kind} />
    <span class="nmt selectable" class:gone={removed}>{nameOf(id)}</span>
    {#if shown.change}<span class="chip {shown.change}">{shown.change}</span>{/if}
  </div>
  {#if where.container || where.namespace}
    <div class="hq selectable">
      <span>{where.container}</span>
      {#if where.namespace}<span class="hn" class:sep={where.container}>{where.namespace}</span>{/if}
    </div>
  {/if}
  <button
    class="flat hp"
    type="button"
    title="Open {id.path}"
    onclick={() => onopen(id.path, shown.symbol.at.line, side)}
  >
    <Icon name="file" size={11} /><span class="selectable">{file.dir}<b>{file.name}</b></span>
  </button>
</div>

<SymbolRoot id="signature" title="Signature" load={symbols.signature}>
  {#snippet children(signature)}
    {#if signature.signature}
      <div class="block">
        <code class="mono selectable" use:codeBlock={signature.signature}></code>
      </div>
    {:else}
      <p class="note">{signature.unavailable?.message ?? "The language server has no text for it"}</p>
    {/if}
  {/snippet}
</SymbolRoot>

{#if removed}
  <SymbolRoot id="remaining" title="Remaining references" load={symbols.removal} build={removalBody} ongo={go} />
{:else}
  <SymbolRoot
    id="callers"
    title="Changed callers"
    hint={CALLERS_HINT}
    load={symbols.calls}
    build={(calls) => callersBody(calls)}
    ongo={go}
  />
  <SymbolRoot
    id="callees"
    title="Changed callees"
    load={symbols.calls}
    build={(calls) => calleesBody(calls.callees, calls.gaps)}
    ongo={go}
  />
  {#if shown.change}
    <SymbolRoot
      id="affected"
      title="Possibly affected"
      hint={AFFECTED_HINT}
      warn
      load={symbols.calls}
      build={(calls) => affectedBody(calls.affected, calls.gaps)}
      ongo={go}
    />
  {/if}
  <SymbolRoot id="reach" title="Reached from" load={symbols.reach} build={(r) => reachBody(r.reach)} ongo={go} />
  <SymbolRoot id="usages" title="Usages" load={symbols.usages} build={usagesBody} ongo={go} find={FIND} />
{/if}

<style>
  .head {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px 8px;
    border-bottom: 1px solid var(--line-soft);
  }
  .nm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
  }
  .nmt {
    font-size: 15px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .nmt.gone {
    text-decoration: line-through;
    color: var(--text-2);
  }
  .chip {
    font-size: 11px;
    line-height: 16px;
    padding: 0 6px;
    border-radius: 8px;
    border: 1px solid currentColor;
  }
  .chip.added {
    color: var(--add-text);
  }
  .chip.modified {
    color: var(--old-base);
  }
  .chip.removed {
    color: var(--del-text);
  }
  .hq {
    color: var(--text-2);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .hn {
    color: var(--text-3);
  }
  .hn.sep::before {
    content: "· ";
  }
  .hp {
    display: flex;
    gap: 5px;
    align-items: baseline;
    margin: 2px -4px 0;
    padding: 0 4px;
    color: var(--text-3);
    font-size: 11.5px;
    line-height: 1.4;
    overflow-wrap: anywhere;
    text-align: left;
  }
  .hp :global(svg) {
    flex: none;
    transform: translateY(1.5px);
  }
  .hp b {
    font-weight: 500;
    color: var(--text-2);
  }
  .block {
    margin: 0 12px 8px;
    padding: 6px 8px;
    background: var(--bg-editor);
    border: 1px solid var(--line-soft);
    border-radius: 4px;
  }
  code {
    font-size: var(--fs-mono-s);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
