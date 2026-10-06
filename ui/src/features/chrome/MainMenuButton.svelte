<script lang="ts">
  import IconButton from "../../lib/ui/IconButton.svelte";
  import Menu from "../../lib/ui/Menu.svelte";
  import Popover from "../../lib/ui/Popover.svelte";
  import { mainMenu } from "../../state/mainMenu.svelte";

  let open = $state(false);
  let button = $state<HTMLButtonElement>();
  // Alt pressed and released on its own focuses the menu, as it does in a Windows or Linux app.
  let altAlone = false;

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Alt") altAlone = !e.repeat ? !(e.ctrlKey || e.shiftKey || e.metaKey) : altAlone;
    else altAlone = false;
  }

  function onkeyup(e: KeyboardEvent) {
    if (e.key !== "Alt" || !altAlone) return;
    altAlone = false;
    e.preventDefault();
    button?.focus();
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

<svelte:window {onkeydown} {onkeyup} onpointerdown={() => (altAlone = false)} onblur={() => (altAlone = false)} />

<!-- The key handler only adds the menu bar's keys to the button inside. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span class="menu" onkeydown={onbuttonkey}>
  <Popover bind:open>
    {#snippet trigger(toggle)}
      <IconButton bind:button icon="menu" label="Main menu" active={open} onclick={toggle} />
    {/snippet}
    <Menu
      items={mainMenu.items}
      onselect={(id) => {
        open = false;
        mainMenu.run(id);
      }}
    />
  </Popover>
</span>

<style>
  .menu {
    display: inline-flex;
  }
</style>
