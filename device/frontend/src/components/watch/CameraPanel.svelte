<script lang="ts">
  import { control } from "@/stores/control.svelte";
  import { auth } from "@/stores/auth.svelte";
  import Collapsible from "@/components/ui/Collapsible.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import Slider from "@/components/ui/Slider.svelte";
  import Camera from "@lucide/svelte/icons/camera";
  import { snackbar } from "@/stores/snackbar.svelte";
  import {
    FPS,
    SHUTTERS,
    aspectBlocked,
    cameraModel,
    hzOf,
    listsRate,
    rateBlocked,
    resBlocked,
    shutterBlocked,
    type Aspect,
    type Res,
  } from "@/lib/cameraModels";

  const ISO = [0, 100, 200, 400, 800, 1600, 3200, 6400, 12800, 25600];
  const EV = [
    -3.0, -2.7, -2.3, -2.0, -1.7, -1.3, -1.0, -0.7, -0.3, 0.0, 0.3, 0.7, 1.0,
    1.3, 1.7, 2.0, 2.3, 2.7, 3.0,
  ];

  const STEPS = [-2, -1, 0, 1, 2];
  const NR_STEPS = [-2, -1, 0, 1];

  const MODE = [
    { label: "Auto", value: "auto" },
    { label: "Manual", value: "manual" },
  ];
  const model = $derived(control.aircraft?.code);
  const camera = $derived(cameraModel(model));
  const hz = $derived(hzOf(control.fps));
  const format = $derived({
    res: control.res as Res | null,
    ar: control.ar as Aspect | null,
    hz,
  });
  const RES = $derived(
    (["1080p", "2.7K", "4K"] as Res[]).map((res) => ({
      label: res,
      value: res,
      blocked: resBlocked(camera, res, format),
    })),
  );
  const AR = $derived(
    (["16:9", "4:3"] as Aspect[]).map((ar) => ({
      label: ar,
      value: ar,
      blocked: aspectBlocked(camera, ar, format),
    })),
  );
  const RATES = $derived(
    FPS.filter((f) => listsRate(camera, f.hz)).map((f) => ({
      label: String(f.hz),
      value: f.wire,
      blocked: rateBlocked(camera, f.hz, format),
    })),
  );

  const SHUTTER = $derived([
    0,
    ...SHUTTERS.filter((speed) => !shutterBlocked(speed, hz)).toReversed(),
  ]);
  const slowestShutter = $derived(SHUTTER.at(-1));
  const explain = (reason: string) => snackbar.show(reason);
  // The first-generation FPV air units (the Air Unit and the Lite one, as
  // in the Vista) take other commands for video format and image settings
  const noFormatOrImage = $derived(model === "WM150" || model === "LT150");

  // O4 Lites don't have D-Log/D-Cinelike
  const hasProfiles = $derived(model !== "ZA530");

  // The O3's flat profile is D-Cinelike, where later air units have D-Log
  const flatProfile = $derived(
    model?.startsWith("WM169") || control.colorProfile === "dcinelike"
      ? { label: "D-Cinelike", value: "dcinelike" }
      : { label: "D-Log", value: "dlog" },
  );
  const PROFILE = $derived([{ label: "Normal", value: "normal" }, flatProfile]);
  const ANTI_FLICKER = [
    { label: "Auto", value: "auto" },
    { label: "50 Hz", value: "50hz" },
    { label: "60 Hz", value: "60hz" },
    { label: "Off", value: "off" },
  ];
  const WB_MODE = [
    { label: "Auto", value: true },
    { label: "Manual", value: false },
  ];

  const isoText = (v: number) => (v === 0 ? "Auto" : String(v));
  const shutterText = (v: number) => (v === 0 ? "Auto" : `1/${v}`);
  const evText = (v: number) => (v > 0 ? `+${v.toFixed(1)}` : v.toFixed(1));
  const stepText = (v: number) => (v > 0 ? `+${v}` : String(v));
  const fpsText = (wire: number | null) => hzOf(wire) ?? "—";

  function snap(table: number[], v: number | null): number | null {
    if (v === null) return null;
    let best = table[0];
    for (const t of table) if (Math.abs(t - v) < Math.abs(best - v)) best = t;
    return best;
  }

  const isManual = $derived(control.exposureMode === "manual");
  const isAuto = $derived(control.exposureMode === "auto");
  const evValue = $derived(snap(EV, control.ev));

  const noGoggles = $derived(!control.connected || control.link !== "live");
  const off = $derived(noGoggles || !control.aircraft);
  const locked = $derived(off || !auth.admin);

  const exposureSummary = $derived(
    control.exposureMode === null
      ? "—"
      : isManual
        ? `${isoText(control.iso ?? 0)} · ${shutterText(control.shutter ?? 0)}`
        : `Auto · EV ${evValue === null ? "—" : evText(evValue)}`,
  );
  const wbSummary = $derived(
    control.wbAuto === null
      ? "—"
      : control.wbAuto
        ? control.wbTemp
          ? `Auto · ${control.wbTemp}K`
          : "Auto"
        : `${control.wbTemp ?? 5000}K`,
  );
  const formatSummary = $derived(
    `${control.res ?? "—"} · ${control.ar ?? "—"} · ${fpsText(control.fps)}`,
  );
  const profileText = $derived(
    PROFILE.find((p) => p.value === control.colorProfile)?.label ?? "—",
  );
  const imageSummary = $derived(
    `${hasProfiles ? `${profileText} · ` : ""}Sharp ${control.sharpness === null ? "—" : stepText(control.sharpness)} · NR ${
      control.noiseReduction === null ? "—" : stepText(control.noiseReduction)
    }`,
  );

  const WB_GRADIENT =
    "linear-gradient(to right, #6aa9ff 0%, #a8c8ff 22%, #ffffff 42%, #ffd9a0 68%, #ff9d3d 100%)";
</script>

<section
  class="@container overflow-hidden rounded-xl border border-line bg-surface shadow-sm"
>
  <div
    class="flex items-center justify-between border-b border-line bg-surface-2/40 px-4 py-3"
  >
    <div class="flex items-center gap-2">
      <Camera class="h-4 w-4 text-muted" aria-hidden="true" />
      <h2 class="text-[13px] font-medium text-fg">Camera</h2>
    </div>
  </div>

  {#if off}
    <p class="border-b border-line px-4 py-2.5 text-[12px] text-muted">
      {noGoggles
        ? "Connect the goggles to change camera settings."
        : "Waiting for the aircraft. Its camera can be set up once it's linked to the goggles."}
    </p>
  {/if}

  <div
    class="grid divide-line {noFormatOrImage
      ? ''
      : '@2xl:grid-cols-2 @2xl:divide-x'}"
  >
    <div class="min-w-0">
      <Collapsible
        id="exposure"
        title="Exposure"
        summary={exposureSummary}
        stale={off}
      >
        <Segmented
          options={MODE}
          value={control.exposureMode}
          disabled={locked}
          onchange={(v) => control.setExposureMode(v)}
        />

        <Slider
          label="ISO"
          values={ISO}
          value={control.iso}
          format={isoText}
          disabled={locked || isAuto}
          onchange={(v) => control.setIso(v)}
        />

        <Slider
          label="Shutter"
          values={SHUTTER}
          value={snap(SHUTTER, control.shutter)}
          format={shutterText}
          disabled={locked || isAuto}
          note={slowestShutter && slowestShutter > SHUTTERS[0]
            ? `1/${slowestShutter} is the slowest at ${hz} fps`
            : undefined}
          onchange={(v) => control.setShutter(v)}
        />

        <Slider
          label={isManual ? "Metered exposure" : "Exposure compensation"}
          values={EV}
          value={isManual ? snap(EV, control.evMetered) : evValue}
          format={evText}
          disabled={locked || isManual}
          onchange={(v) => control.setEv(v)}
        />
      </Collapsible>

      <Collapsible
        id="wb"
        title="White balance"
        summary={wbSummary}
        stale={off}
      >
        <Segmented
          options={WB_MODE}
          value={control.wbAuto}
          disabled={locked}
          onchange={(auto) =>
            auto
              ? control.setWb(true)
              : control.setWb(false, control.wbTemp ?? 5000)}
        />

        <Slider
          label="Colour temperature"
          value={control.wbTemp}
          min={2000}
          max={10000}
          step={100}
          format={(v) => `${v}K`}
          gradient={WB_GRADIENT}
          disabled={locked || control.wbAuto !== false}
          onchange={(v) => control.setWb(false, v)}
        />
      </Collapsible>
    </div>
    {#if !noFormatOrImage}
      <div class="min-w-0 border-t border-line @2xl:border-t-0">
        <Collapsible
          stale={off}
          id="format"
          title="Video format"
          summary={formatSummary}
          open={false}
        >
          <div class="grid grid-cols-2 gap-3">
            <Segmented
              label="Resolution"
              options={RES}
              value={control.res}
              disabled={locked}
              onblocked={explain}
              onchange={(res) =>
                control.setVideoFormat(
                  res,
                  control.ar ?? "16:9",
                  control.fps ?? 6,
                )}
            />
            <Segmented
              label="Aspect ratio"
              options={AR}
              value={control.ar}
              disabled={locked}
              onblocked={explain}
              onchange={(ar) =>
                control.setVideoFormat(
                  control.res ?? "1080p",
                  ar,
                  control.fps ?? 6,
                )}
            />
          </div>
          <Segmented
            label="Frame rate"
            options={RATES}
            value={control.fps}
            disabled={locked}
            onblocked={explain}
            onchange={(fps) =>
              control.setVideoFormat(
                control.res ?? "1080p",
                control.ar ?? "16:9",
                fps,
              )}
          />
        </Collapsible>

        <Collapsible
          stale={off}
          id="image"
          title="Image processing"
          summary={imageSummary}
          open={false}
        >
          {#if hasProfiles}
            <Segmented
              label="Color profile"
              options={PROFILE}
              value={control.colorProfile}
              disabled={locked}
              onchange={(v) => control.setColorProfile(v)}
            />
          {/if}
          <Segmented
            label="Anti-flicker"
            options={ANTI_FLICKER}
            value={control.antiFlicker}
            disabled={locked}
            onchange={(v) => control.setAntiFlicker(v)}
          />
          <Slider
            label="Sharpness"
            values={STEPS}
            value={control.sharpness}
            format={stepText}
            disabled={locked}
            onchange={(v) => control.setSharpness(v)}
          />
          <Slider
            label="Noise reduction"
            values={NR_STEPS}
            value={control.noiseReduction}
            format={stepText}
            disabled={locked}
            onchange={(v) => control.setNr(v)}
          />
        </Collapsible>
      </div>
    {/if}
  </div>
</section>
