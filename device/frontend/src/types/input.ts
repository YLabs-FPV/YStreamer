export type InputMode = "dji_fpv" | "uvc" | "dji_fpv_legacy" | "rtmp";

export type UvcFormat = "h264" | "mjpeg" | "raw";

export interface UvcMode {
  /** `WxH@fps`, what settings store */
  id: string;
  width: number;
  height: number;
  fps: number;
  format: UvcFormat;
}

export interface UvcDevice {
  /** Stable /dev/v4l/by-id path */
  path: string;
  name: string;
  modes: UvcMode[];
}

export interface UvcStatus {
  state: "idle" | "no_device" | "starting" | "live" | "error";
  device: string | null;
  mode: UvcMode | null;
  /** The format in use; may not be the cheapest the device offers */
  format: UvcFormat | null;
  encoder: "passthrough" | "hardware" | "software" | null;
  error: string | null;
}

/** DJI Goggles V1 and V2: video only, experimental */
export interface LegacyStatus {
  state: "idle" | "no_device" | "waiting" | "live" | "error";
  error: string | null;
  /** Bytes of the raw stream saved so far, while or after capturing */
  captured: number | null;
}

/** Pushed to the device by OBS, a phone app or DJI Fly */
export interface RtmpInputStatus {
  state: "idle" | "waiting" | "connected" | "live" | "error";
  /** The port listened on */
  port: number | null;
  /** Address of the app streaming */
  publisher: string | null;
  /** The stream has B-frames, which add delay */
  reorders: boolean;
  /** Why the port couldn't be opened, or why the last app was turned away */
  error: string | null;
}

export interface InputStatus {
  mode: InputMode;
  uvc: UvcStatus;
  legacy: LegacyStatus;
  rtmp: RtmpInputStatus;
}
