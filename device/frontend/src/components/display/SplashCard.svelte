<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";
  let { splash = $bindable() }: { splash: Settings["splash"] } = $props();

  const SCALING = [
    { label: "Fit", value: "fit" },
    { label: "Fill", value: "fill" },
  ];
  const WIDTH = 1920;
  const HEIGHT = 1080;

  let custom = $state<boolean | null>(null);
  let busy = $state(false);
  let version = $state(Date.now());
  let previewFailed = $state(false);
  let input: HTMLInputElement;

  onMount(async () => {
    try {
      custom = (await api.splash()).custom_image;
    } catch {
      // the switches still work without it
    }
  });

  /** No bigger than it needs to be to cover the screen, in its own shape:
   *  fitting or filling happens on the device, so it can be changed later */
  async function sizedForScreen(file: File): Promise<Blob> {
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      img.src = url;
      await img.decode();
      const scale = Math.min(
        1,
        Math.max(WIDTH / img.naturalWidth, HEIGHT / img.naturalHeight),
      );
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(img.naturalWidth * scale);
      canvas.height = Math.round(img.naturalHeight * scale);
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("Couldn't process the image");
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      return await new Promise((resolve, reject) =>
        canvas.toBlob(
          (b) =>
            b ? resolve(b) : reject(new Error("Couldn't process the image")),
          "image/jpeg",
          0.92,
        ),
      );
    } catch (e) {
      throw e instanceof Error && e.message.startsWith("Couldn't")
        ? e
        : new Error("That file isn't an image this browser can open");
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  async function upload(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    input.value = "";
    if (!file) return;
    busy = true;
    try {
      await api.uploadSplash(await sizedForScreen(file));
      custom = true;
      refreshPreview();
      snackbar.show("Splash image updated");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      await api.removeSplash();
      custom = false;
      refreshPreview();
      snackbar.show("Back to the default splash screen");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  function refreshPreview() {
    previewFailed = false;
    version = Date.now();
  }
</script>

<Card title="Splash screen">
  <div class="flex flex-col gap-3 px-4 py-3">
    <div
      class="relative aspect-video w-full overflow-hidden rounded-lg border border-line bg-black sm:max-w-md"
    >
      {#if previewFailed}
        <p
          class="absolute inset-0 flex items-center justify-center text-[12px] text-muted"
        >
          Preview not available yet
        </p>
      {:else}
        <img
          src={api.splashPreviewUrl(
            version,
            splash.status_over_image,
            splash.image_scaling,
          )}
          alt="Splash screen preview"
          class="h-full w-full {custom && splash.image_scaling === 'fill'
            ? 'object-cover'
            : 'object-contain'}"
          onerror={() => (previewFailed = true)}
        />
      {/if}
    </div>
    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        class={btn.plain}
        disabled={busy}
        onclick={() => input.click()}
      >
        {busy ? "Working…" : custom ? "Replace image" : "Upload image"}
      </button>
      {#if custom}
        <button type="button" class={btn.text} disabled={busy} onclick={remove}>
          Use default
        </button>
      {/if}
    </div>
    <input
      bind:this={input}
      type="file"
      accept="image/*"
      class="hidden"
      onchange={upload}
    />
  </div>

  {#if custom}
    <Field label="Image scaling" disabled={!custom}>
      <div class="w-full sm:w-64">
        <Segmented
          options={SCALING}
          value={splash.image_scaling}
          disabled={!custom}
          onchange={(v) => (splash.image_scaling = v)}
        />
      </div>
    </Field>
  {/if}
  <Field
    label="Show status over custom image"
    for="splash-status"
    help="Keeps the addresses and connection status visible along the bottom."
    disabled={!custom}
  >
    <Switch
      id="splash-status"
      bind:checked={splash.status_over_image}
      disabled={!custom}
    />
  </Field>
  <Field label="Show access point password on screen" for="splash-ap-password">
    <Switch id="splash-ap-password" bind:checked={splash.show_ap_password} />
  </Field>
  <Field label="Freeze on last frame" for="splash-freeze">
    <Switch id="splash-freeze" bind:checked={splash.freeze_last_frame} />
  </Field>
  <Field
    label="Grayscale frozen frame"
    for="splash-gray"
    help="Like the Fly app, so a frozen picture can't be mistaken for live video."
    disabled={!splash.freeze_last_frame}
  >
    <Switch
      id="splash-gray"
      bind:checked={splash.grayscale_freeze}
      disabled={!splash.freeze_last_frame}
    />
  </Field>
</Card>
