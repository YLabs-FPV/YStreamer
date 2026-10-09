import { api, message, setUnauthorizedHandler } from "@/api/client";
import type { Access } from "@/types/auth";
import type { ViewerTransport } from "@/types/settings";

const RETRY_MS = 3000;

class AuthStore {
  loaded = $state(false);
  unreachable = $state<string | null>(null);
  enabled = $state(false);
  protectViewing = $state(false);
  access = $state<Access>("none");
  transport = $state<ViewerTransport>("websocket");

  admin = $derived(this.access === "admin");
  canView = $derived(this.access !== "none");

  #listeners: (() => void)[] = [];
  #retry: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    setUnauthorizedHandler(() => this.load());
  }

  onChange(fn: () => void) {
    this.#listeners.push(fn);
  }

  async load() {
    const before = this.access;
    if (this.#retry) clearTimeout(this.#retry);
    this.#retry = null;
    try {
      const s = await api.auth();
      this.enabled = s.enabled;
      this.protectViewing = s.protect_viewing;
      this.access = s.access;
      this.transport = s.transport;
      this.loaded = true;
      this.unreachable = null;
    } catch (e) {
      if (!this.loaded) {
        this.unreachable = message(e);
        this.#retry = setTimeout(() => this.load(), RETRY_MS);
      }
    }
    if (this.access !== before) this.#listeners.forEach((fn) => fn());
  }

  async login(password: string) {
    await api.login(password);
    await this.load();
  }

  async logout() {
    await api.logout();
    await this.load();
  }

  async setPassword(current: string, next: string | null) {
    await api.setPassword(current, next);
    await this.load();
  }

  async setProtectViewing(protect: boolean) {
    await api.setProtectViewing(protect);
    await this.load();
  }
}

export const auth = new AuthStore();
