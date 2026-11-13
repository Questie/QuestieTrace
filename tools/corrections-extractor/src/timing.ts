// Formatting for the run's progress and timing logs.

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, "0")}s`;
  const minutes = Math.round(seconds / 60);
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1e6) return `${bytes} B`;
  if (bytes < 1e9) return `${(bytes / 1e6).toFixed(1)} MB`;
  return `${(bytes / 1e9).toFixed(2)} GB`;
}

/** Runs `run` and logs `<label> in <duration>`. */
export async function timed<T>(log: (message: string) => void, label: string, run: () => T | Promise<T>): Promise<T> {
  const startedAt = performance.now();
  const result = await run();
  log(`${label} in ${formatDuration(performance.now() - startedAt)}`);
  return result;
}
