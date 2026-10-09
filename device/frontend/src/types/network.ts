export interface InterfaceStatus {
  device: string;
  state: string;
  connection: string | null;
  addresses: string[];
  role: "ap" | "client" | null;
  ssid: string | null;
  /** USB product name of a tethered phone */
  product: string | null;
  /** "auto" (DHCP), "manual" (static), "shared" (our access point) */
  method: string | null;
  gateway: string | null;
}

export interface NetworkStatus {
  wifi: InterfaceStatus;
  ethernet: InterfaceStatus | null;
  tether: InterfaceStatus | null;
  internet_via: string | null;
  /** usbmuxd is installed, which iPhones need */
  iphone_support: boolean;
  /** The ready-made image: YStreamer owns the network, no "System" modes */
  image: boolean;
  fallback_active: boolean;
  /** Why the fallback access point is on, as a sentence */
  fallback_reason: string | null;
  /** The cause is gone; the access point stays on anyway */
  fallback_resolved: boolean;
  applying: boolean;
  last_error: string | null;
}

export interface ScanResult {
  ssid: string;
  signal: number;
  security: string;
  channel: number;
}

export interface WifiCountries {
  /** What the radio follows right now, if any country at all */
  current: string | null;
  countries: { code: string; name: string }[];
}
