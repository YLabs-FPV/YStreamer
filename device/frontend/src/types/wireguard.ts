export interface WireguardPeer {
  endpoint: string | null;
  allowed_ips: string[];
  /** Seconds since the server last answered; null if it never has */
  handshake_age_secs: number | null;
  rx_bytes: number;
  tx_bytes: number;
  keepalive: boolean;
}

export interface WireguardStatus {
  /** "none": nothing imported yet */
  state: "none" | "off" | "on";
  address: string | null;
  peers: WireguardPeer[];
}
