<script lang="ts">
  import { api } from "@/api/client";
  import type { RtmpStatus, Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";

  let {
    rtmp = $bindable(),
    saved,
  }: { rtmp: Settings["rtmp"]; saved: Settings["rtmp"] } = $props();

  const POLL_MS = 2000;

  const SERVICES = [
    { label: "YouTube", value: "rtmp://a.rtmp.youtube.com/live2" },
    { label: "Twitch", value: "rtmp://live.twitch.tv/app" },
    { label: "Facebook", value: "rtmps://live-api-s.facebook.com:443/rtmp/" },
    { label: "Custom server", value: "" },
  ];

  const service = $derived(
    SERVICES.find((s) => s.value && s.value === rtmp.url.trim())?.value ?? "",
  );

  function pickService(value: string | number) {
    rtmp.url = String(value);
    if (!value) document.getElementById("rtmp-url")?.focus();
  }

  let status = $state<RtmpStatus | null>(null);

  $effect(() => {
    if (!saved.enabled) {
      status = null;
      return;
    }
    const poll = async () => {
      try {
        status = await api.rtmp();
      } catch {
        status = null;
      }
    };
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => clearInterval(timer);
  });

  const uptime = (s: number) => {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = String(s % 60).padStart(2, "0");
    return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
  };

  const summary = $derived.by(() => {
    if (!status) return null;
    switch (status.state) {
      case "live":
        return {
          text: `Live · ${uptime(status.uptime_secs)} · ${status.bitrate_kbps} kbps`,
          tone: "text-ok",
        };
      case "connecting":
        return { text: "Connecting…", tone: "text-warn" };
      case "retrying":
        return {
          text: `${status.error ?? "Connection lost"}. Retrying…`,
          tone: "text-down",
        };
      default:
        return null;
    }
  });
</script>

<Card title="RTMP">
  {#snippet aside()}
    <Switch bind:checked={rtmp.enabled} label="RTMP" />
  {/snippet}

  {#if summary}
    <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
  {/if}

  <Field label="Service" for="rtmp-service" disabled={!rtmp.enabled}>
    <Select
      id="rtmp-service"
      options={SERVICES}
      value={service}
      onchange={pickService}
      disabled={!rtmp.enabled}
    />
  </Field>

  <Field
    label="Server URL"
    for="rtmp-url"
    help="rtmp:// or rtmps://, without the stream key."
    disabled={!rtmp.enabled}
  >
    <TextField
      id="rtmp-url"
      mono
      placeholder="rtmp://server/app"
      bind:value={rtmp.url}
      disabled={!rtmp.enabled}
    />
  </Field>

  <Field label="Stream key" for="rtmp-key" disabled={!rtmp.enabled}>
    <TextField
      id="rtmp-key"
      secret
      bind:value={rtmp.key}
      disabled={!rtmp.enabled}
    />
  </Field>

  <Field
    label="Silent audio track"
    for="rtmp-audio"
    help="The goggles send no sound, but some platforms refuse or flag streams without audio."
    disabled={!rtmp.enabled}
  >
    <Switch
      id="rtmp-audio"
      bind:checked={rtmp.silent_audio}
      disabled={!rtmp.enabled}
    />
  </Field>
</Card>
