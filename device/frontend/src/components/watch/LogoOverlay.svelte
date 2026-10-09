<script lang="ts">
  interface Props {
    src: string;
    /** Width / height of the picture the logo sits on */
    aspect: number;
    x: number;
    y: number;
    sizePercent: number;
    opacityPercent: number;
  }

  let { src, aspect, x, y, sizePercent, opacityPercent }: Props = $props();

  let logoAspect = $state<number | null>(null);

  // Same rule as the device: a logo taller than the picture shrinks to fit
  const width = $derived(
    logoAspect === null
      ? 0
      : Math.min(sizePercent, (100 * logoAspect) / aspect),
  );
</script>

<!-- The outer box measures the player; the inner one is the picture inside it,
     letterboxed the way the video element does it -->
<div
  class="@container-size pointer-events-none absolute inset-0 grid place-items-center"
>
  <div
    class="relative"
    style="width:min(100cqw, calc(100cqh * {aspect}));height:min(100cqh, calc(100cqw / {aspect}))"
  >
    <!-- Moving it back by its own share turns left/top into a share of the
         room around the logo -->
    <img
      {src}
      alt=""
      class="absolute h-auto max-w-none select-none"
      class:invisible={logoAspect === null}
      style="width:{width}%;opacity:{opacityPercent /
        100};left:{x}%;top:{y}%;transform:translate(-{x}%,-{y}%)"
      onload={(e) => {
        const img = e.currentTarget as HTMLImageElement;
        logoAspect = img.naturalWidth / img.naturalHeight;
      }}
    />
  </div>
</div>
