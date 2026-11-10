export type InputMode = "dji_fpv" | "uvc" | "dji_fpv_legacy";

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
  /** What to check when the goggles were found but send no video */
  hint: string | null;
  /** Bytes of the raw stream saved so far, while or after capturing */
  captured: number | null;
}

/** DJI Fly on an Android phone in a USB-A port */
export interface PassthroughStatus {
  state: "off" | "no_phone" | "phone" | "dji_fly";
  error: string | null;
  /** Bytes since the service started */
  to_phone: number;
  from_phone: number;
  dropped: number;
  /** What DJI Fly sent that the goggles had no room for */
  lost: number;
}

/** The USB-C link: goggles send H.264, an RC-N remote H.265 */
export interface DjiStatus {
  /** What the link carries, once worked out */
  codec: "h264" | "h265" | null;
  /** How H.265 is turned into H.264, while it is */
  converter: "hardware" | null;
  error: string | null;
}

export interface InputStatus {
  mode: InputMode;
  dji: DjiStatus;
  uvc: UvcStatus;
  legacy: LegacyStatus;
  passthrough: PassthroughStatus;
}
