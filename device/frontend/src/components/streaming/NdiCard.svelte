<script lang="ts">
  import { api } from "@/api/client";
  import { WEBSITE } from "@/lib/links";
  import type { NdiStatus, Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import SourceFields from "@/components/streaming/SourceFields.svelte";

  let {
    ndi = $bindable(),
    saved,
  }: { ndi: Settings["ndi"]; saved: Settings["ndi"] } = $props();

  const POLL_MS = 3000;
  const GUIDE = `${WEBSITE}/docs/other-software/ndi-and-omt`;

  let status = $state<NdiStatus | null>(null);

  // Polled even when off: it's how the card knows the runtime is there
  $effect(() => {
    void saved.enabled;
    const poll = async () => {
      try {
        status = await api.ndi();
      } catch {
        status = null;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const missing = $derived(status !== null && !status.installed);

  const summary = $derived.by(() => {
    if (!status || !saved.enabled) return null;
    if (status.error) return { text: status.error, tone: "text-down" };
    if (!status.active) return null;
    if (!status.receivers)
      return {
        text: `Waiting for a receiver. Look for "${saved.name}" in the list of NDI sources.`,
        tone: "text-muted",
      };
    const size =
      status.width && status.height ? `, ${status.width}×${status.height}` : "";
    const n = status.receivers;
    return {
      text: `Sending to ${n} ${n === 1 ? "receiver" : "receivers"}${size}`,
      tone: "text-ok",
    };
  });
</script>

<Card
  title="NDI"
  description="A source vMix, OBS (with DistroAV) and TriCaster find on the network."
>
  {#snippet aside()}
    <Switch
      bind:checked={ndi.enabled}
      label="NDI"
      disabled={missing && !ndi.enabled}
    />
  {/snippet}

  {#if missing}
    <p class="px-4 py-3 text-[12px] text-warn">
      The NDI runtime isn't installed on this device. It's NDI's own software,
      under NDI's licence, so it doesn't come with YStreamer.
      <a class="underline" href={GUIDE} target="_blank" rel="noreferrer"
        >How to install it</a
      >
    </p>
  {/if}

  {#if summary}
    <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
  {/if}

  <SourceFields bind:source={ndi} id="ndi" />

  <p class="px-4 py-3 text-[11px] text-muted">
    NDI® is a registered trademark of Vizrt NDI AB.
  </p>
</Card>
