<script lang="ts">
  import type { NavPage, NavTab } from "@/lib/nav";
  import { settings } from "@/stores/settings.svelte";

  let {
    page,
    tabs,
    current,
  }: { page: NavPage; tabs: NavTab[]; current: string } = $props();

  const isDirty = (t: NavTab) =>
    t.sections.some((s) => settings.dirty.includes(s));
</script>

<nav
  aria-label="{page.label} sections"
  class="-mx-4 mb-5 flex overflow-x-auto px-4 shadow-[inset_0_-1px_0_var(--color-line)] sm:mx-0 sm:px-0"
>
  {#each tabs as t (t.id)}
    {@const active = t.id === current}
    <a
      href="/{page.id}/{t.id}"
      aria-current={active ? "page" : undefined}
      class="flex shrink-0 items-center gap-1.5 border-b-2 px-3 py-2 text-[13px] font-medium whitespace-nowrap transition-colors focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-primary
        {active
        ? 'border-primary text-fg'
        : 'border-transparent text-muted hover:border-line-strong hover:text-fg'}"
    >
      {#if settings.saved && t.on?.(settings.saved)}
        <span class="h-1.5 w-1.5 rounded-full bg-ok" title="Enabled"></span>
      {/if}
      {t.label}
      {#if isDirty(t)}
        <span class="h-1.5 w-1.5 rounded-full bg-accent" title="Unsaved changes"
        ></span>
      {/if}
    </a>
  {/each}
</nav>
