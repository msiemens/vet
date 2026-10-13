// Escape in Find in Files closes the scope menu if it is open and the dialog only otherwise. The
// dialog raises `cancel` for the same press unless the key's default is prevented, and whether that
// holds is up to the browser engine, so a cancel that comes with a press that closed the menu is
// absorbed here instead.

export class EscapeGate {
  private absorbing = false;

  /** Escape went down; answers whether it closes the menu, in which case the dialog stays. */
  press(menuOpen: boolean): boolean {
    this.absorbing = menuOpen;
    return menuOpen;
  }

  /** The dialog was cancelled; answers whether it closes. */
  cancel(): boolean {
    return !this.absorbing;
  }

  /** The press is over: whatever cancel it caused has come. */
  settle() {
    this.absorbing = false;
  }
}
