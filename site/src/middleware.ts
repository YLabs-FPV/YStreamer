import { defineMiddleware } from "astro:middleware";
import { languages, defaultLang, type Language } from "./i18n";

// Pick the best supported language from an Accept-Language header.
function detectBrowserLanguage(acceptLanguage: string | null): Language {
  if (!acceptLanguage) return defaultLang;

  const supported = Object.keys(languages) as Language[];
  const browserLangs = acceptLanguage
    .split(",")
    .map((lang) => lang.split(";")[0].trim().toLowerCase())
    .map((lang) => lang.split("-")[0]); // 'en' from 'en-US'

  for (const browserLang of browserLangs) {
    const matched = supported.find((lang) => lang === browserLang);
    if (matched) return matched;
  }
  return defaultLang;
}

function getStoredLanguage(cookies: {
  get: (name: string) => { value?: string } | undefined;
}): Language | null {
  const stored = cookies.get("preferred-language")?.value;
  return stored && stored in languages ? (stored as Language) : null;
}

function hasLocaleInPath(pathname: string): boolean {
  const [first] = pathname.split("/").filter(Boolean);
  return !!first && first in languages;
}

export const onRequest = defineMiddleware(async (context, next) => {
  const { url, request, cookies, redirect } = context;
  const { pathname } = url;

  // Skip static assets and API routes - they have no locale.
  if (
    pathname.startsWith("/_") ||
    (pathname.includes(".") && !pathname.endsWith("/")) ||
    pathname.startsWith("/api/")
  ) {
    return next();
  }

  // Never rewrite the special 404 route; it is prerendered to /404.html with no
  // locale prefix by design and is what Cloudflare serves for any not-found path.
  if (pathname === "/404" || pathname === "/404/") {
    return next();
  }

  // Already locale-prefixed (/en/..., /es/..., /uk/...) - serve as-is.
  if (hasLocaleInPath(pathname)) {
    return next();
  }

  // No locale in the path: pick one from the saved preference, then the browser.
  const target =
    getStoredLanguage(cookies) ??
    detectBrowserLanguage(request.headers.get("accept-language"));

  // Root ("/") gets a trailing slash so the redirect target matches the URLs
  // the navbar and language selector generate (/en/, /es/, /uk/).
  const newUrl = new URL(
    `/${target}${pathname === "/" ? "/" : pathname}`,
    url.origin,
  );
  newUrl.search = url.search;

  return redirect(newUrl.toString(), 302);
});
