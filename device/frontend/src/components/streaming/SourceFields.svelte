<script lang="ts">
  import type { NetworkSource } from "@/types/settings";
  import Field from "@/components/ui/Field.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let { source = $bindable(), id }: { source: NetworkSource; id: string } =
    $props();

  const HEIGHTS = [480, 540, 720, 1080].map((h) => ({
    label: `${h}p`,
    value: h,
  }));

  const RATES = [
    { label: "30 fps", value: 30 },
    { label: "60 fps", value: 60 },
  ];
</script>

<Field
  label="Source name"
  for="{id}-name"
  help="Shown as HOSTNAME (name) in the list of sources."
  disabled={!source.enabled}
>
  <TextField
    id="{id}-name"
    bind:value={source.name}
    disabled={!source.enabled}
  />
</Field>

<Field
  label="Picture size"
  for="{id}-height"
  help="Larger video is scaled down. Each picture is compressed again on the Pi, so bigger costs more processor."
  disabled={!source.enabled}
>
  <Select
    id="{id}-height"
    options={HEIGHTS}
    bind:value={source.max_height}
    disabled={!source.enabled}
  />
</Field>

<Field
  label="Frame rate"
  help="At most. 30 halves the work compared with 60."
  disabled={!source.enabled}
>
  <div class="w-full sm:w-64">
    <Segmented
      options={RATES}
      value={source.max_fps}
      disabled={!source.enabled}
      onchange={(v) => (source.max_fps = v)}
    />
  </div>
</Field>
