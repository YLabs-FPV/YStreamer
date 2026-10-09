<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { OverlayStatus, Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Slider from "@/components/ui/Slider.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import { btn } from "@/components/ui/buttons";
  import { SocketStream, socketStreamSupported } from "@/lib/socketStream";
  let { overlay = $bindable() }: { overlay: Settings["overlay"] } = $props();

  const MAX_SIDE = 2048;
  const MIN_SIZE = 2;
  const SNAP = 6;

  let status = $state<OverlayStatus | null>(null);
  let busy = $state(false);
  let input: HTMLInputElement;

  async function refresh() {
    try {
      status = await api.overlay();
    } catch {
      // the settings still work without it
    }
  }

  // The live picture behind the logo
  let videoEl: HTMLVideoElement;
  let stream: SocketStream | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let live = $state(false);
  let aspect = $state(16 / 9);

  function watch() {
    retryTimer = null;
    stream = new SocketStream(videoEl, {
      onplaying: () => (live = true),
      onclose: (busy) => {
        live = false;
        stream = null;
        retryTimer = setTimeout(watch, busy ? 5000 : 1500);
      },
    });
  }

  onMount(() => {
    refresh();
    if (socketStreamSupported()) watch();
  });
  onDestroy(() => {
    if (retryTimer) clearTimeout(retryTimer);
    stream?.close();
  });

  // PNG whatever came in, so transparency survives and the device has one
  // format to decode
  async function toPng(file: File): Promise<Blob> {
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      img.src = url;
      await img.decode();
      const scale = Math.min(
        1,
        MAX_SIDE / Math.max(img.naturalWidth, img.naturalHeight),
      );
      const canvas = document.createElement("canvas");
      canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
      canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("Couldn't process the image");
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      return await new Promise((resolve, reject) =>
        canvas.toBlob(
          (b) =>
            b ? resolve(b) : reject(new Error("Couldn't process the image")),
          "image/png",
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

  function picked(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    input.value = "";
    if (file) upload(file);
  }

  function dropped(e: DragEvent) {
    e.preventDefault();
    dropping = false;
    const file = e.dataTransfer?.files[0];
    if (file && !busy) upload(file);
  }

  async function upload(file: File) {
    busy = true;
    try {
      await api.uploadOverlay(await toPng(file));
      await refresh();
      overlay.enabled = true;
      snackbar.show("Logo updated");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    busy = true;
    try {
      await api.removeOverlay();
      await refresh();
      snackbar.show("Logo removed");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  const percent = (v: number) => `${Math.round(v)}%`;
  const tenth = (v: number) => Math.round(v * 10) / 10;
  const clamp = (v: number, lo: number, hi: number) =>
    Math.min(hi, Math.max(lo, v));

  let stageW = $state(0);
  let stageH = $state(0);
  let dropping = $state(false);
  let gesture = $state<"move" | "resize" | null>(null);
  let grab = { x: 0, y: 0 };

  const logoAspect = $derived(
    status?.image ? status.image[0] / status.image[1] : 1,
  );
  // Largest size at which the logo is still no taller than the picture
  const maxSize = $derived(
    stageW ? Math.min(100, ((stageH * logoAspect) / stageW) * 100) : 100,
  );

  // The logo on the stage, in pixels: the same rule the device applies
  const box = $derived.by(() => {
    const w = (stageW * Math.min(overlay.size_percent, maxSize)) / 100;
    const h = w / logoAspect;
    return {
      w,
      h,
      left: ((stageW - w) * overlay.x_percent) / 100,
      top: ((stageH - h) * overlay.y_percent) / 100,
    };
  });

  const centredX = $derived(overlay.x_percent === 50);
  const centredY = $derived(overlay.y_percent === 50);

  /** Pixels from the edge to a share of the room, settling on edges and middle */
  function share(px: number, room: number) {
    if (room <= 0) return 50;
    for (const stop of [0, room / 2, room])
      if (Math.abs(px - stop) <= SNAP) return (stop / room) * 100;
    return tenth(clamp((px / room) * 100, 0, 100));
  }

  function startMove(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    (e.currentTarget as HTMLElement).focus();
    gesture = "move";
    grab = { x: e.clientX - box.left, y: e.clientY - box.top };
  }

  function startResize(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    gesture = "resize";
    // Distance from the pointer to the corner it holds
    grab = { x: e.clientX - (box.left + box.w), y: 0 };
  }

  function drag(e: PointerEvent) {
    if (gesture === "move") {
      overlay.x_percent = share(e.clientX - grab.x, stageW - box.w);
      overlay.y_percent = share(e.clientY - grab.y, stageH - box.h);
    } else if (gesture === "resize") {
      // The top-left corner stays where it is while the opposite one follows
      const { left, top } = box;
      const room = Math.min(stageW - left, (stageH - top) * logoAspect);
      const w = clamp(
        e.clientX - grab.x - left,
        (stageW * MIN_SIZE) / 100,
        room,
      );
      overlay.size_percent = tenth((w / stageW) * 100);
      const h = w / logoAspect;
      overlay.x_percent =
        stageW - w > 0 ? tenth(clamp((left / (stageW - w)) * 100, 0, 100)) : 50;
      overlay.y_percent =
        stageH - h > 0 ? tenth(clamp((top / (stageH - h)) * 100, 0, 100)) : 50;
    }
  }

  function nudge(e: KeyboardEvent) {
    const step = e.shiftKey ? 5 : 1;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[e.key];
    if (!move) return;
    e.preventDefault();
    overlay.x_percent = clamp(overlay.x_percent + move[0], 0, 100);
    overlay.y_percent = clamp(overlay.y_percent + move[1], 0, 100);
  }

  function setSize(v: number) {
    overlay.size_percent = Math.min(v, Math.floor(maxSize));
  }
</script>

<Card
  title="Logo"
  description="Not applied to the goggles' streams and recordings."
>
  <div class="flex flex-col gap-3 px-4 py-3">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:clientWidth={stageW}
      bind:clientHeight={stageH}
      class="stage relative w-full touch-none overflow-hidden rounded-lg border select-none
        {dropping ? 'border-primary' : 'border-line'}"
      style="aspect-ratio:{aspect}"
      ondragover={(e) => {
        e.preventDefault();
        dropping = true;
      }}
      ondragleave={() => (dropping = false)}
      ondrop={dropped}
    >
      <video
        bind:this={videoEl}
        autoplay
        muted
        playsinline
        class="pointer-events-none absolute inset-0 h-full w-full {live
          ? ''
          : 'invisible'}"
        onresize={() => {
          if (videoEl.videoWidth && videoEl.videoHeight)
            aspect = videoEl.videoWidth / videoEl.videoHeight;
        }}
      ></video>
      {#if status?.image}
        {#if gesture === "move" && centredX}
          <span class="guide top-0 bottom-0 left-1/2 w-px"></span>
        {/if}
        {#if gesture === "move" && centredY}
          <span class="guide top-1/2 right-0 left-0 h-px"></span>
        {/if}
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          role="application"
          aria-label="Logo position. Drag it, or use the arrow keys"
          tabindex="0"
          class="logo absolute outline-1 outline-white/70 focus-visible:outline-2 focus-visible:outline-primary
            {gesture === 'move' ? 'cursor-grabbing' : 'cursor-grab'}"
          style="left:{box.left}px;top:{box.top}px;width:{box.w}px;height:{box.h}px"
          onpointerdown={startMove}
          onpointermove={drag}
          onpointerup={() => (gesture = null)}
          onpointercancel={() => (gesture = null)}
          onkeydown={nudge}
        >
          <img
            src={api.overlayUrl(status.version)}
            alt="Your logo"
            draggable="false"
            class="pointer-events-none h-full w-full"
            style="opacity:{overlay.opacity_percent / 100}"
          />
          <span
            class="absolute -right-1.5 -bottom-1.5 h-3 w-3 cursor-nwse-resize rounded-full border border-black/40 bg-white"
            onpointerdown={startResize}
            onpointermove={drag}
            onpointerup={() => (gesture = null)}
            onpointercancel={() => (gesture = null)}
          ></span>
        </div>
      {:else}
        <button
          type="button"
          class="absolute inset-0 flex flex-col items-center justify-center gap-1 bg-black/30 text-[13px] font-medium text-white backdrop-blur-md"
          disabled={busy}
          onclick={() => input.click()}
        >
          <span
            >{busy
              ? "Working…"
              : "Drop an image here, or click to choose one"}</span
          >
          <span class="text-[12px] text-white/70">
            PNG, WebP or JPEG. Transparency is kept.
          </span>
        </button>
      {/if}
    </div>

    {#if status?.image}
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => input.click()}
        >
          {busy ? "Working…" : "Replace logo"}
        </button>
        <button type="button" class={btn.text} disabled={busy} onclick={remove}>
          Remove
        </button>
        <p class="text-[12px] text-muted">
          Drag the logo to move it, and its corner to resize it.
        </p>
      </div>
    {/if}
    <input
      bind:this={input}
      type="file"
      accept="image/png,image/webp,image/jpeg,image/svg+xml"
      class="hidden"
      onchange={picked}
    />
  </div>

  <Field
    label="Show logo"
    for="overlay-enabled"
    help={status && !status.image
      ? "Upload a logo for it to appear."
      : undefined}
  >
    <Switch id="overlay-enabled" bind:checked={overlay.enabled} />
  </Field>

  <div class="flex flex-col gap-4 border-t border-line px-4 py-3">
    <Slider
      label="Size"
      value={Math.round(Math.min(overlay.size_percent, maxSize))}
      min={MIN_SIZE}
      max={100}
      format={percent}
      onchange={setSize}
    />
    <Slider
      label="Opacity"
      value={overlay.opacity_percent}
      min={10}
      max={100}
      step={5}
      format={percent}
      onchange={(v) => (overlay.opacity_percent = v)}
    />
  </div>
</Card>

<style>
  .stage {
    background: #1f3a4d;
  }
  .guide {
    position: absolute;
    background: var(--color-primary);
    pointer-events: none;
  }
</style>
