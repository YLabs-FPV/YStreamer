import type { APIRoute } from "astro";
import { INSTALL_SCRIPT_TARGET } from "../consts";

// `curl -fsSL https://ystreamer.yarosfpv.com/install.sh | sudo sh`
// The path is part of the published install command: do not rename it.
export const prerender = false;

export const GET: APIRoute = () =>
  new Response(null, {
    status: 302,
    headers: { Location: INSTALL_SCRIPT_TARGET, "Cache-Control": "no-store" },
  });
