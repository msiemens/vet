// Measurement helpers and result formatting for SPEC §12.0.

/** Resolves after the next frame has been painted (rAF, then a task after it). */
export function nextPaint(): Promise<void> {
  return new Promise((resolve) =>
    requestAnimationFrame(() => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => resolve();
      channel.port2.postMessage(null);
    }),
  );
}

export function percentile95(samples: number[]): number {
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(sorted.length * 0.95) - 1)];
}

export interface Row {
  metric: string;
  p95: number | null;
  unit: string;
  threshold: number;
  cmp: "<" | "<=";
  n: number;
  /** null: informational, or could not be measured here. */
  pass: boolean | null;
  /** "diagnostic" marks an informational row. */
  note?: string;
}

export function row(metric: string, samples: number[], unit: string, threshold: number, cmp: "<" | "<=" = "<", note?: string): Row {
  const p95 = percentile95(samples);
  return { metric, p95, unit, threshold, cmp, n: samples.length, pass: cmp === "<" ? p95 < threshold : p95 <= threshold, note };
}

/** Rows with unit "" are pass/fail checks without a number. */
export function cells(r: Row): [string, string, string, string, string] {
  const check = r.unit === "";
  const info = r.note === "diagnostic";
  const value = r.p95 === null ? "-" : `${r.p95.toFixed(2)} ${r.unit}`;
  const threshold = check || info ? "-" : `${r.cmp} ${r.threshold} ${r.unit}`;
  const verdict = info ? "INFO" : r.pass === null ? "MANUAL" : r.pass ? "PASS" : "FAIL";
  return [r.metric, value, threshold, String(r.n), r.note && !info ? `${verdict} (${r.note})` : verdict];
}

export function formatTable(rows: Row[]): string {
  const lines = ["| Metric | p95 | Threshold | n | Result |", "|---|---|---|---|---|"];
  for (const r of rows) lines.push(`| ${cells(r).join(" | ")} |`);
  return lines.join("\n");
}
