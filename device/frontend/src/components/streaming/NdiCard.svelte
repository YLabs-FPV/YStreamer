<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import { WEBSITE } from "@/lib/links";
  import type { NdiRuntime, NdiStatus, Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { bytes } from "@/stores/recording.svelte";
  import { btn } from "@/components/ui/buttons";
  import Card from "@/components/ui/Card.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import SourceFields from "@/components/streaming/SourceFields.svelte";
  import NdiLicenseDialog from "@/components/streaming/NdiLicenseDialog.svelte";

  let {
    ndi = $bindable(),
    saved,
  }: { ndi: Settings["ndi"]; saved: Settings["ndi"] } = $props();

  const POLL_MS = 3000;
  const PROGRESS_MS = 1000;
  const GUIDE = `${WEBSITE}/docs/other-software/ndi-and-omt`;
  const NDI = "https://ndi.video/";

  let status = $state<NdiStatus | null>(null);
  /** NDI's runtime fetched and waiting for the owner's answer */
  let runtime = $state<NdiRuntime | null>(null);
  let downloading = $state(false);
  let uploading = $state(false);
  let installing = $state(false);
  let licenseOpen = $state(false);
  let input: HTMLInputElement;

  async function poll() {
    try {
      status = await api.ndi();
    } catch {
      status = null;
    }
  }

  // Polled even when off: it's how the card knows the runtime is there
  $effect(() => {
    void saved.enabled;
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  // A download from before a reload is still underway, or waits for the
  // owner's answer
  onMount(async () => {
    try {
      runtime = await api.ndiRuntime();
      licenseOpen = !!runtime.staged;
    } catch {
      runtime = null;
    }
  });

  const missing = $derived(status !== null && !status.installed);
  const staged = $derived(runtime?.staged ?? null);
  const progress = $derived(runtime?.download ?? null);
  const fetching = $derived(downloading || !!progress);
  const working = $derived(fetching || uploading || installing);

  // The download's request only answers once it's done
  $effect(() => {
    if (!fetching) return;
    const timer = setInterval(async () => {
      try {
        const next = await api.ndiRuntime();
        // A download from before a reload, finished meanwhile
        if (!downloading && runtime?.download && !next.download)
          licenseOpen = !!next.staged;
        runtime = next;
      } catch {
        // Tried again on the next tick
      }
    }, PROGRESS_MS);
    return () => clearInterval(timer);
  });

  const percent = $derived(
    progress?.total
      ? Math.min(100, (progress.received / progress.total) * 100)
      : null,
  );

  async function download() {
    downloading = true;
    try {
      runtime = await api.downloadNdiRuntime();
      licenseOpen = !!runtime.staged;
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      downloading = false;
    }
  }

  async function upload(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    input.value = "";
    if (!file) return;
    uploading = true;
    try {
      runtime = await api.uploadNdiRuntime(file);
      licenseOpen = !!runtime.staged;
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      uploading = false;
    }
  }

  async function decline() {
    try {
      runtime = await api.discardNdiRuntime();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  // Only ever from the owner pressing "Accept and install"
  async function accept() {
    installing = true;
    try {
      await api.installNdiRuntime();
      runtime = { staged: null, download: null };
      licenseOpen = false;
      await poll();
    } catch (e) {
      // Not behind the dialog, where it couldn't be seen
      licenseOpen = false;
      snackbar.show(message(e), true);
    } finally {
      installing = false;
    }
  }

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

  {#if staged}
    <div class="flex flex-wrap items-center gap-3 px-4 py-3">
      <p class="min-w-0 flex-1 text-[12px] text-warn">
        NDI's runtime {staged.version} is downloaded and waits for you to accept or
        decline NDI's licence.
      </p>
      <button
        type="button"
        class={btn.primary}
        onclick={() => (licenseOpen = true)}
      >
        Read NDI's licence
      </button>
    </div>
  {:else if fetching}
    <div class="px-4 py-3">
      <p class="text-[13px] text-fg">
        {#if progress?.checking}
          Checking the download…
        {:else}
          Downloading from NDI…
          {#if progress?.received}
            <span class="text-muted tabular-nums">
              · {bytes(progress.received)}{progress.total
                ? ` of ${bytes(progress.total)}`
                : ""}
            </span>
          {/if}
        {/if}
      </p>
      <div class="mt-2 h-1.5 overflow-hidden rounded-full bg-surface-3">
        <div
          class="h-full bg-primary transition-[width] duration-500 {percent ===
            null && !progress?.checking
            ? 'animate-pulse'
            : ''}"
          style="width: {progress?.checking ? 100 : (percent ?? 100)}%"
        ></div>
      </div>
    </div>
  {:else if missing}
    <div class="px-4 py-3">
      <p class="text-[12px] text-warn">
        NDI's runtime is NDI's own software under NDI's licence, so it doesn't
        come with YStreamer. The device can download it from NDI for you; you'll
        see NDI's licence before anything is installed.
      </p>
      <div class="mt-3 flex flex-wrap gap-2">
        <button
          type="button"
          class={btn.primary}
          disabled={working}
          onclick={download}
        >
          Download from NDI
        </button>
        <button
          type="button"
          class={btn.plain}
          disabled={working}
          onclick={() => input.click()}
        >
          {uploading ? "Uploading…" : "Upload file…"}
        </button>
      </div>
      <p class="mt-2 text-[11px] text-muted">
        Without internet on the device, upload NDI SDK for Linux, downloaded
        from <a class="underline" href={NDI} target="_blank" rel="noreferrer"
          >ndi.video</a
        >.
        <a class="underline" href={GUIDE} target="_blank" rel="noreferrer"
          >More about installing it</a
        >
      </p>
    </div>
  {/if}
  <input
    bind:this={input}
    type="file"
    accept=".gz,.sh"
    class="hidden"
    onchange={upload}
  />

  {#if summary}
    <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
  {/if}

  <SourceFields bind:source={ndi} id="ndi" />

  <p class="px-4 py-3 text-[11px] text-muted">
    NDI® is a registered trademark of Vizrt NDI AB.
  </p>
</Card>

{#if staged}
  <NdiLicenseDialog
    bind:open={licenseOpen}
    version={staged.version}
    license={staged.license}
    {installing}
    onaccept={accept}
    ondecline={decline}
  />
{/if}
