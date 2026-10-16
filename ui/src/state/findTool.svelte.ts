// The Find tool window's own state, which outlasts the changeset whose result sets it lists.

class FindTool {
  /** The list shows a preview of the chosen place. */
  preview = $state(true);
  /** Shows the Find tool, and puts the focus in its list if `focus`; set by the window that owns the bottom panel. */
  show: (focus: boolean) => void = () => undefined;
}

export const findTool = new FindTool();
