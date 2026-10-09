/** The address bar as the app's route, without full page loads */
class Route {
  path = $state(location.pathname);

  constructor() {
    // Bookmarks from when routes lived behind a #
    if (location.hash.startsWith("#/")) this.go(location.hash.slice(1), true);
    addEventListener("popstate", () => (this.path = location.pathname));
    addEventListener("click", (e) => this.#follow(e));
  }

  go(path: string, replace = false) {
    if (replace) history.replaceState(null, "", path);
    else if (path !== location.pathname) history.pushState(null, "", path);
    this.path = location.pathname;
  }

  /** Links to our own pages; downloads, files and other sites load as usual */
  #follow(e: MouseEvent) {
    if (e.defaultPrevented || e.button !== 0) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const link = (e.target as Element | null)?.closest?.("a");
    if (!link || link.target || link.hasAttribute("download")) return;
    const url = new URL(link.href, location.href);
    if (url.origin !== location.origin) return;
    if (url.pathname.startsWith("/api/") || url.pathname.includes(".")) return;
    e.preventDefault();
    this.go(url.pathname);
  }
}

export const route = new Route();
