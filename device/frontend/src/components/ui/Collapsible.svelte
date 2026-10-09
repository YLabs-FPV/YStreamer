<script lang="ts">
  import { slide } from "svelte/transition";
  import type { Snippet } from "svelte";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";

  interface Props {
    title: string;
    summary?: string;
    id?: string;
    open?: boolean;
    /** The summary may be out of date */
    stale?: boolean;
    children: Snippet;
  }

  let {
    title,
    summary,
    id,
    open: initial = true,
    stale = false,
    children,
  }: Props = $props();

  const key = $derived(id ? `ystreamer.panel.${id}` : null);

  function restore(): boolean {
    if (!key) return initial;
    try {
      const v = localStorage.getItem(key);
      return v === null ? initial : v === "1";
    } catch {
      return initial;
    }
  }

  let open = $state(restore());

  function toggle() {
    open = !open;
    if (!key) return;
    try {
      localStorage.setItem(key, open ? "1" : "0");
    } catch {}
  }
</script>

<section class="border-t border-line first:border-t-0">
  <h3>
    <button
      type="button"
      onclick={toggle}
      aria-expanded={open}
      class="flex w-full items-center gap-3 px-4 py-2 text-left transition-colors hover:bg-surface-2/60 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary"
    >
      <ChevronRight
        class="h-3.5 w-3.5 shrink-0 text-faint transition-transform duration-200 {open
          ? 'rotate-90'
          : ''}"
        aria-hidden="true"
      />
      <span class="text-[13px] font-medium text-fg">{title}</span>
      {#if summary}
        <span
          class="ml-auto truncate font-mono text-[11px] text-faint tabular-nums transition-opacity duration-200 {open
            ? 'opacity-0'
            : stale
              ? 'opacity-45'
              : 'opacity-100'}"
        >
          {summary}
        </span>
      {/if}
    </button>
  </h3>

  {#if open}
    <div transition:slide={{ duration: 180 }}>
      <div class="flex flex-col gap-2 px-4 pb-3">
        {@render children()}
      </div>
    </div>
  {/if}
</section>
