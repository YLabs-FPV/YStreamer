import enTranslations from "./locales/en.json";
import esTranslations from "./locales/es.json";
import ukTranslations from "./locales/uk.json";

export const languages = {
  en: "English",
  es: "Español",
  uk: "Українська",
} as const;

export type Language = keyof typeof languages;

export const defaultLang: Language = "en";

export const ui = {
  en: enTranslations,
  es: esTranslations,
  uk: ukTranslations,
} as const;

export function getLangFromUrl(url: URL): Language {
  const [, lang] = url.pathname.split("/");
  if (lang in ui) return lang as Language;
  return defaultLang;
}

function getNestedValue(obj: unknown, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>(
      (acc, part) =>
        acc && typeof acc === "object"
          ? (acc as Record<string, unknown>)[part]
          : undefined,
      obj,
    );
}

/**
 * Returns a translator for `lang`. Missing keys fall back to the default
 * language, then to the key itself. Returned as a string for the common case.
 */
export function useTranslations(lang: Language) {
  return function t(key: string): string {
    const value =
      getNestedValue(ui[lang], key) ?? getNestedValue(ui[defaultLang], key);
    return typeof value === "string" ? value : key;
  };
}

/**
 * Like `useTranslations` but returns the raw nested value (arrays / objects),
 * for tables and lists that live in the locale files.
 */
export function useData(lang: Language) {
  return function d<T>(key: string): T {
    const value =
      getNestedValue(ui[lang], key) ?? getNestedValue(ui[defaultLang], key);
    return value as T;
  };
}

export function getLocalizedUrl(lang: Language, path: string = ""): string {
  return `/${lang}${path === "" ? "" : path}`;
}

export function getRouteFromUrl(url: URL): string {
  let pathname = new URL(url).pathname;
  if (pathname !== "/" && pathname.endsWith("/")) {
    pathname = pathname.slice(0, -1);
  }
  const parts = pathname.split("/").filter(Boolean);
  const isLanguagePrefixed = parts.length > 0 && parts[0] in languages;
  if (isLanguagePrefixed) {
    const route = parts.slice(1).join("/");
    return route ? `/${route}` : "/";
  }
  return pathname === "/" ? "/" : pathname;
}

export function saveLanguagePreference(lang: Language): void {
  if (typeof window !== "undefined") {
    localStorage.setItem("preferred-language", lang);
    document.cookie = `preferred-language=${lang}; path=/; max-age=${
      60 * 60 * 24 * 365
    }; SameSite=Lax`;
  }
}
