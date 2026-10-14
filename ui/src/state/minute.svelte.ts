import { createSubscriber } from "svelte/reactivity";
import { onMinute } from "../lib/minute";

const subscribe = createSubscriber((update) => onMinute(update));

/**
 * The time now. Whatever reads `now` in a reactive context (a template, a `$derived`) is run again
 * every minute, so the short times it shows (`shortTime`) stay right.
 */
export const minute = {
  get now(): number {
    subscribe();
    return Date.now();
  },
};
