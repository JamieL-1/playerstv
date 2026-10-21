/** "1:02:03" or "12:34". */
export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${pad(m)}:${pad(sec)}`;
}

/** "4.3 GB", "850 MB" (decimal units, like the Rust side). */
export function formatSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${Math.floor(bytes / 1e6)} MB`;
}

export function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}
