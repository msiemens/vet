// The Find tool window's own state, which outlasts the changeset whose result sets it lists: whether the list shows a preview.

class FindTool {
  /** The list shows a preview of the chosen place. */
  preview = $state(true);
}

export const findTool = new FindTool();
