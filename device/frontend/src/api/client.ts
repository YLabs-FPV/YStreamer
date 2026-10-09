import type {
  Saved,
  Section,
  Settings,
  SplashStatus,
  SrtStatus,
  OverlayStatus,
  RtmpStatus,
  UdpStatus,
  DisplayStatus,
} from "@/types/settings";
import type { NetworkStatus, ScanResult, WifiCountries } from "@/types/network";
import type { TailscaleStatus } from "@/types/tailscale";
import type { MetricsSnapshot } from "@/types/metrics";
import type { WireguardStatus } from "@/types/wireguard";
import type {
  SshStatus,
  SystemAction,
  SystemInfo,
  Release,
  UpdateStatus,
} from "@/types/system";
import type { DumlStatus } from "@/types/duml";
import type { AuthStatus } from "@/types/auth";
import type { InputStatus, UvcDevice } from "@/types/input";
import type {
  ButtonStatus,
  RecordingFile,
  RecordingStatus,
  StorageOption,
  Transfer,
} from "@/types/recording";

let onUnauthorized: () => void = () => {};

export function setUnauthorizedHandler(handler: () => void) {
  onUnauthorized = handler;
}

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  let res: Response;
  const raw = body instanceof Blob;
  try {
    res = await fetch(`/api${path}`, {
      method,
      headers:
        body === undefined
          ? undefined
          : { "Content-Type": raw ? body.type : "application/json" },
      body: body === undefined ? undefined : raw ? body : JSON.stringify(body),
    });
  } catch {
    throw new Error("Can't reach the device");
  }
  const type = res.headers.get("content-type") ?? "";
  const data = type.includes("json") ? await res.json() : await res.text();
  if (res.status === 401 && !path.startsWith("/auth")) onUnauthorized();
  if (!res.ok) {
    const fallback = `Request failed (${res.status}${res.statusText ? ` ${res.statusText}` : ""})`;
    const msg =
      typeof data === "object" && data?.error
        ? data.error
        : typeof data === "string" && !data.trimStart().startsWith("<")
          ? data
          : "";
    throw new Error(String(msg || fallback));
  }
  return data as T;
}

export function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const at = (location?: string | null) =>
  location ? `?location=${encodeURIComponent(location)}` : "";

export const api = {
  auth: () => request<AuthStatus>("GET", "/auth"),
  login: (password: string) =>
    request<void>("POST", "/auth/login", { password }),
  logout: () => request<void>("POST", "/auth/logout"),
  setPassword: (current: string, next: string | null) =>
    request<void>("PUT", "/auth/password", { current, new: next }),
  setProtectViewing: (protect: boolean) =>
    request<void>("PUT", "/auth/viewing", { protect }),
  settings: () => request<Settings>("GET", "/settings"),
  saveSection: <K extends Section>(section: K, value: Settings[K]) =>
    request<Saved>("PUT", `/settings/${section}`, value),
  replaceAll: (value: unknown) => request<Saved>("PUT", "/settings", value),
  reset: () => request<Saved>("POST", "/settings/reset"),
  network: () => request<NetworkStatus>("GET", "/network"),
  scan: () => request<ScanResult[]>("GET", "/network/scan"),
  countries: () => request<WifiCountries>("GET", "/network/countries"),
  wireguard: () => request<WireguardStatus>("GET", "/wireguard"),
  wireguardImport: (conf: Blob) =>
    request<WireguardStatus>("PUT", "/wireguard/config", conf),
  wireguardAction: (action: "connect" | "disconnect") =>
    request<WireguardStatus>("POST", `/wireguard/${action}`),
  wireguardRemove: () => request<WireguardStatus>("DELETE", "/wireguard"),
  terminal: () =>
    request<{ refused: string | null; users: string[] }>("GET", "/terminal"),
  metrics: (after: number) =>
    request<MetricsSnapshot>("GET", `/metrics?after=${after}`),
  tailscale: () => request<TailscaleStatus>("GET", "/tailscale"),
  tailscaleAction: (action: "connect" | "disconnect" | "logout") =>
    request<TailscaleStatus>("POST", `/tailscale/${action}`),
  system: () => request<SystemInfo>("GET", "/system"),
  logs: (lines = 300) => request<string>("GET", `/system/logs?lines=${lines}`),
  action: (action: SystemAction) => request<void>("POST", `/system/${action}`),
  update: () => request<UpdateStatus>("GET", "/system/update"),
  uploadUpdate: (deb: Blob, signature: string) =>
    request<UpdateStatus>(
      "PUT",
      `/system/update/package?signature=${encodeURIComponent(signature)}`,
      deb,
    ),
  discardUpdate: () =>
    request<UpdateStatus>("DELETE", "/system/update/package"),
  checkUpdate: () => request<Release>("POST", "/system/update/check"),
  downloadUpdate: () =>
    request<UpdateStatus>("POST", "/system/update/download"),
  installUpdate: () => request<void>("POST", "/system/update/install"),
  rollBackUpdate: () => request<void>("POST", "/system/update/rollback"),
  ssh: () => request<SshStatus>("GET", "/system/ssh"),
  setSsh: (enabled: boolean) =>
    request<SshStatus>("PUT", "/system/ssh", { enabled }),
  setSshPassword: (password: string) =>
    request<SshStatus>("PUT", "/system/ssh/password", { password }),
  recording: () => request<RecordingStatus>("GET", "/recording"),
  recordStart: () => request<RecordingStatus>("POST", "/recording/start"),
  recordStop: () => request<RecordingStatus>("POST", "/recording/stop"),
  recordButton: () => request<ButtonStatus>("GET", "/recording/button"),
  /** `location` is one of the storage options; left out, where recordings go */
  recordings: (location?: string | null) =>
    request<RecordingFile[]>("GET", `/recording/files${at(location)}`),
  deleteRecording: (name: string, location?: string | null) =>
    request<void>(
      "DELETE",
      `/recording/files/${encodeURIComponent(name)}${at(location)}`,
    ),
  transfer: () => request<Transfer | null>("GET", "/recording/transfer"),
  startTransfer: (job: {
    from: string;
    to: string;
    files?: string[];
    move?: boolean;
  }) => request<Transfer | null>("POST", "/recording/transfer", job),
  cancelTransfer: () =>
    request<Transfer | null>("DELETE", "/recording/transfer"),
  ejectDrive: (path: string) =>
    request<void>("POST", "/recording/eject", { path }),
  storageOptions: () =>
    request<StorageOption[]>("GET", "/recording/storage-options"),
  srt: () => request<SrtStatus>("GET", "/srt"),
  rtmp: () => request<RtmpStatus>("GET", "/rtmp"),
  udp: () => request<UdpStatus>("GET", "/udp"),
  display: () => request<DisplayStatus>("GET", "/display"),
  input: () => request<InputStatus>("GET", "/input"),
  /** Probes every device; not for polling */
  inputDevices: () => request<UvcDevice[]>("GET", "/input/devices"),
  startLegacyCapture: () => request<void>("POST", "/input/legacy/capture"),
  legacyCaptureUrl: "/api/input/legacy/capture",
  duml: () => request<DumlStatus>("GET", "/duml"),
  clearDuml: () => request<void>("DELETE", "/duml"),
  sendDuml: (frame: {
    dst: number;
    cmd_set: number;
    cmd_id: number;
    payload: string;
  }) => request<void>("POST", "/duml/send", frame),
  overlay: () => request<OverlayStatus>("GET", "/overlay"),
  uploadOverlay: (png: Blob) => request<void>("PUT", "/overlay/image", png),
  removeOverlay: () => request<void>("DELETE", "/overlay/image"),
  /** Served to anyone allowed to watch; `v` busts the browser cache */
  overlayUrl: (v: number) => `/api/overlay.png?v=${v}`,
  splash: () => request<SplashStatus>("GET", "/splash"),
  uploadSplash: (image: Blob) => request<void>("PUT", "/splash/image", image),
  removeSplash: () => request<void>("DELETE", "/splash/image"),
  /** What shows while there's no video; `v` busts the browser cache */
  splashPreviewUrl: (
    v: number,
    statusOverImage: boolean,
    imageScaling: string,
  ) =>
    `/api/splash/preview?v=${v}&status_over_image=${statusOverImage}&image_scaling=${imageScaling}`,
  recordingUrl: (name: string, location?: string | null) =>
    `/api/recording/files/${encodeURIComponent(name)}${at(location)}`,
};
