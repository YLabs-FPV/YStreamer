<script lang="ts">
  import { onMount } from "svelte";
  import SunMoon from "@lucide/svelte/icons/sun-moon";
  import Sun from "@lucide/svelte/icons/sun";
  import Moon from "@lucide/svelte/icons/moon";

  const THEME_STORAGE_KEY = "theme-preference";
  const THEMES = { AUTO: "auto", LIGHT: "light", DARK: "dark" } as const;
  type Theme = (typeof THEMES)[keyof typeof THEMES];

  export let labels = {
    toggle: "Toggle theme",
    auto: "Auto",
    light: "Light",
    dark: "Dark",
  };

  let currentTheme: Theme = THEMES.AUTO;
  let isDropdownVisible = false;

  function applyTheme(theme: Theme) {
    localStorage.setItem(THEME_STORAGE_KEY, theme);
    currentTheme = theme;
    document.documentElement.classList.toggle("light", theme === THEMES.LIGHT);
    document.documentElement.classList.toggle("dark", theme === THEMES.DARK);
    if (theme === THEMES.AUTO) {
      document.documentElement.classList.remove("light", "dark");
    }
  }

  function toggleDropdown(event: MouseEvent) {
    event.stopPropagation();
    isDropdownVisible = !isDropdownVisible;
  }

  function selectTheme(theme: Theme) {
    applyTheme(theme);
    isDropdownVisible = false;
  }

  onMount(() => {
    const saved = localStorage.getItem(THEME_STORAGE_KEY) as Theme | null;
    applyTheme(saved ?? THEMES.AUTO);
    const handleClickOutside = () => (isDropdownVisible = false);
    document.addEventListener("click", handleClickOutside);
    return () => document.removeEventListener("click", handleClickOutside);
  });

  const options: { value: Theme; label: string; icon: typeof Sun }[] = [
    { value: THEMES.AUTO, label: labels.auto, icon: SunMoon },
    { value: THEMES.LIGHT, label: labels.light, icon: Sun },
    { value: THEMES.DARK, label: labels.dark, icon: Moon },
  ];
</script>

<div class="relative">
  <button
    class="flex items-center justify-center p-2 rounded-full text-[rgb(var(--color-text))] hover:bg-[rgb(var(--color-secondary-hover))] transition-colors"
    aria-label={labels.toggle}
    on:click={toggleDropdown}
  >
    {#if currentTheme === THEMES.LIGHT}
      <Sun class="w-5 h-5" />
    {:else if currentTheme === THEMES.DARK}
      <Moon class="w-5 h-5" />
    {:else}
      <SunMoon class="w-5 h-5" />
    {/if}
  </button>

  {#if isDropdownVisible}
    <div
      class="absolute right-0 mt-2 py-2 w-40 bg-[rgb(var(--color-card-bg))] rounded-lg shadow-lg border border-[rgb(var(--color-border))] z-20"
    >
      {#each options as option}
        <button
          on:click={() => selectTheme(option.value)}
          class="w-full text-left px-4 py-2 hover:bg-[rgb(var(--color-secondary-hover))] transition-colors flex items-center {currentTheme ===
          option.value
            ? 'bg-[rgb(var(--color-secondary-hover))] font-medium'
            : ''}"
        >
          <svelte:component this={option.icon} class="w-4 h-4 mr-2" />
          {option.label}
        </button>
      {/each}
    </div>
  {/if}
</div>
