<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { AltTap } from "../../lib/ui/altTap";
  import { commands } from "../../state/commands";
  import { mainMenu } from "../../state/mainMenu.svelte";

  let open = $state(false);
  let button = $state<HTMLButtonElement>();
  // Alt pressed and released on its own opens the menu, or closes it, as it does in a Windows app.
  const alt = new AltTap();

  function onkeydown(e: KeyboardEvent) {
    alt.keydown(e);
  }

  function onkeyup(e: KeyboardEvent) {
    if (!alt.keyup(e)) return;
    e.preventDefault();
    open = !open;
  }

  /** Down arrow opens the focused button's menu; Escape gives the focus back to the page. */
  function onbuttonkey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      open = true;
    } else if (e.key === "Escape" && !open) {
      button?.blur();
    }
  }
</script>

<svelte:window {onkeydown} {onkeyup} onpointerdown={() => alt.cancel()} onblur={() => alt.cancel()} />

<!-- The key handler only adds the menu bar's keys to the button inside. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span class="menu" onkeydown={onbuttonkey}>
  <Popover bind:open>
    {#snippet trigger(toggle)}
      <IconButton bind:button icon="menu" label="Main menu" active={open} onclick={toggle} />
    {/snippet}
    <Menu
      items={mainMenu.items}
      mnemonics
      onselect={(id) => {
        open = false;
        commands.run(id);
      }}
    />
  </Popover>
</span>

<style>
  .menu {
    display: inline-flex;
  }
</style>
