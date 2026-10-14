// One timer for everything that shows a time as short text ("now", the time of day, the weekday):
// it ticks on the wall clock's minute and runs only while someone listens.

const MINUTE = 60_000;

const listeners = new Set<() => void>();
let timer: ReturnType<typeof setTimeout> | undefined;

function schedule() {
  timer = setTimeout(
    () => {
      for (const listener of [...listeners]) listener();
      schedule();
    },
    MINUTE - (Date.now() % MINUTE),
  );
}

/** Calls `listener` at the start of every minute until the returned function is called. */
export function onMinute(listener: () => void): () => void {
  listeners.add(listener);
  if (timer === undefined) schedule();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  };
}
