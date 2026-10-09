import { languages } from "./index";

export function getLangPaths() {
  return Object.keys(languages).map((lang) => ({
    params: { lang },
  }));
}
