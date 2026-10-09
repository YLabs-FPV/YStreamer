import { api, message } from "@/api/client";
import type { Saved, Section, Settings } from "@/types/settings";
const SECTIONS: Section[] = [
  "input",
  "device",
  "wifi",
  "ethernet",
  "rtsp",
  "srt",
  "rtmp",
  "udp",
  "recording",
  "viewer",
  "display",
  "splash",
  "overlay",
  "advanced",
];

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));
const same = (a: unknown, b: unknown) =>
  JSON.stringify(a) === JSON.stringify(b);

class SettingsStore {
  saved = $state<Settings | null>(null);
  draft = $state<Settings | null>(null);
  loadError = $state<string | null>(null);
  saving = $state(false);
  error = $state<string | null>(null);

  dirty = $derived.by(() => {
    const { saved, draft } = this;
    if (!saved || !draft) return [] as Section[];
    return SECTIONS.filter((s) => !same(saved[s], draft[s]));
  });

  async load() {
    this.loadError = null;
    try {
      this.#accept(await api.settings());
    } catch (e) {
      this.loadError = message(e);
    }
  }

  discard() {
    if (this.saved) this.draft = clone(this.saved);
    this.error = null;
  }

  async save(): Promise<string[]> {
    if (!this.draft) return [];
    this.saving = true;
    this.error = null;
    const notices: string[] = [];
    try {
      const order = [...this.dirty].sort(
        (a, b) => Number(a === "wifi") - Number(b === "wifi"),
      );
      for (const section of order) {
        const res = await api.saveSection(section, this.draft[section]);
        this.#acceptSection(res, section);
        if (res.notice) notices.push(res.notice);
      }
    } catch (e) {
      this.error = message(e);
    } finally {
      this.saving = false;
    }
    return notices;
  }

  replace(res: Saved) {
    this.#accept(res.settings);
  }

  #accept(s: Settings) {
    this.saved = s;
    this.draft = clone(s);
    this.error = null;
  }

  #acceptSection(res: Saved, section: Section) {
    this.saved = res.settings;
    if (this.draft) this.draft[section] = clone(res.settings[section]) as never;
  }
}

export const settings = new SettingsStore();
