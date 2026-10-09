<script lang="ts">
  import { onMount, onDestroy, untrack, flushSync } from "svelte";
  import { statusLabel } from "@/lib/status";
  import type { PlayerHistory, PlayerStat, Status } from "@/types/stream";
  import { recording, duration } from "@/stores/recording.svelte";
  import { auth } from "@/stores/auth.svelte";
  import { settings } from "@/stores/settings.svelte";
  import type { ViewerTransport } from "@/types/settings";
  import { SocketStream, socketStreamSupported } from "@/lib/socketStream";
  import { api } from "@/api/client";
  import { control } from "@/stores/control.svelte";
  import LogoOverlay from "@/components/watch/LogoOverlay.svelte";
  import BarChart3 from "@lucide/svelte/icons/bar-chart-3";
  import Maximize2 from "@lucide/svelte/icons/maximize-2";
  import Minimize2 from "@lucide/svelte/icons/minimize-2";
  import PictureInPicture2 from "@lucide/svelte/icons/picture-in-picture-2";
  import RectangleHorizontal from "@lucide/svelte/icons/rectangle-horizontal";

  let {
    status = $bindable("connecting"),
    theater = $bindable(false),
    showStats = $bindable(false),
    stats = $bindable([]),
    history = $bindable({ t: [], kbps: [], bufferMs: [] }),
  }: {
    status?: Status;
    theater?: boolean;
    showStats?: boolean;
    stats?: PlayerStat[];
    history?: PlayerHistory;
  } = $props();

  const HISTORY = 120;
  let kbpsNow: number | null = null;
  let bufferMsNow: number | null = null;

  function record() {
    history = {
      t: [...history.t, Date.now()].slice(-HISTORY),
      kbps: [...history.kbps, kbpsNow].slice(-HISTORY),
      bufferMs: [...history.bufferMs, bufferMsNow].slice(-HISTORY),
    };
  }

  let videoEl: HTMLVideoElement;
  let stageEl: HTMLDivElement;
  let pc: RTCPeerConnection | null = null;
  let socket: SocketStream | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let statsTimer: ReturnType<typeof setInterval> | null = null;

  let resolution = $state("—");
  let fps = $state("—");
  let bitrate = $state("—");
  let buffer = $state("—");
  let loss = $state("—");

  let lastBytes = 0;
  let lastStatsAt = 0;
  let lastDelay = 0;
  let lastEmitted = 0;
  let lastLost = 0;
  let lastReceived = 0;

  let isFullscreen = $state(false);
  let aspect = $state(16 / 9);
  let isPip = $state(false);
  const canPip = !!document.pictureInPictureEnabled;
  showStats = readPref("ystreamer.hud", false);
  theater = readPref("ystreamer.theater", false);

  function readPref(key: string, fallback: boolean) {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : v === "1";
    } catch {
      return fallback;
    }
  }

  function savePref(key: string, on: boolean) {
    try {
      localStorage.setItem(key, on ? "1" : "0");
    } catch {}
  }

  export function toggleStats() {
    showStats = !showStats;
    savePref("ystreamer.hud", showStats);
  }

  export function toggleTheater() {
    const flip = () => {
      theater = !theater;
      savePref("ystreamer.theater", theater);
      flushSync();
    };
    if (document.startViewTransition) document.startViewTransition(flip);
    else flip();
  }

  export function togglePip() {
    if (!canPip) return;
    if (document.pictureInPictureElement) {
      document.exitPictureInPicture().catch(() => {});
    } else {
      videoEl.requestPictureInPicture().catch(() => {});
    }
  }

  const isLive = $derived(status === "playing");
  const label = $derived(statusLabel(status));

  // Admins wait for the settings, so the first connection already uses the
  // right transport. Viewers can't read those and go by the login status
  const transport = $derived<ViewerTransport | null>(
    settings.saved
      ? settings.saved.viewer.transport
      : !auth.admin || settings.loadError
        ? auth.transport
        : null,
  );

  $effect(() => {
    stats = [
      { k: "Res", label: "Resolution", v: resolution },
      { k: "FPS", label: "Frame rate", v: fps },
      { k: "Rate", label: "Bitrate", v: bitrate },
      { k: "Buffer", label: "Buffer", v: buffer },
      // TCP never loses packets; late frames are dropped on the Pi instead
      ...(transport === "websocket"
        ? []
        : [{ k: "Loss", label: "Packet loss", v: loss }]),
      {
        k: "Via",
        label: "Transport",
        v: transport === "websocket" ? "WebSocket" : "WebRTC",
      },
    ];
  });

  async function waitForIceGathering(p: RTCPeerConnection): Promise<void> {
    if (p.iceGatheringState === "complete") return;
    await new Promise<void>((resolve) => {
      const check = () => {
        if (p.iceGatheringState === "complete") {
          p.removeEventListener("icegatheringstatechange", check);
          resolve();
        }
      };
      p.addEventListener("icegatheringstatechange", check);
    });
  }

  function retry(delay = 1500) {
    cleanup();
    retryTimer = setTimeout(start, delay);
  }

  async function pollStats() {
    if (!pc) return;
    const report = await pc.getStats();
    report.forEach((s: any) => {
      if (s.type !== "inbound-rtp" || s.kind !== "video") return;
      if (s.frameWidth && s.frameHeight) {
        resolution = `${s.frameWidth}×${s.frameHeight}`;
      }
      if (typeof s.framesPerSecond === "number") {
        fps = `${Math.round(s.framesPerSecond)}`;
      }
      const now = s.timestamp as number;
      if (lastStatsAt && s.bytesReceived >= lastBytes) {
        const bps =
          ((s.bytesReceived - lastBytes) * 8) / ((now - lastStatsAt) / 1000);
        bitrate = formatBitrate(bps);
        kbpsNow = bps / 1000;
      }
      lastBytes = s.bytesReceived;
      lastStatsAt = now;

      const dCount = (s.jitterBufferEmittedCount ?? 0) - lastEmitted;
      const dDelay = (s.jitterBufferDelay ?? 0) - lastDelay;
      if (lastEmitted && dCount > 0 && dDelay >= 0) {
        bufferMsNow = (dDelay / dCount) * 1000;
        buffer = `${Math.round(bufferMsNow)} ms`;
      }
      lastEmitted = s.jitterBufferEmittedCount ?? 0;
      lastDelay = s.jitterBufferDelay ?? 0;

      const dLost = (s.packetsLost ?? 0) - lastLost;
      const dRecv = (s.packetsReceived ?? 0) - lastReceived;
      if (lastReceived && dRecv >= 0 && dLost >= 0 && dRecv + dLost > 0) {
        loss = `${((dLost / (dRecv + dLost)) * 100).toFixed(1)}%`;
      }
      lastLost = s.packetsLost ?? 0;
      lastReceived = s.packetsReceived ?? 0;
    });
    record();
  }

  function formatBitrate(bps: number) {
    return bps > 1_000_000
      ? `${(bps / 1_000_000).toFixed(1)} Mb/s`
      : `${Math.round(bps / 1000)} kb/s`;
  }

  function start() {
    retryTimer = null;
    if (status !== "busy") status = "negotiating";
    if (transport === "websocket" && socketStreamSupported()) {
      startSocket();
    } else {
      startRtc();
    }
  }

  function startSocket() {
    socket = new SocketStream(videoEl, {
      onplaying: () => (status = "playing"),
      onclose: (busy) => {
        status = busy ? "busy" : "disconnected";
        retry(busy ? 5000 : 1500);
      },
    });
    statsTimer = setInterval(pollSocketStats, 1000);
  }

  function pollSocketStats() {
    if (!socket) return;
    const s = socket.stats();
    if (s.width && s.height) resolution = `${s.width}×${s.height}`;
    if (s.fps) fps = `${Math.round(s.fps)}`;
    bitrate = formatBitrate(s.bitsPerSecond);
    buffer = `${Math.round(s.buffered * 1000)} ms`;
    kbpsNow = s.bitsPerSecond / 1000;
    bufferMsNow = s.buffered * 1000;
    record();
  }

  async function startRtc() {
    pc = new RTCPeerConnection({ iceServers: [] });
    pc.addTransceiver("video", { direction: "recvonly" });

    pc.ontrack = (ev) => {
      videoEl.srcObject = ev.streams[0];
      // Nudge the browser toward low latency over smoothness
      const receiver = pc
        ?.getReceivers()
        .find((r) => r.track?.kind === "video");
      if (receiver && "playoutDelayHint" in receiver) {
        receiver.playoutDelayHint = 0;
      }
      status = "playing";
    };

    pc.onconnectionstatechange = () => {
      if (!pc) return;
      const s = pc.connectionState;
      if (s === "failed" || s === "disconnected" || s === "closed") {
        status = "disconnected";
        retry();
      }
    };

    try {
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await waitForIceGathering(pc);

      const res = await fetch("/api/offer", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(pc.localDescription),
      });
      if (res.status === 503) {
        status = "busy";
        retry(5000);
        return;
      }
      if (!res.ok) throw new Error("offer rejected");

      const answer = await res.json();
      await pc.setRemoteDescription(answer);
      // ontrack may already have fired during negotiation
      if ((status as Status) !== "playing") status = "connected";
      statsTimer = setInterval(pollStats, 1000);
    } catch {
      status = "error";
      retry();
    }
  }

  function cleanup() {
    if (retryTimer) clearTimeout(retryTimer);
    retryTimer = null;
    if (statsTimer) clearInterval(statsTimer);
    statsTimer = null;
    lastBytes = 0;
    lastStatsAt = 0;
    lastDelay = 0;
    lastEmitted = 0;
    lastLost = 0;
    lastReceived = 0;
    resolution = "—";
    fps = "—";
    bitrate = "—";
    buffer = "—";
    loss = "—";
    kbpsNow = null;
    bufferMsNow = null;
    try {
      pc?.close();
    } catch {}
    pc = null;
    socket?.close();
    socket = null;
  }

  export function reconnect() {
    status = "connecting";
    retry(0);
  }

  export function toggleFullscreen() {
    if (document.fullscreenElement) {
      document.exitFullscreen().catch(() => {});
    } else {
      stageEl?.requestFullscreen?.().catch(() => {});
    }
  }

  function onFullscreenChange() {
    isFullscreen = document.fullscreenElement === stageEl;
  }

  $effect(() => {
    if (!transport) return;
    untrack(reconnect);
  });

  onMount(() => {
    videoEl.addEventListener("enterpictureinpicture", () => (isPip = true));
    videoEl.addEventListener("leavepictureinpicture", () => (isPip = false));
    document.addEventListener("fullscreenchange", onFullscreenChange);
  });
  onDestroy(() => {
    document.removeEventListener("fullscreenchange", onFullscreenChange);
    cleanup();
  });
</script>

{#snippet button(
  kind: "stats" | "pip" | "theater" | "fullscreen",
  label: string,
  onclick: () => void,
  pressed?: boolean,
)}
  <button
    type="button"
    {onclick}
    title={label}
    aria-label={label}
    aria-pressed={pressed}
    class="grid h-9 w-9 place-items-center rounded-md text-white/85 transition-colors hover:bg-white/15 hover:text-white focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-white"
  >
    {#if kind === "stats"}
      <BarChart3 class="h-4.5 w-4.5" aria-hidden="true" />
    {:else if kind === "pip"}
      <PictureInPicture2 class="h-4.5 w-4.5" aria-hidden="true" />
    {:else if kind === "theater"}
      <RectangleHorizontal
        class={theater ? "h-3.5 w-3.5" : "h-4.5 w-4.5"}
        aria-hidden="true"
      />
    {:else if isFullscreen}
      <Minimize2 class="h-4.5 w-4.5" aria-hidden="true" />
    {:else}
      <Maximize2 class="h-4.5 w-4.5" aria-hidden="true" />
    {/if}
  </button>
{/snippet}

<div class="overflow-hidden rounded-xl border border-line bg-black shadow-sm">
  <!-- In theater mode a wide window would make the picture taller than the
       screen, so the height is capped and the picture letterboxed -->
  <div
    bind:this={stageEl}
    class="video-stage group relative mx-auto aspect-video w-full bg-black
      {theater && !isFullscreen ? 'max-h-[calc(100dvh-9rem)]' : ''}"
  >
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <video
      bind:this={videoEl}
      autoplay
      muted
      playsinline
      ondblclick={toggleFullscreen}
      onresize={() => {
        if (videoEl.videoWidth && videoEl.videoHeight)
          aspect = videoEl.videoWidth / videoEl.videoHeight;
      }}
      class="h-full w-full object-contain"
    ></video>

    {#if control.overlay?.visible}
      <LogoOverlay
        src={api.overlayUrl(control.overlay.version)}
        {aspect}
        x={control.overlay.x_percent}
        y={control.overlay.y_percent}
        sizePercent={control.overlay.size_percent}
        opacityPercent={control.overlay.opacity_percent}
      />
    {/if}

    {#if recording.active}
      <span
        class="absolute top-3 left-3 flex items-center gap-1.5 rounded-md bg-black/55 px-2 py-1 text-[10px] font-semibold tracking-[0.08em] text-white/90 uppercase backdrop-blur-sm"
      >
        <span class="h-1.5 w-1.5 animate-pulse rounded-full bg-down"></span>
        Rec {duration(recording.elapsed)}
      </span>
    {/if}

    <!-- The widget beside the player isn't there in full screen -->
    {#if showStats && isLive && isFullscreen}
      <dl
        class="absolute top-3 right-3 flex flex-wrap justify-end gap-1.5 text-[11px]"
      >
        {#each stats as s}
          <div
            class="flex items-baseline gap-1.5 rounded-md bg-black/55 px-2 py-1 backdrop-blur-sm"
          >
            <dt class="tracking-wide text-white/45 uppercase">{s.k}</dt>
            <dd class="font-mono text-white/90 tabular-nums">{s.v}</dd>
          </div>
        {/each}
      </dl>
    {/if}

    {#if !isLive}
      <div class="absolute inset-0 grid place-items-center bg-black">
        <span
          role="status"
          aria-label={label}
          class="h-8 w-8 animate-spin rounded-full border-2 border-white/15 border-t-white/70"
        ></span>
      </div>
    {/if}

    <div
      class="absolute inset-x-0 bottom-0 flex items-center justify-end gap-0.5 bg-linear-to-t from-black/70 to-transparent px-2 pt-8 pb-1.5 opacity-0 transition-opacity group-hover:opacity-100 has-focus-visible:opacity-100 [@media(hover:none)]:opacity-100"
    >
      {@render button(
        "stats",
        showStats ? "Hide stats (S)" : "Show stats (S)",
        toggleStats,
        showStats,
      )}
      {#if canPip}
        {@render button(
          "pip",
          isPip ? "Exit picture in picture (P)" : "Picture in picture (P)",
          togglePip,
          isPip,
        )}
      {/if}
      {#if !isFullscreen}
        <span class="hidden lg:contents">
          {@render button(
            "theater",
            theater ? "Default view (T)" : "Theater mode (T)",
            toggleTheater,
            theater,
          )}
        </span>
      {/if}
      {@render button(
        "fullscreen",
        isFullscreen ? "Exit full screen (F)" : "Full screen (F)",
        toggleFullscreen,
      )}
    </div>
  </div>
</div>
