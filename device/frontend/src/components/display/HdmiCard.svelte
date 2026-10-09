<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { DisplayStatus, Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let { display = $bindable() }: { display: Settings["display"] } = $props();

  const SCALING = [
    { label: "Fit", value: "fit" },
    { label: "Fill", value: "fill" },
  ];

  let status = $state<DisplayStatus | null>(null);
  let loadError = $state<string | null>(null);

  onMount(async () => {
    try {
      status = await api.display();
    } catch (e) {
      loadError = message(e);
    }
  });

  const hz = (r: number) =>
    Math.abs(r - Math.round(r)) < 0.02 ? String(Math.round(r)) : r.toFixed(2);

  const modes = $derived.by(() => {
    const list = (status?.modes ?? []).map((m) => {
      const notes = [
        m.preferred && "screen's choice",
        !m.fits_screen && "wrong shape, gets stretched",
        m.width >= 3840 && "may stutter",
      ].filter(Boolean);
      return {
        value: m.id,
        label: `${m.width} × ${m.height} · ${hz(m.refresh)} Hz${notes.length ? ` (${notes.join(", ")})` : ""}`,
      };
    });
    // A mode saved while another screen was plugged in
    if (display.mode !== "auto" && !list.some((m) => m.value === display.mode))
      list.unshift({
        value: display.mode,
        label: `${display.mode.replace("x", " × ").replace("@", " · ")} Hz (not offered by this screen)`,
      });
    return [{ value: "auto", label: "Automatic (best for video)" }, ...list];
  });

  const modeHelp = $derived(
    status?.error ??
      loadError ??
      "Automatic picks the sharpest mode up to 1080p, as close to 60 Hz as the screen goes. 4K is limited to 30 Hz.",
  );
</script>

<Card title="HDMI output">
  <Field
    label="Show video on HDMI"
    for="hdmi"
    help="Turn off when nothing is plugged in. Frees the decoder and some CPU for streaming."
  >
    <Switch id="hdmi" bind:checked={display.hdmi_output} />
  </Field>

  <Field
    label="Resolution"
    for="hdmi-mode"
    help={modeHelp}
    disabled={!display.hdmi_output}
  >
    <Select
      id="hdmi-mode"
      options={modes}
      bind:value={display.mode}
      disabled={!display.hdmi_output}
    />
  </Field>

  <Field label="Scaling" disabled={!display.hdmi_output}>
    <div class="w-full sm:w-64">
      <Segmented
        options={SCALING}
        value={display.scaling}
        disabled={!display.hdmi_output}
        onchange={(v) => (display.scaling = v)}
      />
    </div>
  </Field>

  <Field label="Rotate 180°" for="hdmi-rotate" disabled={!display.hdmi_output}>
    <Switch
      id="hdmi-rotate"
      bind:checked={display.rotate_180}
      disabled={!display.hdmi_output}
    />
  </Field>

  <Field label="Mirror" for="hdmi-mirror" disabled={!display.hdmi_output}>
    <Switch
      id="hdmi-mirror"
      bind:checked={display.mirror}
      disabled={!display.hdmi_output}
    />
  </Field>
</Card>
