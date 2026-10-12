// The popup that lists the places Show Usages and Go to Implementation found: what it shows, read
// by `features/locations/LocationPicker.svelte`.

import type { PickerSink, PickerView } from "./pickerTypes";

class LocationPickerStore implements PickerSink {
  view = $state.raw<PickerView>();

  show(view: PickerView) {
    this.view = view;
  }

  close() {
    this.view = undefined;
  }
}

export const locationPicker = new LocationPickerStore();
