import { defineCollection } from "astro:content";
import { glob } from "astro/loaders";
import { z } from "astro/zod";

// Laid out as <lang>/<section>/<page>.mdx. Each section folder has an
// index.mdx: the section's overview, whose title names it in the sidebar and
// whose order places the section. Pages are ordered within their section.
const docs = defineCollection({
  loader: glob({
    base: "./src/content/docs",
    pattern: ["**/*.{md,mdx}", "!**/_*.{md,mdx}"],
    // The path as it is, "index" included, so sections can be told apart
    generateId: ({ entry }) => entry.replace(/\.mdx?$/, ""),
  }),
  schema: () =>
    z.object({
      title: z.string(),
      description: z.string().optional(),
      order: z.number().default(100),
    }),
});

export const collections = { docs };
