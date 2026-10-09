<script lang="ts">
  import { api } from "@/api/client";
  import type { Settings, UdpStatus } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let {
    udp = $bindable(),
    saved,
  }: { udp: Settings["udp"]; saved: Settings["udp"] } = $props();

  const POLL_MS = 3000;

  const FORMATS = [
    { label: "RTP", value: "rtp" },
    { label: "MPEG-TS", value: "mpegts" },
  ];

  const PLACEHOLDER = {
    rtp: "192.168.1.20:5600",
    mpegts: "192.168.1.20:1234, 239.0.0.1:1234",
  };

  let status = $state<UdpStatus | null>(null);

  $effect(() => {
    if (!saved.enabled) {
      status = null;
      return;
    }
    const poll = async () => {
      try {
        status = await api.udp();
      } catch {
        status = null;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const summary = $derived.by(() => {
    if (!status) return null;
    if (status.error) return { text: status.error, tone: "text-down" };
    if (!status.active) return null;
    const n = status.destinations;
    return {
      text: `Sending to ${n} ${n === 1 ? "destination" : "destinations"}`,
      tone: "text-ok",
    };
  });
</script>

<Card title="UDP">
  {#snippet aside()}
    <Switch bind:checked={udp.enabled} label="UDP" />
  {/snippet}

  {#if summary}
    <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
  {/if}

  <Field label="Format" disabled={!udp.enabled}>
    <div class="w-full sm:w-64">
      <Segmented
        options={FORMATS}
        value={udp.format}
        disabled={!udp.enabled}
        onchange={(v) => (udp.format = v)}
      />
    </div>
  </Field>

  <Field
    label="Destinations"
    for="udp-destinations"
    help="host:port of each receiver, separated by commas. Up to 8."
    disabled={!udp.enabled}
  >
    <TextField
      id="udp-destinations"
      mono
      placeholder={PLACEHOLDER[udp.format]}
      bind:value={udp.destinations}
      disabled={!udp.enabled}
    />
  </Field>
</Card>
