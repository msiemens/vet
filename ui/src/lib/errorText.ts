/** The message of something a request rejected with. */
export const errorText = (error: unknown): string => (error instanceof Error ? error.message : String(error));
