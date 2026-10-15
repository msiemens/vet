// A surface that takes the focus (a dialog, popup, menu, popover) remembers what had it, so that it
// can hand it back when it closes. `focusScope` captures the focused element; `leave` gives it back
// unless it is gone or the focus moved on by itself, and `cancel` ends the scope without giving it
// back, for a press or a choice that took the focus. Scopes nest: only the innermost one still open
// hands the focus back, so an outer surface closing under an inner one does not steal it from it.

/** A surface's hold on the focus it took. */
export interface FocusScope {
  /**
   * Gives the focus back to what had it when the scope was entered. `surface` is the element the
   * focus is in while the scope is open: a focus that is still on the body or inside it is given
   * back, one that moved to something else meanwhile stays where it is.
   */
  leave(surface?: Element): void;
  /** Ends the scope without giving the focus back: a press or the chosen thing took it. */
  cancel(): void;
}

export interface FocusScopeOptions {
  /** Where to give the focus back instead of the element that had it, for a named place like the editor. */
  restore?: () => void;
}

class Scope implements FocusScope {
  private readonly element: HTMLElement | undefined;
  private readonly restore: (() => void) | undefined;
  private closed = false;

  constructor(options: FocusScopeOptions) {
    this.restore = options.restore;
    this.element = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
    open.push(this);
  }

  leave(surface?: Element): void {
    if (this.closed) return;
    this.closed = true;
    const innermost = open.at(-1) === this;
    drop(this);
    if (innermost && this.ours(surface)) this.give();
  }

  cancel(): void {
    if (this.closed) return;
    this.closed = true;
    drop(this);
  }

  /** Whether the focus is still the scope's: nothing took it, or it is still inside the surface. */
  private ours(surface: Element | undefined): boolean {
    const at = document.activeElement;
    if (at === null || at === document.body || at === this.element) return true;
    return surface?.contains(at) ?? false;
  }

  private give(): void {
    if (this.restore) {
      this.restore();
      return;
    }
    if (this.element?.isConnected) this.element.focus({ preventScroll: true });
  }
}

/** The scopes still open, innermost last. */
let open: Scope[] = [];

function drop(scope: Scope): void {
  open = open.filter((s) => s !== scope);
}

/**
 * Enters a focus scope: the focused element is remembered. `leave` gives it back, `cancel` does not,
 * and `restore` names where to give it back instead, for a target like the editor.
 */
export function focusScope(options: FocusScopeOptions = {}): FocusScope {
  return new Scope(options);
}
