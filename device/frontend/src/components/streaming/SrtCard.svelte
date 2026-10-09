<script lang="ts">
  import { copyText } from "@/lib/clipboard";
  import { api } from "@/api/client";
  import type { Settings, SrtStatus } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";

  let {
    srt = $bindable(),
    saved,
  }: { srt: Settings["srt"]; saved: Settings["srt"] } = $props();

  const POLL_MS = 2000;

  const MODES = [
    { label: "Listener", value: "listener" },
    { label: "Caller", value: "caller" },
  ];

  let status = $state<SrtStatus | null>(null);

  $effect(() => {
    if (!saved.enabled) {
      status = null;
      return;
    }
    const poll = async () => {
      try {
        status = await api.srt();
      } catch {
        status = null;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const target = $derived(
    saved.mode === "listener"
      ? `${location.hostname}:${saved.port}`
      : `${saved.host}:${saved.port}`,
  );

  const summary = $derived.by(() => {
    if (!status) return null;
    if (status.error) return { text: status.error, tone: "text-down" };
    if (saved.mode === "caller")
      return status.clients
        ? { text: "Connected", tone: "text-ok" }
        : { text: "Trying to reach the receiver…", tone: "text-warn" };
    return status.clients
      ? {
          text: `${status.clients} ${status.clients === 1 ? "receiver" : "receivers"} connected`,
          tone: "text-ok",
        }
      : { text: "Waiting for a receiver", tone: "text-muted" };
  });

  async function copyUrl() {
    const query = saved.passphrase
      ? `?passphrase=${encodeURIComponent(saved.passphrase)}`
      : "";
    try {
      await copyText(`srt://${target}${query}`);
      snackbar.show("SRT URL copied");
    } catch {
      snackbar.show("Couldn't copy; select the URL and copy it by hand", true);
    }
  }
</script>

<Card title="SRT">
  {#snippet aside()}
    <Switch bind:checked={srt.enabled} label="SRT" />
  {/snippet}

  {#if saved.enabled}
    <div class="flex flex-col gap-2 px-4 py-3">
      {#if saved.mode === "listener"}
        <div class="flex items-center gap-2">
          <code
            class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-3 py-1.5 font-mono text-[13px] text-fg select-all"
          >
            srt://{target}{saved.passphrase ? "?passphrase=••••" : ""}
          </code>
          <button type="button" class={btn.plain} onclick={copyUrl}>Copy</button
          >
        </div>
      {:else}
        <p class="text-[13px] text-muted">
          Sending to <span class="font-mono text-fg">srt://{target}</span>
        </p>
      {/if}
      {#if summary}
        <p class="text-[12px] {summary.tone}">{summary.text}</p>
      {/if}
    </div>
  {/if}

  <Field label="Mode" disabled={!srt.enabled}>
    <div class="w-full sm:w-64">
      <Segmented
        options={MODES}
        value={srt.mode}
        disabled={!srt.enabled}
        onchange={(v) => (srt.mode = v)}
      />
    </div>
  </Field>

  {#if srt.mode === "caller"}
    <Field label="Receiver address" for="srt-host" disabled={!srt.enabled}>
      <TextField
        id="srt-host"
        mono
        placeholder="192.168.1.20"
        bind:value={srt.host}
        disabled={!srt.enabled}
      />
    </Field>
  {/if}

  <Field label="Port" for="srt-port" disabled={!srt.enabled}>
    <TextField
      id="srt-port"
      type="number"
      min={1}
      max={65535}
      mono
      bind:value={srt.port}
      disabled={!srt.enabled}
    />
  </Field>

  <Field
    label="Latency (ms)"
    for="srt-latency"
    help="200 on a local network, 500–2000 over the internet."
    disabled={!srt.enabled}
  >
    <TextField
      id="srt-latency"
      type="number"
      min={20}
      max={8000}
      step={10}
      mono
      bind:value={srt.latency_ms}
      disabled={!srt.enabled}
    />
  </Field>

  <Field
    label="Passphrase"
    for="srt-passphrase"
    help="10–79 characters, or empty for none."
    disabled={!srt.enabled}
  >
    <TextField
      id="srt-passphrase"
      secret
      bind:value={srt.passphrase}
      disabled={!srt.enabled}
    />
  </Field>

  {#if srt.mode === "caller"}
    <Field label="Stream ID" for="srt-streamid" disabled={!srt.enabled}>
      <TextField
        id="srt-streamid"
        mono
        bind:value={srt.stream_id}
        disabled={!srt.enabled}
      />
    </Field>
  {/if}
</Card>
