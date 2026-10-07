/**
 * The reply if it belongs to manifest `revision`. A reply of another revision is dropped: the
 * views that asked are rebuilt when the new manifest arrives.
 */
export function atRevision<T extends { revision: number }>(
  revision: number | undefined,
  reply: T | undefined,
): T | undefined {
  return reply?.revision === revision ? reply : undefined;
}
