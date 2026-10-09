<script lang="ts">
  import { onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { DumlFrame, DumlStatus } from "@/types/duml";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import { btn } from "@/components/ui/buttons";

  const POLL_MS = 1000;
  const FRESH_MS = 1500;
  const PREVIEW_BYTES = 20;

  let live = $state(false);
  let status = $state<DumlStatus | null>(null);
  let error = $state<string | null>(null);
  let rates = $state<Record<string, number>>({});
  let expanded = $state<string | null>(null);

  /** Payloads at the moment "Mark" was pressed, to diff against */
  let mark = $state<Record<string, string> | null>(null);
  let changedOnly = $state(false);

  let prev: { at: number; counts: Record<string, number> } | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;

  const key = (f: DumlFrame) =>
    `${f.src}-${f.dst}-${f.response ? 1 : 0}-${f.cmd_set}-${f.cmd_id}`;
  const hex2 = (n: number) => n.toString(16).padStart(2, "0").toUpperCase();
  const addr = (n: number) => `${hex2(n)} (${n & 0x1f}.${n >> 5})`;

  async function poll() {
    try {
      const next = await api.duml();
      const at = performance.now();
      const counts: Record<string, number> = {};
      const nextRates: Record<string, number> = {};
      for (const f of next.frames) {
        const k = key(f);
        counts[k] = f.count;
        if (prev && prev.counts[k] !== undefined) {
          const dt = (at - prev.at) / 1000;
          nextRates[k] = dt > 0 ? (f.count - prev.counts[k]) / dt : 0;
        }
      }
      prev = { at, counts };
      rates = nextRates;
      status = next;
      error = null;
    } catch (e) {
      error = message(e);
    }
  }

  $effect(() => {
    if (!live) return;
    poll();
    timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  onDestroy(() => clearInterval(timer));

  function setMark() {
    if (!status) return;
    mark = Object.fromEntries(status.frames.map((f) => [key(f), f.payload]));
    snackbar.show("Marked. Rows that change from here are highlighted.");
  }

  async function clear() {
    try {
      await api.clearDuml();
      prev = null;
      mark = null;
      expanded = null;
      await poll();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  let send = $state({ dst: "01", set: "02", id: "", payload: "" });
  let sending = $state(false);

  const byte = (s: string) =>
    /^[0-9a-f]{1,2}$/i.test(s.trim()) ? parseInt(s, 16) : null;
  const request = $derived({
    dst: byte(send.dst),
    cmd_set: byte(send.set),
    cmd_id: byte(send.id),
  });
  const payloadOk = $derived(
    /^([0-9a-f]{2})*$/i.test(send.payload.replace(/\s+/g, "")),
  );
  const sendable = $derived(
    request.dst !== null &&
      request.cmd_set !== null &&
      request.cmd_id !== null &&
      payloadOk,
  );

  async function sendFrame(e: SubmitEvent) {
    e.preventDefault();
    if (!sendable) return;
    sending = true;
    try {
      await api.sendDuml({
        dst: request.dst!,
        cmd_set: request.cmd_set!,
        cmd_id: request.cmd_id!,
        payload: send.payload,
      });
      snackbar.show("Sent. A reply shows up as a response row.");
      if (live) poll();
    } catch (err) {
      snackbar.show(message(err), true);
    } finally {
      sending = false;
    }
  }

  const differsFromMark = (f: DumlFrame) =>
    mark !== null && mark[key(f)] !== f.payload;

  const rows = $derived(
    (status?.frames ?? []).filter((f) => !changedOnly || differsFromMark(f)),
  );

  function bytesOf(payload: string): string[] {
    return payload.match(/../g) ?? [];
  }

  function preview(f: DumlFrame): string {
    const b = bytesOf(f.payload).slice(0, PREVIEW_BYTES).join(" ");
    return f.len > PREVIEW_BYTES ? `${b} …` : b;
  }

  function dump(f: DumlFrame) {
    const cur = bytesOf(f.payload);
    const old = mark ? bytesOf(mark[key(f)] ?? "") : null;
    const lines = [];
    for (let off = 0; off < cur.length; off += 16) {
      const bytes = cur.slice(off, off + 16).map((b, i) => ({
        b,
        diff: old !== null && old[off + i] !== b,
      }));
      const ascii = bytes
        .map(({ b }) => {
          const c = parseInt(b, 16);
          return c >= 0x20 && c < 0x7f ? String.fromCharCode(c) : ".";
        })
        .join("");
      lines.push({ off: off.toString(16).padStart(4, "0"), bytes, ascii });
    }
    return lines;
  }

  const rate = (k: string) => {
    const r = rates[k];
    return r === undefined ? "–" : r < 10 ? r.toFixed(1) : Math.round(r);
  };
</script>

<Card
  title="DUML monitor"
  description="Every control frame the goggles send, latest payload per kind. Mark, change something (arm, switch modes), then look for highlighted rows."
>
  {#snippet aside()}
    <Switch bind:checked={live} label="Live" />
  {/snippet}

  <div
    class="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 text-[12px]"
  >
    {#if status}
      <span class={status.session.registered ? "text-ok" : "text-warn"}>
        {status.session.registered ? "Registered" : "Not registered"}
      </span>
      <span class="text-muted">
        Devices <span class="font-mono text-fg"
          >{status.session.devices
            .map((d) => `${d.code}@${hex2(d.address)}`)
            .join(", ") || "–"}</span
        >
      </span>
      <span class="text-muted">
        Attempts <span class="font-mono text-fg">{status.session.attempts}</span
        >
      </span>
      <span class="text-muted">
        Heartbeats <span class="font-mono text-fg"
          >{status.session.heartbeats}</span
        >
      </span>
    {:else}
      <span class="text-muted">Turn on Live to start watching.</span>
    {/if}
    <div class="ml-auto flex items-center gap-2">
      <label class="flex items-center gap-2 text-muted">
        <input
          type="checkbox"
          bind:checked={changedOnly}
          disabled={mark === null}
          class="accent-primary"
        />
        Changed since mark
      </label>
      <button
        type="button"
        class={btn.plain}
        disabled={!status}
        onclick={setMark}>Mark</button
      >
      <button type="button" class={btn.plain} onclick={clear}>Clear</button>
    </div>
  </div>

  <form
    onsubmit={sendFrame}
    class="flex flex-wrap items-end gap-2 px-4 py-3 text-[12px]"
  >
    {@render field("To", "dst", "w-12")}
    {@render field("Set", "set", "w-12")}
    {@render field("Id", "id", "w-12")}
    <label class="flex min-w-40 flex-1 flex-col gap-1 text-muted">
      Payload
      <input
        bind:value={send.payload}
        placeholder="empty, or hex bytes"
        autocomplete="off"
        spellcheck="false"
        class="rounded-md border bg-surface-2 px-2 py-1.5 font-mono text-[12px] text-fg outline-none focus:border-primary {payloadOk
          ? 'border-line'
          : 'border-down'}"
      />
    </label>
    <button type="submit" class={btn.plain} disabled={!sendable || sending}>
      {sending ? "Sending…" : "Send"}
    </button>
  </form>

  {#if error}
    <p class="px-4 py-3 text-[13px] text-down">{error}</p>
  {/if}

  {#if status && rows.length === 0}
    <p class="px-4 py-6 text-center text-[13px] text-muted">
      {changedOnly ? "Nothing changed since the mark." : "No frames yet."}
    </p>
  {:else if rows.length > 0}
    <div class="overflow-x-auto">
      <table class="w-full font-mono text-[11px] tabular-nums">
        <thead class="text-left text-faint">
          <tr class="border-b border-line">
            <th class="px-4 py-2 font-normal">set/id</th>
            <th class="px-2 py-2 font-normal">src → dst</th>
            <th class="px-2 py-2 font-normal">dir</th>
            <th class="px-2 py-2 text-right font-normal">count</th>
            <th class="px-2 py-2 text-right font-normal">/s</th>
            <th class="px-2 py-2 text-right font-normal">chg</th>
            <th class="px-2 py-2 text-right font-normal">len</th>
            <th class="px-4 py-2 font-normal">payload</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as f (key(f))}
            {@const k = key(f)}
            {@const fresh = f.changes > 0 && f.changed_ms_ago < FRESH_MS}
            {@const marked = differsFromMark(f)}
            <tr
              class="cursor-pointer border-b border-line transition-colors hover:bg-surface-2/60
                {marked ? 'bg-accent/10' : ''}"
              onclick={() => (expanded = expanded === k ? null : k)}
            >
              <td
                class="border-l-2 px-4 py-1.5 whitespace-nowrap text-fg {fresh
                  ? 'border-accent'
                  : 'border-transparent'}"
              >
                {hex2(f.cmd_set)}/{hex2(f.cmd_id)}
              </td>
              <td class="px-2 py-1.5 whitespace-nowrap text-muted"
                >{addr(f.src)} → {addr(f.dst)}</td
              >
              <td class="px-2 py-1.5 text-muted"
                >{f.response ? "RSP" : "REQ"}</td
              >
              <td class="px-2 py-1.5 text-right text-fg">{f.count}</td>
              <td class="px-2 py-1.5 text-right text-muted">{rate(k)}</td>
              <td class="px-2 py-1.5 text-right text-muted">{f.changes}</td>
              <td class="px-2 py-1.5 text-right text-muted">{f.len}</td>
              <td class="px-4 py-1.5 whitespace-nowrap text-muted"
                >{preview(f)}</td
              >
            </tr>
            {#if expanded === k}
              <tr class="border-b border-line bg-surface-2/40">
                <td colspan="8" class="px-4 py-3">
                  {#if f.len > f.payload.length / 2}
                    <p class="mb-2 text-faint">
                      Showing the first {f.payload.length / 2} of {f.len} bytes
                    </p>
                  {/if}
                  {#each dump(f) as line (line.off)}
                    <div class="flex gap-4 whitespace-pre">
                      <span class="text-faint">{line.off}</span>
                      <span class="text-fg"
                        >{#each line.bytes as { b, diff }, i (i)}<span
                            class={diff
                              ? "rounded-sm bg-accent/25 text-accent"
                              : ""}>{b}</span
                          >{i < line.bytes.length - 1 ? " " : ""}{/each}</span
                      >
                      <span class="text-faint">{line.ascii}</span>
                    </div>
                  {:else}
                    <p class="text-faint">Empty payload</p>
                  {/each}
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</Card>

{#snippet field(label: string, name: "dst" | "set" | "id", width: string)}
  <label class="flex flex-col gap-1 text-muted">
    {label}
    <input
      bind:value={send[name]}
      maxlength="2"
      placeholder="hex"
      autocomplete="off"
      spellcheck="false"
      class="{width} rounded-md border border-line bg-surface-2 px-2 py-1.5 text-center font-mono text-[12px] text-fg uppercase outline-none focus:border-primary"
    />
  </label>
{/snippet}
