export interface MetricsSnapshot {
  /** Number of the newest sample; passed back as `after` for only newer ones */
  seq: number;
  /** Number of the first sample in this answer */
  first: number;
  interval_ms: number;
  /** How many samples the device keeps */
  history: number;
  /** When each sample was taken, Unix milliseconds */
  t: number[];
  /** One value per entry of `t`; null where there was no reading */
  series: Record<string, (number | null)[]>;
}
