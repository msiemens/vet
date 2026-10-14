/**
 * The load held in `slot`, started by `start` if there is none. A load that fails is handed to
 * `failed` and forgotten, so the next call tries again instead of answering the old failure.
 */
export function loadOnce<T>(
  slot: { loading?: Promise<T> },
  start: () => Promise<T>,
  failed: (error: unknown) => void,
): Promise<T> {
  slot.loading ??= start().catch((error: unknown) => {
    slot.loading = undefined;
    failed(error);
    throw error;
  });
  return slot.loading;
}
