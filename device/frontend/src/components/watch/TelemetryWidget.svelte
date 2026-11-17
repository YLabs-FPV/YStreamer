<script lang="ts">
  import Navigation2 from "@lucide/svelte/icons/navigation-2";
  import Battery from "@/components/ui/Battery.svelte";
  import { control } from "@/stores/control.svelte";

  const t = $derived(control.telemetry);

  const BLANK = "—";
  const POINTS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

  const metres = (m: number | null) =>
    m === null ? BLANK : `${m.toFixed(1)} m`;
  const speed = (v: number | null) =>
    v === null ? BLANK : `${Math.abs(v).toFixed(1)} m/s`;
  const distance = (m: number) =>
    m >= 1000 ? `${(m / 1000).toFixed(2)} km` : `${Math.round(m)} m`;
  const point = (deg: number) => POINTS[Math.round(deg / 45) % 8];

  const gps = $derived.by(() => {
    if (!t || t.gps_fix === null) return BLANK;
    if (t.satellites !== null) return `${t.satellites} sats`;
    if (t.gps_level !== null) return `Level ${t.gps_level}`;
    return t.gps_fix ? "Fix" : "No fix";
  });
  const noFix = $derived(t?.gps_fix === false);

  const climb = $derived(t?.vertical_speed_mps ?? null);

  const home = $derived.by(() => {
    if (!t || t.home_set === null) return BLANK;
    if (!t.home_set) return "Not set";
    return t.home_distance_m === null ? BLANK : distance(t.home_distance_m);
  });
</script>

{#if t}
  <section
    class="overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
  >
    <header
      class="flex items-center justify-between gap-3 border-b border-line bg-surface-2/40 px-4 py-3"
    >
      <h2 class="truncate text-[13px] font-medium text-fg">
        {control.aircraft?.name ?? "Aircraft"}
        <span class="font-normal text-muted">· telemetry</span>
      </h2>
      <Battery percent={t.battery_percent} label="Aircraft battery" />
    </header>

    <dl class="grid grid-cols-3 gap-x-4 gap-y-3 px-4 py-3">
      {@render stat("Altitude", metres(t.altitude_m))}
      {@render stat(
        "Vertical",
        climb === null
          ? BLANK
          : `${climb > 0 ? "↑" : climb < 0 ? "↓" : ""}${speed(climb)}`,
      )}
      {@render stat("Speed", speed(t.horizontal_speed_mps))}
      {@render stat("GPS", gps, noFix)}
      <div class="col-span-2">
        <dt class="text-[11px] tracking-wide text-faint uppercase">Home</dt>
        <dd
          class="flex items-center gap-1.5 font-mono text-[13px] text-fg tabular-nums"
        >
          {home}
          {#if t.home_bearing_deg !== null}
            <!-- Relative to where the aircraft points, when it says;
                 otherwise north up -->
            <Navigation2
              class="h-3.5 w-3.5 shrink-0 text-primary"
              style="transform: rotate({t.home_bearing_deg -
                (t.yaw_deg ?? 0)}deg)"
              aria-hidden="true"
            />
            <span class="text-muted">
              {point(t.home_bearing_deg)}
              {Math.round(t.home_bearing_deg)}°
            </span>
          {/if}
        </dd>
      </div>
    </dl>
  </section>
{/if}

{#snippet stat(label: string, value: string, warn = false)}
  <div class="min-w-0">
    <dt class="text-[11px] tracking-wide text-faint uppercase">{label}</dt>
    <dd
      class="truncate font-mono text-[13px] tabular-nums {warn
        ? 'text-warn'
        : 'text-fg'}"
    >
      {value}
    </dd>
  </div>
{/snippet}
