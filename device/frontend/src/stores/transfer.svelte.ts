import { api, message } from "@/api/client";
import type { Transfer } from "@/types/recording";
import { snackbar } from "@/stores/snackbar.svelte";

const POLL_MS = 1000;

/** The copy or move the device is doing, which carries on without a browser */
class TransferStore {
  progress = $state<Transfer | null>(null);
  readonly running = $derived(this.progress?.state === "running");
  /** Told once when a transfer ends, to refresh what it changed */
  onfinish: (() => void) | null = null;
  #timer: ReturnType<typeof setInterval> | null = null;

  /** Picks up a transfer started earlier or from another browser */
  async check() {
    try {
      this.#apply(await api.transfer());
    } catch {
      // Viewers aren't told, and a device mid-restart will answer later
    }
  }

  async start(job: Parameters<typeof api.startTransfer>[0]) {
    try {
      this.#apply(await api.startTransfer(job));
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  /** Stops it while it runs; clears the result afterwards */
  async cancel() {
    try {
      this.#apply(await api.cancelTransfer());
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  #apply(next: Transfer | null) {
    const wasRunning = this.running;
    this.progress = next;
    if (this.running && !this.#timer) {
      this.#timer = setInterval(() => this.check(), POLL_MS);
    } else if (!this.running && this.#timer) {
      clearInterval(this.#timer);
      this.#timer = null;
    }
    if (wasRunning && !this.running) this.onfinish?.();
  }
}

export const transfer = new TransferStore();
