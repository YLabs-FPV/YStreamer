/** What the aircraft reports about its flight; null when it hasn't lately */
export interface Telemetry {
  battery_percent: number | null;
  /** Whether the latest position from the aircraft was a real fix */
  gps_fix: boolean | null;
  /** The O4 units' satellite count */
  satellites: number | null;
  /** What some units send instead of a count: DJI Fly's 0 to 5 GPS level */
  gps_level: number | null;
  /** Only from `/api/telemetry`; the Watch page doesn't get positions */
  lat?: number | null;
  lon?: number | null;
  /** Height above the takeoff point */
  altitude_m: number | null;
  /** Positive when climbing */
  vertical_speed_mps: number | null;
  horizontal_speed_mps: number | null;
  pitch_deg: number | null;
  roll_deg: number | null;
  yaw_deg: number | null;
  home_set: boolean | null;
  home_lat?: number | null;
  home_lon?: number | null;
  home_altitude_m?: number | null;
  home_distance_m: number | null;
  /** From the aircraft towards home, clockwise from north */
  home_bearing_deg: number | null;
  /** Signal bars, 0 to 5; which is which isn't confirmed */
  rc_signal: number | null;
  vtx_signal: number | null;
}
