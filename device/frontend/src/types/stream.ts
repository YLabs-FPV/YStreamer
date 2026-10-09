export type Status =
  | "connecting"
  | "negotiating"
  | "connected"
  | "playing"
  | "busy"
  | "disconnected"
  | "error";

export interface PlayerHistory {
  t: number[];
  kbps: (number | null)[];
  bufferMs: (number | null)[];
}

export interface PlayerStat {
  k: string;
  label: string;
  v: string;
}
