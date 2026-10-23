<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { InputStatus, UvcDevice, UvcMode } from "@/types/input";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { developer } from "@/stores/developer.svelte";
  import { settings } from "@/stores/settings.svelte";
  import { copyText } from "@/lib/clipboard";

  let { input = $bindable() }: { input: Settings["input"] } = $props();

  const SOURCES = [
    { label: "DJI goggles", value: "dji_fpv" },
    { label: "FPV Goggles V1/V2", value: "dji_fpv_legacy" },
    { label: "USB camera", value: "uvc" },
    { label: "RTMP stream", value: "rtmp" },
  ];

  const ABOUT: Record<string, string> = {
    dji_fpv: "DJI Goggles 2, Integra, 3 or N3 connected to the USB-C port.",
    dji_fpv_legacy:
      "The first DJI FPV Goggles, V1 or V2, in one of the USB-A ports. Video only: no camera settings, battery or link quality.",
    uvc: "An analog receiver, HDMI capture stick or webcam in one of the USB-A ports. Its video is encoded to H.264 by the Pi's hardware encoder.",
    rtmp: "A stream pushed over the network by OBS, a phone app or DJI Fly's live streaming. H.264 up to 1080p; the app's sound is left out.",
  };

  const FORMAT: Record<string, string> = {
    h264: "H.264",
    mjpeg: "MJPEG",
    raw: "Uncompressed",
  };

  let status = $state<InputStatus | null>(null);
  let devices = $state<UvcDevice[] | null>(null);
  let devicesError = $state<string | null>(null);
  let scanning = $state(false);
  let timer: ReturnType<typeof setInterval> | null = null;

  const legacy = $derived(
    status?.mode === "dji_fpv_legacy" ? status.legacy : null,
  );
  const legacySummary = $derived.by(() => {
    switch (legacy?.state) {
      case "live":
        return { text: "Receiving video.", tone: "text-ok" };
      case "waiting":
        return {
          text: "Goggles found, no video yet. Power the aircraft and wait for the picture in the goggles.",
          tone: "text-warn",
        };
      case "no_device":
        return {
          text: "No goggles found. Plug them into a USB-A port.",
          tone: "text-muted",
        };
      case "error":
        return { text: `${legacy.error}. Retrying…`, tone: "text-down" };
      default:
        return null;
    }
  });

  const CAPTURE_BYTES = 8 * 1024 * 1024;
  const capturing = $derived(
    legacy?.captured != null && legacy.captured < CAPTURE_BYTES,
  );

  async function capture() {
    try {
      await api.startLegacyCapture();
      await refresh();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  async function refresh() {
    try {
      status = await api.input();
    } catch {
      // Shown as offline elsewhere
    }
  }

  async function scan() {
    scanning = true;
    try {
      devices = await api.inputDevices();
      devicesError = null;
    } catch (e) {
      devicesError = message(e);
    } finally {
      scanning = false;
    }
  }

  onMount(() => {
    refresh();
    scan();
    timer = setInterval(refresh, 3000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  const RTMP_PORT = 1935;
  // Letters and digits that can't be mistaken for each other
  const KEY_CHARS = "abcdefghijkmnpqrstuvwxyz23456789";

  function newKey() {
    const bytes = crypto.getRandomValues(new Uint8Array(10));
    input.rtmp.key = Array.from(
      bytes,
      (b) => KEY_CHARS[b % KEY_CHARS.length],
    ).join("");
  }

  // The saved settings: what the device is listening for right now
  const savedRtmp = $derived(
    settings.saved?.input.mode === "rtmp" && settings.saved.input.rtmp.key
      ? settings.saved.input.rtmp
      : null,
  );
  const rtmpServer = $derived(
    savedRtmp
      ? `rtmp://${location.hostname}${savedRtmp.port === RTMP_PORT ? "" : `:${savedRtmp.port}`}/live`
      : "",
  );

  async function copy(text: string, what: string) {
    try {
      await copyText(text);
      snackbar.show(`${what} copied`);
    } catch {
      snackbar.show(
        `Couldn't copy; select the ${what.toLowerCase()} and copy it by hand`,
        true,
      );
    }
  }

  const rtmp = $derived(status?.mode === "rtmp" ? status.rtmp : null);
  const rtmpSummary = $derived.by(() => {
    switch (rtmp?.state) {
      case "live":
        return {
          text: `Receiving video from ${rtmp.publisher}.`,
          tone: "text-ok",
        };
      case "connected":
        return {
          text: `${rtmp.publisher} is connected, no video yet.`,
          tone: "text-warn",
        };
      case "waiting":
        return {
          text: rtmp.error
            ? `Waiting for a stream. ${rtmp.error}.`
            : "Waiting for a stream.",
          tone: rtmp.error ? "text-down" : "text-muted",
        };
      case "error":
        return { text: `${rtmp.error}. Retrying…`, tone: "text-down" };
      default:
        return null;
    }
  });

  // The configured device, or the first one: what "First found" resolves to
  const device = $derived(
    devices?.find((d) => d.path === input.uvc.device) ?? devices?.[0] ?? null,
  );

  const deviceOptions = $derived.by(() => {
    const list = (devices ?? []).map((d) => ({ value: d.path, label: d.name }));
    if (input.uvc.device && !list.some((d) => d.value === input.uvc.device))
      list.unshift({
        value: input.uvc.device,
        label: "Saved device (not plugged in)",
      });
    return [{ value: "", label: "First one found" }, ...list];
  });

  const modeLabel = (m: UvcMode) =>
    `${m.width} × ${m.height} · ${m.fps} fps · ${FORMAT[m.format]}`;

  const modeOptions = $derived.by(() => {
    const list = (device?.modes ?? []).map((m) => ({
      value: m.id,
      label: modeLabel(m),
    }));
    if (input.uvc.mode && !list.some((m) => m.value === input.uvc.mode))
      list.unshift({
        value: input.uvc.mode,
        label: `${input.uvc.mode} (not offered)`,
      });
    return [{ value: "", label: "Best available" }, ...list];
  });

  // Passing the device's H.264 through: nothing to encode, so no bitrate
  const passthrough = $derived(status?.uvc.encoder === "passthrough");

  const ENCODER: Record<string, string> = {
    passthrough: "passed through",
    hardware: "hardware encoding",
    software: "software encoding",
  };

  const summary = $derived.by(() => {
    const u = status?.uvc;
    if (!u || status?.mode !== "uvc") return null;
    switch (u.state) {
      case "live": {
        const mode = u.mode
          ? `${u.mode.width} × ${u.mode.height} · ${u.mode.fps} fps`
          : "";
        const how = [
          u.format && FORMAT[u.format],
          u.encoder && ENCODER[u.encoder],
        ]
          .filter(Boolean)
          .join(", ");
        return { text: `${u.device} · ${mode} · ${how}`, tone: "text-ok" };
      }
      case "starting":
        return {
          text: `Starting ${u.device ?? "the camera"}…`,
          tone: "text-warn",
        };
      case "no_device":
        return {
          text: "No USB camera found. Plug one into a USB-A port.",
          tone: "text-muted",
        };
      case "error":
        return { text: `${u.error}. Retrying…`, tone: "text-down" };
      default:
        return null;
    }
  });
</script>

<div class="flex flex-col gap-4">
  <Card title="Video source">
    <div class="flex flex-col gap-2 px-4 py-3">
      <Segmented
        options={SOURCES}
        value={input.mode}
        onchange={(v) => (input.mode = v)}
      />
      <p class="text-[12px] text-muted">
        {ABOUT[input.mode]}
      </p>
      {#if input.mode === "dji_fpv"}
        <p
          class="rounded-xl border border-line bg-warn/10 px-4 py-3 text-[12px] text-fg shadow-sm"
        >
          To turn off OSD in the video feed, turn off "Camera View Recording" in
          Settings → Camera → Advanced Camera Settings
        </p>
      {/if}
    </div>
  </Card>

  {#if input.mode === "dji_fpv_legacy"}
    <Card title="FPV Goggles V1/V2">
      <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
        Experimental: not tried on real hardware by maintainers
      </p>
      {#if legacySummary}
        <p class="px-4 py-3 text-[12px] {legacySummary.tone}">
          {legacySummary.text}
        </p>
      {/if}
      {#if developer.enabled}
        <Field
          label="Capture the raw stream"
          help="Saves the next 8 MB exactly as the goggles send them."
        >
          <button
            type="button"
            class={btn.plain}
            disabled={!legacy || capturing}
            onclick={capture}
          >
            {capturing
              ? `Capturing… ${Math.round(((legacy?.captured ?? 0) / CAPTURE_BYTES) * 100)}%`
              : "Capture"}
          </button>
          {#if legacy?.captured}
            <a
              class={btn.text}
              href={api.legacyCaptureUrl}
              download="fpv-goggles-capture.h264"
            >
              Download
            </a>
          {/if}
        </Field>
      {/if}
    </Card>
  {/if}

  {#if input.mode === "rtmp"}
    <Card title="RTMP stream">
      {#if savedRtmp}
        <div class="flex flex-col gap-2 px-4 py-3">
          <div class="flex items-center gap-2">
            <code
              class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-3 py-1.5 font-mono text-[13px] text-fg select-all"
            >
              {rtmpServer}/••••
            </code>
            <button
              type="button"
              class={btn.plain}
              onclick={() => copy(`${rtmpServer}/${savedRtmp.key}`, "URL")}
            >
              Copy
            </button>
          </div>
          <p class="text-[12px] text-muted">
            Apps that ask for a server and a stream key separately: the server
            is <span class="font-mono text-fg">{rtmpServer}</span>, the key is
            the one below.
          </p>
          {#if rtmpSummary}
            <p class="text-[12px] {rtmpSummary.tone}">{rtmpSummary.text}</p>
          {/if}
        </div>
      {:else}
        <p class="px-4 py-3 text-[12px] text-muted">
          Save to start listening. The address to stream to shows here.
        </p>
      {/if}
      {#if rtmp?.reorders}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          The stream has B-frames, which add delay. Set B-frames to 0 in the app
          or encoder for the lowest delay.
        </p>
      {/if}

      <Field
        label="Stream key"
        for="rtmp-in-key"
        help="Only an app that knows it can stream here. Made up when you first save."
      >
        <TextField id="rtmp-in-key" secret mono bind:value={input.rtmp.key} />
        <button type="button" class={btn.plain} onclick={newKey}>New</button>
      </Field>

      <Field
        label="Port"
        for="rtmp-in-port"
        help="1935 is what apps expect when the URL has none."
      >
        <TextField
          id="rtmp-in-port"
          type="number"
          min={1024}
          max={65535}
          mono
          bind:value={input.rtmp.port}
        />
      </Field>

      <Field
        label="Only from"
        for="rtmp-in-allowed"
        help="IP addresses allowed to stream, comma separated. Empty: any device that has the key."
      >
        <TextField
          id="rtmp-in-allowed"
          mono
          placeholder="Any address"
          bind:value={input.rtmp.allowed_from}
        />
      </Field>
    </Card>
  {/if}

  {#if input.mode === "uvc"}
    <Card title="USB camera">
      {#snippet aside()}
        <button
          type="button"
          class={btn.plain}
          disabled={scanning}
          onclick={scan}
        >
          {scanning ? "Looking…" : "Refresh"}
        </button>
      {/snippet}

      {#if summary}
        <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
      {/if}
      {#if status?.mode === "uvc" && status.uvc.encoder === "software"}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          The hardware encoder isn't available, so encoding runs on the CPU, at
          720p at most. Usually that's too little GPU memory: add <code
            class="font-mono">gpu_mem=256</code
          >
          to <code class="font-mono">/boot/firmware/config.txt</code> and reboot.
        </p>
      {/if}
      {#if devicesError}
        <p class="px-4 py-3 text-[12px] text-down">{devicesError}</p>
      {/if}

      <Field label="Device" for="uvc-device">
        <Select
          id="uvc-device"
          options={deviceOptions}
          bind:value={input.uvc.device}
        />
      </Field>

      <Field label="Resolution" for="uvc-mode">
        <Select
          id="uvc-mode"
          options={modeOptions}
          bind:value={input.uvc.mode}
        />
      </Field>

      <Field
        label="Bitrate (kbit/s)"
        for="uvc-bitrate"
        help={passthrough
          ? "This device sends H.264 itself, so its own bitrate applies."
          : "About 6000 for 1080p, 2500 for an analog receiver."}
        disabled={passthrough}
      >
        <TextField
          id="uvc-bitrate"
          type="number"
          min={500}
          max={20000}
          step={500}
          mono
          bind:value={input.uvc.bitrate_kbps}
          disabled={passthrough}
        />
      </Field>

      <Field
        label="Keyframe every (s)"
        for="uvc-keyframe"
        disabled={passthrough}
      >
        <TextField
          id="uvc-keyframe"
          type="number"
          min={1}
          max={10}
          mono
          bind:value={input.uvc.keyframe_secs}
          disabled={passthrough}
        />
      </Field>
    </Card>
  {/if}
</div>
