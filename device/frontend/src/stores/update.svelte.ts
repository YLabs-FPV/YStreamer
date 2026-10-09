import { api } from "@/api/client";
import type { Release } from "@/types/system";

const REFRESH_MS = 30 * 60 * 1000;

class Update {
  /** A newer version the device has found, for the sidebar to point at */
  available = $state<Release | null>(null);

  constructor() {
    setInterval(() => this.refresh(), REFRESH_MS);
  }

  async refresh() {
    try {
      this.available = (await api.update()).available;
    } catch {
      // Viewers aren't told, and a device mid-restart will answer later
      this.available = null;
    }
  }
}

export const update = new Update();
