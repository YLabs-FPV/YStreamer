/** @type {import("@sveltejs/vite-plugin-svelte").SvelteConfig} */
export default {
  compilerOptions: {
    // The video never has captions
    warningFilter: (warning) => warning.code !== "a11y_media_has_caption",
  },
};
