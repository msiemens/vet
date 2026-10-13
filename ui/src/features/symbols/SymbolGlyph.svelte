<script lang="ts">
  // What a symbol is, in one small box: C for a type, ƒ for a function, {} for a module. A symbol
  // of another kind (a field, a constant) has an empty slot, so names stay aligned.
  import type { SymbolKind } from "../../generated/SymbolKind";

  let { kind }: { kind: SymbolKind } = $props();

  const GLYPH: Record<SymbolKind, string> = { type: "C", function: "ƒ", module: "{}", other: "" };
</script>

<span class="kind {kind}" aria-hidden="true">{GLYPH[kind]}</span>

<style>
  .kind {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 3px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font: 500 9.5px/1 var(--font-mono);
    border: 1px solid currentColor;
    box-sizing: border-box;
  }
  .type {
    color: var(--syn-type);
  }
  .function {
    color: var(--syn-function);
  }
  .module {
    color: var(--note);
  }
  .other {
    border-color: transparent;
  }
</style>
