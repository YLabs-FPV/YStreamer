<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { control } from "@/stores/control.svelte";
  import Collapsible from "@/components/ui/Collapsible.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import Slider from "@/components/ui/Slider.svelte";
  import { btn } from "@/components/ui/buttons";
  import { BUTTON, Presses, firstPad, rightStick } from "@/lib/gamepad";

  let { locked, stale }: { locked: boolean; stale: boolean } = $props();

  // What the device accepts; see input/duml/gimbal.rs and avata360.rs
  const TILT_MIN = -90;
  const TILT_MAX = 50;
  const FOV_MIN = 104;
  const FOV_MAX = 272;
  // Degrees per second with the stick all the way over
  const TILT_RATE = 60;
  const HEADING_RATE = 90;
  // Well inside the device's 1.5 s, after which the remote gets it back
  const RESEND_MS = 100;

  const VISION_ASSIST = [
    { label: "Off", value: false },
    { label: "On", value: true },
  ];

  // The aircraft doesn't report these back, so they start unknown
  let held = $state(false);
  let tilt = $state(0);
  let heading = $state<number | null>(null);
  let fov = $state<number | null>(null);
  let visionAssist = $state<boolean | null>(null);
  let pad = $state<string | null>(null);

  const deg = (v: number) => `${v > 0 ? "+" : ""}${Math.round(v)}°`;
  const clamp = (v: number, lo: number, hi: number) =>
    Math.min(hi, Math.max(lo, v));
  const wrap = (v: number) => ((((v + 180) % 360) + 360) % 360) - 180;

  let resend: ReturnType<typeof setInterval> | null = null;

  function take() {
    if (held || locked) return;
    held = true;
    control.gimbalTilt(tilt);
    resend = setInterval(() => control.gimbalTilt(tilt), RESEND_MS);
  }

  function handBack() {
    if (resend) clearInterval(resend);
    resend = null;
    if (!held) return;
    held = false;
    control.gimbalRelease();
  }

  function setHeading(v: number) {
    heading = v;
    control.setViewHeading(v);
  }

  // Losing the link, the permission or the setting hands the gimbal back
  $effect(() => {
    if (locked || !control.gimbalControl || control.avata360) untrack(handBack);
  });

  $effect(() => {
    const hidden = () => document.hidden && handBack();
    document.addEventListener("visibilitychange", hidden);
    window.addEventListener("pagehide", handBack);
    return () => {
      document.removeEventListener("visibilitychange", hidden);
      window.removeEventListener("pagehide", handBack);
    };
  });

  onDestroy(handBack);

  // The Gamepad API has no events for sticks, so it's read every frame
  $effect(() => {
    if (locked || !control.gimbalControl) return;
    const presses = new Presses();
    let last = performance.now();
    let sentAt = 0;
    let unsent = false;
    let frame = requestAnimationFrame(function step(now) {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const p = firstPad();
      pad = p ? p.id.split(" (")[0] : null;
      if (p) {
        const stick = rightStick(p);
        if (control.avata360) {
          if (presses.pressed(p, BUTTON.center)) {
            heading = 0;
            unsent = true;
          } else if (stick.x) {
            heading = wrap((heading ?? 0) + stick.x * HEADING_RATE * dt);
            unsent = true;
          }
          // The aircraft answers each one, so not one per frame
          if (unsent && now - sentAt > RESEND_MS && heading !== null) {
            control.setViewHeading(Math.round(heading));
            sentAt = now;
            unsent = false;
          }
        } else {
          if (presses.pressed(p, BUTTON.hold)) {
            if (held) handBack();
            else take();
          }
          if (presses.pressed(p, BUTTON.release)) handBack();
          if (held) {
            if (presses.pressed(p, BUTTON.center)) tilt = 0;
            tilt = clamp(tilt + stick.y * TILT_RATE * dt, TILT_MIN, TILT_MAX);
          }
        }
      }
      frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  });

  const gimbalSummary = $derived(
    control.avata360
      ? `Heading ${heading === null ? "—" : deg(heading)}`
      : held
        ? `Holding · ${deg(tilt)}`
        : "Remote",
  );
  const lensSummary = $derived(
    `${fov === null ? "—" : `${fov}°`} · Vision Assist ${
      visionAssist === null ? "—" : visionAssist ? "on" : "off"
    }`,
  );
  const padHelp = $derived(
    pad === null
      ? "A gamepad works too: press a button on it to connect it."
      : control.avata360
        ? `${pad}: the right stick turns the view, A faces it forward.`
        : `${pad}: Y takes or hands back the gimbal, the right stick tilts, A levels it, B hands it back.`,
  );
</script>

{#if control.gimbalControl}
  <Collapsible id="gimbal" title="Gimbal" summary={gimbalSummary} {stale}>
    {#if control.avata360}
      <Slider
        label="View heading"
        value={heading}
        min={-180}
        max={180}
        format={deg}
        minLabel="Left"
        maxLabel="Right"
        disabled={locked}
        onchange={setHeading}
      />
    {:else}
      <div class="flex items-center justify-between gap-3">
        <p class="text-[12px] text-muted">
          {held
            ? "The browser has the gimbal, so the remote can't move it."
            : "The remote has the gimbal."}
        </p>
        <button
          type="button"
          class={held ? btn.plain : btn.primary}
          disabled={locked && !held}
          onclick={() => (held ? handBack() : take())}
        >
          {held ? "Hand back" : "Take control"}
        </button>
      </div>
      <Slider
        label="Tilt"
        value={tilt}
        min={TILT_MIN}
        max={TILT_MAX}
        format={deg}
        minLabel="Down"
        maxLabel="Up"
        disabled={!held}
        onchange={(v) => (tilt = v)}
      />
    {/if}
    <p class="text-[11px] text-faint">{padHelp}</p>
  </Collapsible>
{/if}

{#if control.avata360}
  <Collapsible
    id="avata360-lens"
    title="Avata 360 lens"
    summary={lensSummary}
    {stale}
  >
    <Slider
      label="Field of view"
      value={fov}
      min={FOV_MIN}
      max={FOV_MAX}
      format={(v) => `${v}°`}
      minLabel="Flat"
      maxLabel="Whole sphere"
      disabled={locked}
      onchange={(v) => {
        fov = v;
        control.setLensFov(v);
      }}
    />
    <Segmented
      label="Vision Assist"
      options={VISION_ASSIST}
      value={visionAssist}
      disabled={locked}
      onchange={(v) => {
        visionAssist = v;
        control.setVisionAssist(v);
      }}
    />
  </Collapsible>
{/if}
