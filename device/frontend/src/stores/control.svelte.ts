import { recording } from "@/stores/recording.svelte";
import type { AircraftRecorder, LinkedDevice } from "@/types/duml";
import type { InputMode, LegacyStatus, UvcStatus } from "@/types/input";
import type { OverlayView } from "@/types/settings";
import { snackbar } from "@/stores/snackbar.svelte";

// The camera takes a moment to apply a change and keeps reporting the old
// value meanwhile; a new resolution or frame rate restarts its pipeline
const SETTLE_MS = 1000;
const FORMAT_SETTLE_MS = 3000;
// A different value long after the change is someone using the goggles'
// own menu, not a change that failed
const VERDICT_MS = 5000;

/** Message key, the field it lands in, and what to call it */
const CAMERA_FIELDS = [
  ["iso", "iso", "ISO"],
  ["shutter", "shutter", "shutter"],
  ["ev", "ev", "exposure compensation"],
  ["ev_metered", "evMetered", ""],
  ["exposure_mode", "exposureMode", "exposure mode"],
  ["wb_auto", "wbAuto", "white balance"],
  ["wb_temp", "wbTemp", "white balance"],
  ["res", "res", "video format"],
  ["ar", "ar", "video format"],
  ["fps", "fps", "video format"],
  ["sharpness", "sharpness", "sharpness"],
  ["noise_reduction", "noiseReduction", "noise reduction"],
  ["color_profile", "colorProfile", "color profile"],
  ["anti_flicker", "antiFlicker", "anti-flicker"],
] as const;

type CameraKey = (typeof CAMERA_FIELDS)[number][0];

const same = (a: unknown, b: unknown) =>
  typeof a === "number" && typeof b === "number"
    ? Math.abs(a - b) < 0.05
    : a === b;

export type LinkState = "unplugged" | "connecting" | "live";

class ControlStore {
  connected = $state(false);
  link = $state<LinkState | null>(null);
  devices = $state<LinkedDevice[]>([]);
  inputMode = $state<InputMode>("dji_fpv");
  uvc = $state<UvcStatus | null>(null);
  legacy = $state<LegacyStatus | null>(null);
  overlay = $state<OverlayView | null>(null);

  goggles = $derived(this.devices.find((d) => d.kind === "goggles") ?? null);
  aircraft = $derived(this.devices.find((d) => d.kind === "aircraft") ?? null);

  iso = $state<number | null>(null);
  shutter = $state<number | null>(null);
  ev = $state<number | null>(null);
  evMetered = $state<number | null>(null);
  exposureMode = $state<string | null>(null);
  wbAuto = $state<boolean | null>(null);
  wbTemp = $state<number | null>(null);
  res = $state<string | null>(null);
  ar = $state<string | null>(null);
  fps = $state<number | null>(null);
  sharpness = $state<number | null>(null);
  noiseReduction = $state<number | null>(null);
  colorProfile = $state<string | null>(null);
  antiFlicker = $state<string | null>(null);
  gogglesBattery = $state<number | null>(null);
  linkQuality = $state<number | null>(null);
  aircraftRecorder = $state<AircraftRecorder | null>(null);

  /** Values just sent to the camera, and until when to hold on to them */
  #pending = new Map<CameraKey, { value: unknown; until: number }>();

  #ws: WebSocket | null = null;
  #retry: ReturnType<typeof setTimeout> | null = null;

  connect() {
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${proto}://${location.host}/api/control`);
    this.#ws = ws;

    ws.onopen = () => {
      this.connected = true;
    };
    ws.onclose = () => {
      // Replaced on purpose, e.g. reconnected after logging in
      if (this.#ws !== ws) return;
      this.connected = false;
      this.link = null;
      this.devices = [];
      this.gogglesBattery = null;
      this.aircraftRecorder = null;
      this.linkQuality = null;
      this.#scheduleReconnect();
    };
    ws.onerror = () => ws.close();
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg.type === "state") this.#merge(msg);
        else if (msg.type === "recording") recording.apply(msg.recording);
        else if (msg.type === "link") {
          this.link = msg.state;
          if (msg.state !== "live") {
            this.gogglesBattery = null;
            this.aircraftRecorder = null;
            this.linkQuality = null;
          }
        } else if (msg.type === "devices") {
          this.devices = msg.devices;
          if (!this.aircraft) {
            this.aircraftRecorder = null;
            this.linkQuality = null;
          }
        } else if (msg.type === "overlay") this.overlay = msg;
        else if (msg.type === "input") {
          this.inputMode = msg.mode;
          this.uvc = msg.uvc;
          this.legacy = msg.legacy;
        }
      } catch {
        // ignore malformed frames
      }
    };
  }

  disconnect() {
    if (this.#retry) clearTimeout(this.#retry);
    this.#retry = null;
    this.#ws?.close();
    this.#ws = null;
  }

  #scheduleReconnect() {
    if (this.#retry) return;
    this.#retry = setTimeout(() => {
      this.#retry = null;
      this.connect();
    }, 1500);
  }

  #merge(msg: Record<string, unknown>) {
    for (const [key, field, label] of CAMERA_FIELDS) {
      if (!(key in msg)) continue;
      if (this.#stillSettling(key, msg[key], label)) continue;
      (this as Record<string, unknown>)[field] = msg[key];
    }
    if ("goggles_battery" in msg)
      this.gogglesBattery = msg.goggles_battery as number;
    if ("link_quality" in msg) this.linkQuality = msg.link_quality as number;
    if ("aircraft_recorder" in msg)
      this.aircraftRecorder = msg.aircraft_recorder as AircraftRecorder;
  }

  #stillSettling(key: CameraKey, reported: unknown, label: string): boolean {
    const sent = this.#pending.get(key);
    if (!sent) return false;
    if (same(reported, sent.value)) {
      this.#pending.delete(key);
      return false;
    }
    const now = performance.now();
    if (now < sent.until) return true;
    if (now > sent.until + VERDICT_MS) {
      this.#pending.delete(key);
      return false;
    }
    // One message for a change that set several fields at once
    for (const [k, , l] of CAMERA_FIELDS)
      if (l === label) this.#pending.delete(k);
    snackbar.show(`The camera didn't take the new ${label}`, true);
    return false;
  }

  #expect(values: Partial<Record<CameraKey, unknown>>, settle = SETTLE_MS) {
    const until = performance.now() + settle;
    for (const [key, value] of Object.entries(values))
      this.#pending.set(key as CameraKey, { value, until });
  }

  #send(obj: Record<string, unknown>) {
    if (this.#ws && this.#ws.readyState === WebSocket.OPEN) {
      this.#ws.send(JSON.stringify(obj));
    }
  }

  setIso(value: number) {
    this.iso = value;
    this.#expect({ iso: value });
    this.#send({ cmd: "set_iso", value });
  }
  setShutter(value: number) {
    this.shutter = value;
    this.#expect({ shutter: value });
    this.#send({ cmd: "set_shutter", value });
  }
  setEv(value: number) {
    this.ev = value;
    this.#expect({ ev: value });
    this.#send({ cmd: "set_ev", value });
  }
  setExposureMode(value: "auto" | "manual") {
    this.exposureMode = value;
    this.#expect({ exposure_mode: value });
    this.#send({ cmd: "set_exposure_mode", value });
  }
  setWb(auto: boolean, temp = 5000) {
    this.wbAuto = auto;
    if (!auto) this.wbTemp = temp;
    this.#expect(auto ? { wb_auto: true } : { wb_auto: false, wb_temp: temp });
    this.#send({ cmd: "set_wb", auto, temp });
  }
  setVideoFormat(res: string, ar: string, fps: number) {
    this.res = res;
    this.ar = ar;
    this.fps = fps;
    this.#expect({ res, ar, fps }, FORMAT_SETTLE_MS);
    this.#send({ cmd: "set_video_format", res, ar, fps });
  }
  setSharpness(value: number) {
    this.sharpness = value;
    this.#expect({ sharpness: value });
    this.#send({ cmd: "set_sharpness", value });
  }
  setNr(value: number) {
    this.noiseReduction = value;
    this.#expect({ noise_reduction: value });
    this.#send({ cmd: "set_nr", value });
  }
  setColorProfile(value: string) {
    this.colorProfile = value;
    this.#expect({ color_profile: value });
    this.#send({ cmd: "set_color_profile", value });
  }
  setAntiFlicker(value: string) {
    this.antiFlicker = value;
    this.#expect({ anti_flicker: value });
    this.#send({ cmd: "set_anti_flicker", value });
  }
}

export const control = new ControlStore();
