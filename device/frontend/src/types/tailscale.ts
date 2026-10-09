export type TailscaleState =
  | "not_installed"
  | "service_down"
  | "needs_login"
  | "needs_approval"
  | "disconnected"
  | "connecting"
  | "connected";

export interface TailscalePeer {
  name: string;
  direct: boolean;
  relay: string | null;
}

export interface TailscaleStatus {
  state: TailscaleState;
  login_url: string | null;
  name: string | null;
  addresses: string[];
  network: string | null;
  peers: TailscalePeer[];
  problems: string[];
}
