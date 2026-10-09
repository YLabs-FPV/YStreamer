export interface DumlSession {
  registered: boolean;
  attempts: number;
  heartbeats: number;
  devices: LinkedDevice[];
}

export interface LinkedDevice {
  code: string;
  name: string;
  kind: "goggles" | "aircraft" | "remote" | "other" | "unknown";
  address: number;
}

export interface DumlFrame {
  src: number;
  dst: number;
  response: boolean;
  cmd_set: number;
  cmd_id: number;
  count: number;
  changes: number;
  len: number;
  /** Hex, truncated to the first 1 KiB */
  payload: string;
  seen_ms_ago: number;
  changed_ms_ago: number;
}

export interface DumlStatus {
  session: DumlSession;
  frames: DumlFrame[];
}

export interface AircraftStorage {
  total_mb: number;
  free_mb: number;
  /** Recording time that still fits */
  secs_left: number;
}

/** The aircraft's own recording and storage, not YStreamer's */
export interface AircraftRecorder {
  recording: boolean;
  record_secs: number;
  /** The storage being recorded to; all the O3 sends */
  storage: AircraftStorage | null;
  sd: AircraftStorage | null;
  internal: AircraftStorage | null;
  to_internal: boolean | null;
  /** Some units have internal storage only */
  has_sd_slot: boolean;
}
