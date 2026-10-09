<script lang="ts">
  import {
    languages,
    type Language,
    getLocalizedUrl,
    getRouteFromUrl,
    saveLanguagePreference,
  } from "../i18n";

  export let currentLang: Language;
  export let label = "Change language";

  let showDropdown = false;

  function toggleDropdown() {
    showDropdown = !showDropdown;
  }

  function handleLanguageChange(newLang: Language) {
    saveLanguagePreference(newLang);
    const currentPath = getRouteFromUrl(new URL(window.location.href));
    window.location.href = getLocalizedUrl(newLang, currentPath);
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") showDropdown = false;
  }
</script>

<div class="relative">
  <button
    on:click={toggleDropdown}
    on:keydown={handleKeydown}
    class="flex items-center px-3 py-2 text-sm bg-[rgb(var(--color-secondary))] border border-[rgb(var(--color-border))] rounded-lg hover:bg-[rgb(var(--color-bg))] focus:outline-none focus:ring-2 focus:ring-[rgb(var(--color-primary))] focus:border-transparent transition-colors"
    aria-haspopup="true"
    aria-expanded={showDropdown}
    aria-label={label}
  >
    <span class="uppercase font-medium text-[rgb(var(--color-text))]">
      {currentLang}
    </span>
    <svg
      class="w-4 h-4 ml-2 transition-transform {showDropdown
        ? 'rotate-180'
        : ''}"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        stroke-width="2"
        d="M19 9l-7 7-7-7"
      />
    </svg>
  </button>

  {#if showDropdown}
    <!-- svelte-ignore a11y-click-events-have-key-events -->
    <!-- svelte-ignore a11y-no-static-element-interactions -->
    <div
      class="absolute right-0 mt-1 w-40 bg-[rgb(var(--color-bg))] border border-[rgb(var(--color-border))] rounded-lg shadow-lg z-20"
      on:click={() => (showDropdown = false)}
    >
      {#each Object.entries(languages) as [langCode, langName]}
        <button
          on:click={() => handleLanguageChange(langCode as Language)}
          class="w-full px-4 py-2 text-left text-sm hover:bg-[rgb(var(--color-secondary))] transition-colors first:rounded-t-lg last:rounded-b-lg {currentLang ===
          langCode
            ? 'bg-[rgb(var(--color-primary))] text-white'
            : 'text-[rgb(var(--color-text))]'}"
        >
          <span class="uppercase font-medium mr-2">{langCode}</span>
          {langName}
        </button>
      {/each}
    </div>
  {/if}
</div>

<svelte:window
  on:click={(e) => {
    const target = e.target as Element;
    if (!target.closest(".relative")) showDropdown = false;
  }}
/>
