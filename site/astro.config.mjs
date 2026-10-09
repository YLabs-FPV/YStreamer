// @ts-check
import { defineConfig } from "astro/config";
import tailwindcss from "@tailwindcss/vite";
import svelte from "@astrojs/svelte";
import icon from "astro-icon";
import sitemap from "@astrojs/sitemap";
import mdx from "@astrojs/mdx";

import cloudflare from "@astrojs/cloudflare";
import remarkLocalizeDocLinks from "./src/lib/remark-localize-doc-links.mjs";
import rehypeExternalLinks from "./src/lib/rehype-external-links.mjs";

export default defineConfig({
  site: "https://ystreamer.yarosfpv.com",
  compressHTML: true,

  markdown: {
    remarkPlugins: [remarkLocalizeDocLinks],
    rehypePlugins: [rehypeExternalLinks],
    // Both sets of colours go into the page as CSS variables; global.css
    // picks the one for the theme in use
    shikiConfig: {
      themes: { light: "github-light", dark: "github-dark" },
      defaultColor: false,
    },
  },
  integrations: [
    svelte(),
    icon(),
    sitemap({
      // The redirect endpoints aren't pages
      filter: (page) => !/\/(install\.sh|update\/)/.test(page),
      i18n: {
        defaultLocale: "en",
        locales: { en: "en", es: "es", uk: "uk" },
      },
    }),
    mdx(),
  ],

  i18n: {
    defaultLocale: "en",
    locales: ["en", "es", "uk"],
    routing: {
      prefixDefaultLocale: true,
    },
  },

  vite: {
    plugins: [tailwindcss()],
    // Pre-bundle the deps that would otherwise be discovered mid-session and
    // trigger a full optimizer reload, which the Cloudflare workerd dev runner
    // can't survive (crash: ".vite/deps_ssr/handler-*.js does not exist").
    optimizeDeps: {
      include: [
        "@astrojs/svelte/server.js",
        "astro-icon/components",
        "astro/logger/console",
      ],
    },
  },

  adapter: cloudflare(),
});
