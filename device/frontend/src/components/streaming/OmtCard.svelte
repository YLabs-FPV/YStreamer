<script lang="ts">
  import { copyText } from "@/lib/clipboard";
  import { api } from "@/api/client";
  import type { OmtStatus, Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import SourceFields from "@/components/streaming/SourceFields.svelte";
  import { btn } from "@/components/ui/buttons";

  let {
    omt = $bindable(),
    saved,
  }: { omt: Settings["omt"]; saved: Settings["omt"] } = $props();

  const POLL_MS = 3000;

  let status = $state<OmtStatus | null>(null);

  $effect(() => {
    if (!saved.enabled) {
      status = null;
      return;
    }
    const poll = async () => {
      try {
        status = await api.omt();
      } catch {
        status = null;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const url = $derived(
    status?.port ? `omt://${location.hostname}:${status.port}` : null,
  );

  const summary = $derived.by(() => {
    if (!status) return null;
    if (status.error) return { text: status.error, tone: "text-down" };
    if (!status.active) return null;
    if (!status.receivers)
      return {
        text: status.announced
          ? `Waiting for a receiver. Look for "${saved.name}" in the list of OMT sources.`
          : "Waiting for a receiver. Not announced on the network, so add it by address.",
        tone: status.announced ? "text-muted" : "text-warn",
      };
    const size =
      status.width && status.height ? `, ${status.width}×${status.height}` : "";
    const n = status.receivers;
    return {
      text: `Sending to ${n} ${n === 1 ? "receiver" : "receivers"}${size}`,
      tone: "text-ok",
    };
  });

  async function copyUrl() {
    if (!url) return;
    try {
      await copyText(url);
      snackbar.show("OMT address copied");
    } catch {
      snackbar.show(
        "Couldn't copy; select the address and copy it by hand",
        true,
      );
    }
  }
</script>

<Card
  title="OMT"
  description="Open Media Transport, an open alternative to NDI that vMix receives."
>
  {#snippet aside()}
    <Switch bind:checked={omt.enabled} label="OMT" />
  {/snippet}

  {#if saved.enabled && (url || summary)}
    <div class="flex flex-col gap-2 px-4 py-3">
      {#if url}
        <div class="flex items-center gap-2">
          <code
            class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-3 py-1.5 font-mono text-[13px] text-fg select-all"
          >
            {url}
          </code>
          <button type="button" class={btn.plain} onclick={copyUrl}>Copy</button
          >
        </div>
      {/if}
      {#if summary}
        <p class="text-[12px] {summary.tone}">{summary.text}</p>
      {/if}
    </div>
  {/if}

  <SourceFields bind:source={omt} id="omt" />
</Card>
