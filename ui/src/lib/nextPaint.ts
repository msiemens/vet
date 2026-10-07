/** Resolves after the next frame has been painted (rAF, then a task after it), with the time that frame started. */
export function nextPaint(): Promise<number> {
  return new Promise((resolve) =>
    requestAnimationFrame((frameStart) => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => resolve(frameStart);
      channel.port2.postMessage(null);
    }),
  );
}
