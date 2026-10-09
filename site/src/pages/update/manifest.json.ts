import type { APIRoute } from "astro";
import { UPDATE_MANIFEST_TARGET } from "../../consts";

export const prerender = false;

export const GET: APIRoute = () =>
  new Response(null, {
    status: 302,
    headers: { Location: UPDATE_MANIFEST_TARGET, "Cache-Control": "no-store" },
  });
