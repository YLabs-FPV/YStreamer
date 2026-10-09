import type { APIRoute } from "astro";
import { GITHUB_URL, RELEASES_URL, UPDATE_MANIFEST_TARGET } from "../../consts";

export const prerender = false;

const RELEASE_DOWNLOADS = `${GITHUB_URL}/releases/download/`;
const CACHE = { cf: { cacheTtl: 300, cacheEverything: true } } as RequestInit;

interface Manifest {
  version?: unknown;
  packages?: Record<string, { url?: unknown }>;
}

async function latestImage(): Promise<string | null> {
  const res = await fetch(UPDATE_MANIFEST_TARGET, {
    redirect: "follow",
    ...CACHE,
  });
  if (!res.ok) return null;
  const manifest = (await res.json()) as Manifest;

  const version = manifest.version;
  if (typeof version !== "string" || !/^\d[\w.+~-]*$/.test(version))
    return null;
  const deb = Object.values(manifest.packages ?? {})
    .map((p) => p?.url)
    .find((url): url is string => typeof url === "string");
  if (!deb?.startsWith(RELEASE_DOWNLOADS)) return null;

  const image = `${deb.slice(0, deb.lastIndexOf("/"))}/ystreamer-${version}.img.xz`;
  const check = await fetch(image, {
    method: "HEAD",
    redirect: "manual",
    ...CACHE,
  });
  return check.status >= 200 && check.status < 400 ? image : null;
}

export const GET: APIRoute = async () => {
  let target: string | null = null;
  try {
    target = await latestImage();
  } catch {
    // Unreachable or unexpected manifest: fall back to the release page
  }
  return new Response(null, {
    status: 302,
    headers: {
      // The release page still lets people pick the file by hand
      Location: target ?? RELEASES_URL,
      "Cache-Control": "no-store",
    },
  });
};
