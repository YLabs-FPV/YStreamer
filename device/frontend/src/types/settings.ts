export interface Settings {
  input: {
    mode: import("@/types/input").InputMode;
    uvc: {
      /** /dev/v4l/by-id path, empty for the first one found */
      device: string;
      /** `WxH@fps`, empty for the best available */
      mode: string;
      bitrate_kbps: number;
      keyframe_secs: number;
    };
  };
  device: { hostname: string; timezone: string };
  wifi: {
    mode: WifiMode;
    client: { ssid: string; password: string };
    ap: { ssid: string; password: string; channel: number; address: string };
    fallback_ap: boolean;
    /** Two-letter code; empty leaves the radio's rules to the system */
    country: string;
  };
  ethernet: {
    mode: "system" | "auto" | "static";
    address: string;
    gateway: string;
    dns: string;
  };
  rtsp: {
    enabled: boolean;
    port: number;
    path: string;
    tcp_only: boolean;
    auth: boolean;
    username: string;
    password: string;
  };
  srt: {
    enabled: boolean;
    mode: SrtMode;
    host: string;
    port: number;
    latency_ms: number;
    passphrase: string;
    stream_id: string;
  };
  rtmp: {
    enabled: boolean;
    url: string;
    key: string;
    silent_audio: boolean;
  };
  udp: {
    enabled: boolean;
    format: "rtp" | "mpegts";
    /** host:port list, comma separated */
    destinations: string;
  };
  recording: {
    path: string;
    format: RecordingFormat;
    prefix: string;
    split_minutes: number;
    auto_start: boolean;
    reserve_mb: number;
    /** GPIO numbers, not positions on the header */
    button: {
      enabled: boolean;
      kind: "push" | "switch";
      pin: number;
      led_pin: number | null;
    };
  };
  viewer: { transport: ViewerTransport; max_viewers: number };
  display: {
    hdmi_output: boolean;
    /** `WxH@Hz` from the screen's list, or "auto" */
    mode: string;
    scaling: "fit" | "fill";
    rotate_180: boolean;
    mirror: boolean;
  };
  splash: {
    freeze_last_frame: boolean;
    grayscale_freeze: boolean;
    status_over_image: boolean;
    image_scaling: "fit" | "fill";
    show_ap_password: boolean;
  };
  overlay: {
    enabled: boolean;
    /** Where the logo sits between the left edge (0) and the right (100) */
    x_percent: number;
    /** Same, from the top edge to the bottom */
    y_percent: number;
    /** Logo width, percent of the picture's width */
    size_percent: number;
    opacity_percent: number;
    burn_in: boolean;
  };
  advanced: {
    verbose_logs: boolean;
    terminal: boolean;
    check_for_updates: boolean;
  };
}

export type WifiMode = "unmanaged" | "client" | "ap";

export type RecordingFormat = "mp4" | "ts";

export interface OverlayStatus {
  /** Pixel size of the uploaded logo */
  image: [number, number] | null;
  version: number;
}

export interface OverlayView {
  visible: boolean;
  version: number;
  x_percent: number;
  y_percent: number;
  size_percent: number;
  opacity_percent: number;
}

export type SrtMode = "listener" | "caller";

export interface UdpStatus {
  active: boolean;
  destinations: number;
  error: string | null;
}

export interface RtmpStatus {
  state: "off" | "connecting" | "live" | "retrying";
  error: string | null;
  uptime_secs: number;
  bitrate_kbps: number;
}

export interface DisplayMode {
  id: string;
  width: number;
  height: number;
  refresh: number;
  preferred: boolean;
  fits_screen: boolean;
}

export interface DisplayStatus {
  modes: DisplayMode[];
  current: DisplayMode | null;
  error: string | null;
}

export interface SrtStatus {
  active: boolean;
  clients: number;
  error: string | null;
}

export type ViewerTransport = "websocket" | "webrtc";

export type Section = keyof Settings;

export interface SplashStatus {
  custom_image: boolean;
}

export interface Saved {
  settings: Settings;
  notice: string | null;
}
