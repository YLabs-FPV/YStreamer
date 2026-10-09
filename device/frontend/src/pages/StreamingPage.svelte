<script lang="ts">
  import { copyText } from "@/lib/clipboard";
  import type { Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";
  import SrtCard from "@/components/streaming/SrtCard.svelte";
  import RtmpCard from "@/components/streaming/RtmpCard.svelte";
  import UdpCard from "@/components/streaming/UdpCard.svelte";
  let {
    rtsp = $bindable(),
    srt = $bindable(),
    rtmp = $bindable(),
    udp = $bindable(),
    viewer = $bindable(),
    saved,
    savedSrt,
    savedRtmp,
    savedUdp,
    tab,
  }: {
    rtsp: Settings["rtsp"];
    srt: Settings["srt"];
    rtmp: Settings["rtmp"];
    udp: Settings["udp"];
    viewer: Settings["viewer"];
    saved: Settings["rtsp"];
    savedSrt: Settings["srt"];
    savedRtmp: Settings["rtmp"];
    savedUdp: Settings["udp"];
    tab: string;
  } = $props();

  const VIEWERS = [
    { label: "Unlimited", value: 0 },
    ...[1, 2, 3, 4].map((n) => ({ label: String(n), value: n })),
  ];

  const TRANSPORTS = [
    { label: "WebSocket", value: "websocket" },
    { label: "WebRTC", value: "webrtc" },
  ];

  const url = $derived(
    `rtsp://${saved.auth ? `${encodeURIComponent(saved.username)}:••••@` : ""}${location.hostname}:${saved.port}${saved.path}`,
  );

  async function copyUrl() {
    const real = `rtsp://${saved.auth ? `${encodeURIComponent(saved.username)}:${encodeURIComponent(saved.password)}@` : ""}${location.hostname}:${saved.port}${saved.path}`;
    try {
      await copyText(real);
      snackbar.show("RTSP URL copied");
    } catch {
      snackbar.show("Couldn't copy; select the URL and copy it by hand", true);
    }
  }
</script>

<div class="flex flex-col gap-4">
  {#if tab === "rtsp"}
    <Card title="RTSP server">
      {#snippet aside()}
        <Switch bind:checked={rtsp.enabled} label="RTSP server" />
      {/snippet}

      {#if saved.enabled}
        <div class="flex items-center gap-2 px-4 py-3">
          <code
            class="min-w-0 flex-1 truncate rounded-md bg-surface-2 px-3 py-1.5 font-mono text-[13px] text-fg select-all"
          >
            {url}
          </code>
          <button type="button" class={btn.plain} onclick={copyUrl}>Copy</button
          >
        </div>
      {/if}

      <Field label="Port" for="rtsp-port" disabled={!rtsp.enabled}>
        <TextField
          id="rtsp-port"
          type="number"
          min={1}
          max={65535}
          mono
          bind:value={rtsp.port}
          disabled={!rtsp.enabled}
        />
      </Field>
      <Field label="Path" for="rtsp-path" disabled={!rtsp.enabled}>
        <TextField
          id="rtsp-path"
          mono
          bind:value={rtsp.path}
          disabled={!rtsp.enabled}
        />
      </Field>
      <Field
        label="TCP only"
        for="rtsp-tcp"
        help="More latency, but survives lossy WiFi and firewalls."
        disabled={!rtsp.enabled}
      >
        <Switch
          id="rtsp-tcp"
          bind:checked={rtsp.tcp_only}
          disabled={!rtsp.enabled}
        />
      </Field>
      <Field
        label="Require password"
        for="rtsp-auth"
        help="Sent unencrypted."
        disabled={!rtsp.enabled}
      >
        <Switch
          id="rtsp-auth"
          bind:checked={rtsp.auth}
          disabled={!rtsp.enabled}
        />
      </Field>
      {#if rtsp.auth}
        <Field label="Username" for="rtsp-user" disabled={!rtsp.enabled}>
          <TextField
            id="rtsp-user"
            bind:value={rtsp.username}
            disabled={!rtsp.enabled}
          />
        </Field>
        <Field label="Password" for="rtsp-pass" disabled={!rtsp.enabled}>
          <TextField
            id="rtsp-pass"
            secret
            bind:value={rtsp.password}
            disabled={!rtsp.enabled}
          />
        </Field>
      {/if}
    </Card>
  {:else if tab === "srt"}
    <SrtCard bind:srt saved={savedSrt} />
  {:else if tab === "rtmp"}
    <RtmpCard bind:rtmp saved={savedRtmp} />
  {:else if tab === "udp"}
    <UdpCard bind:udp saved={savedUdp} />
  {:else if tab === "browser"}
    <Card title="Browser viewing">
      <Field
        label="Transport"
        help="WebRTC: lowest latency. WebSocket: steadier frame rate."
        stacked
      >
        <div class="w-full sm:w-64">
          <Segmented
            options={TRANSPORTS}
            value={viewer.transport}
            onchange={(v) => (viewer.transport = v)}
          />
        </div>
      </Field>
      <Field
        label="Viewer limit"
        for="viewer-max"
        help="Every viewer costs CPU. Capping them keeps HDMI output and RTSP smooth."
      >
        <Select
          id="viewer-max"
          options={VIEWERS}
          bind:value={viewer.max_viewers}
        />
      </Field>
    </Card>
  {/if}
</div>
