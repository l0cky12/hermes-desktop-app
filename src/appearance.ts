import { FONT_PX, loadAppearance, type Appearance } from "./prefs";

const systemDark = matchMedia("(prefers-color-scheme: dark)");
let current = loadAppearance(localStorage);

/** Theme sets `.dark` on <html>, Skin sets `data-skin`, and Font size sets the rem base. */
export function applyAppearance(a: Appearance = current) {
  current = a;
  const root = document.documentElement;
  root.classList.toggle("dark", a.theme === "dark" || (a.theme === "system" && systemDark.matches));
  if (a.skin === "default") delete root.dataset.skin;
  else root.dataset.skin = a.skin;
  root.style.setProperty("--font-size", `${FONT_PX[a.fontSize]}px`);
}

systemDark.addEventListener("change", () => applyAppearance());
