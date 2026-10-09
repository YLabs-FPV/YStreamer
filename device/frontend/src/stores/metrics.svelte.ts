import { api } from "@/api/client";

type Column = (number | null)[];

class MetricsStore {
  t = $state.raw<number[]>([]);
  series = $state.raw<Record<string, Column>>({});
  loaded = $state(false);

  #seq = 0;
  #timer: ReturnType<typeof setInterval> | null = null;
  #busy = false;

  start() {
    if (this.#timer) return;
    this.#poll();
    this.#timer = setInterval(() => this.#poll(), 1000);
  }

  stop() {
    if (this.#timer) clearInterval(this.#timer);
    this.#timer = null;
  }

  async #poll() {
    if (this.#busy) return;
    this.#busy = true;
    try {
      const res = await api.metrics(this.#seq);
      const continues = res.first === this.#seq + 1 && this.t.length > 0;
      const old = continues ? this.t.length : 0;
      const t = continues ? this.t.concat(res.t) : res.t;
      const cut = Math.max(0, t.length - res.history);

      const series: Record<string, Column> = {};
      const names = new Set([
        ...(continues ? Object.keys(this.series) : []),
        ...Object.keys(res.series),
      ]);
      for (const name of names) {
        const before: Column =
          (continues && this.series[name]) || new Array(old).fill(null);
        const fresh: Column =
          res.series[name] ?? new Array(res.t.length).fill(null);
        const column = before.concat(fresh).slice(cut);
        if (column.some((v) => v !== null)) series[name] = column;
      }
      this.t = t.slice(cut);
      this.series = series;
      this.#seq = res.seq;
      this.loaded = true;
    } catch {
      // Shown as offline elsewhere; the next poll catches up
    } finally {
      this.#busy = false;
    }
  }
}

export const metrics = new MetricsStore();
