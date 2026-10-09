<script lang="ts">
  let { small = false }: { small?: boolean } = $props();
</script>

<span
  class="flex items-center font-semibold tracking-tight text-fg {small
    ? 'gap-2 text-[15px]'
    : 'gap-2.5 text-xl'}"
>
  <img src="/logo.svg" alt="" class={small ? "h-5 w-5" : "h-6 w-6"} />
  <div class="flex flex-col gap-y-0.5 leading-none">
    <span> YStreamer </span>
    <span class="text-xs font-normal text-muted"> by YarosFPV </span>
  </div>
</span>
