import { api, message } from "@/api/client";
import type { RecordingFile, RecordingStatus } from "@/types/recording";
import { snackbar } from "@/stores/snackbar.svelte";

class RecordingStore {
  status = $state<RecordingStatus | null>(null);
  files = $state<RecordingFile[]>([]);
  location = $state<string | null>(null);
  busy = $state(false);
  loadingFiles = $state(false);

  readonly active = $derived(this.status?.active ?? false);
  readonly elapsed = $derived(this.status?.elapsed_secs ?? 0);

  apply(status: RecordingStatus) {
    const wasActive = this.status?.active;
    this.status = status;
    if (wasActive && !status.active && this.files.length) this.loadFiles();
  }

  async load() {
    try {
      this.status = await api.recording();
    } catch {
      // The WebSocket will bring it along anyway
    }
  }

  async toggle() {
    if (this.busy) return;
    this.busy = true;
    try {
      this.status = this.active
        ? await api.recordStop()
        : await api.recordStart();
      if (!this.active) {
        snackbar.show("Recording saved");
        this.loadFiles();
      }
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      this.busy = false;
    }
  }

  async loadFiles() {
    this.loadingFiles = true;
    try {
      this.files = await api.recordings(this.location);
    } catch (e) {
      // A drive that was being looked at may have been unplugged
      if (this.location) {
        this.location = null;
        this.files = await api.recordings().catch(() => []);
      } else {
        snackbar.show(message(e), true);
      }
    } finally {
      this.loadingFiles = false;
    }
  }

  async remove(name: string) {
    try {
      await api.deleteRecording(name, this.location);
      this.files = this.files.filter((f) => f.name !== name);
      snackbar.show(`Deleted ${name}`);
      this.load();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }
}

export const recording = new RecordingStore();

/** 3725 -> "1:02:05", 65 -> "1:05" */
export function duration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

export function bytes(n: number): string {
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${Math.round(n / 1024 ** 2)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}
