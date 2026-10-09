export interface Storage {
  path: string;
  writable: boolean;
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
  count: number;
}

export interface RecordingStatus {
  active: boolean;
  elapsed_secs: number;
  bytes: number;
  file: string | null;
  error: string | null;
  storage: Storage;
}

export interface ButtonStatus {
  ready: boolean;
  error: string | null;
  /** Presses since the device started */
  presses: number;
}

export interface RecordingFile {
  name: string;
  size_bytes: number;
  /** Unix seconds */
  modified: number;
}

export interface StorageOption {
  path: string;
  label: string;
  free_bytes: number;
  removable: boolean;
  /** The configured drive, when it isn't plugged in */
  missing: boolean;
}

export interface Transfer {
  from: string;
  to: string;
  moving: boolean;
  state: "running" | "done" | "failed" | "cancelled";
  files_total: number;
  files_done: number;
  skipped: number;
  bytes_total: number;
  bytes_done: number;
  current: string | null;
  error: string | null;
}
