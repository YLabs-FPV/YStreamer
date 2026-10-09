import { getCollection, type CollectionEntry } from "astro:content";

export type DocEntry = CollectionEntry<"docs">;

// English is the source; other languages show it until a page is translated
const SOURCE_LANG = "en";

export interface DocSection {
  slug: string;
  /** The section's overview page (index.mdx) */
  index: DocEntry;
  pages: DocEntry[];
}

/** `en/<section>/<page>` -> its parts; `page` is "index" for an overview */
export function docParts(id: string) {
  const [lang, section, page = "index"] = id.split("/");
  return { lang, section, page };
}

export function docLang(id: string): string {
  return docParts(id).lang;
}

/** The URL path below /docs: "section" for an overview, else "section/page" */
export function docPath(id: string): string {
  const { section, page } = docParts(id);
  return page === "index" ? section : `${section}/${page}`;
}

const byOrder = (a: DocEntry, b: DocEntry) => a.data.order - b.data.order;

/** Sections for `lang` in reading order, English where there's no translation */
export async function getSections(lang: string): Promise<DocSection[]> {
  const all = await getCollection("docs");
  const translated = new Map(
    all.filter((d) => docLang(d.id) === lang).map((d) => [docPath(d.id), d]),
  );
  const pick = (d: DocEntry) => translated.get(docPath(d.id)) ?? d;

  const sections = new Map<string, { index?: DocEntry; pages: DocEntry[] }>();
  for (const d of all.filter((d) => docLang(d.id) === SOURCE_LANG)) {
    const { section, page } = docParts(d.id);
    const s = sections.get(section) ?? { pages: [] };
    if (page === "index") s.index = pick(d);
    else s.pages.push(pick(d));
    sections.set(section, s);
  }

  return [...sections.entries()]
    .filter(([slug, s]) => {
      if (!s.index)
        console.warn(`[docs] section "${slug}" has no index.mdx; skipped`);
      return !!s.index;
    })
    .map(([slug, s]) => ({
      slug,
      index: s.index!,
      pages: s.pages.sort(byOrder),
    }))
    .sort((a, b) => byOrder(a.index, b.index));
}

/** Every page in reading order: each overview, then its section's pages */
export async function getSortedDocs(lang: string): Promise<DocEntry[]> {
  return (await getSections(lang)).flatMap((s) => [s.index, ...s.pages]);
}

/** The very first page lives at /docs itself */
export function docHref(lang: string, id: string, sorted: DocEntry[]): string {
  const base = `/${lang}/docs`;
  return id === sorted[0]?.id ? base : `${base}/${docPath(id)}`;
}
