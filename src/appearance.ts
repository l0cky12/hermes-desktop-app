import { field, h } from "./dom";
import { FONT_PX, SKINS, loadAppearance, saveAppearance, type Appearance } from "./prefs";

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

/** Settings → Appearance. Each change applies at once and is saved for this machine. */
export function appearanceTab(): HTMLElement {
  const select = (key: keyof Appearance, options: [string, string][]) => {
    const el = h("select", {}, ...options.map(([value, label]) => h("option", { value, textContent: label })));
    el.value = current[key];
    el.onchange = () => {
      const next = { ...current, [key]: el.value } as Appearance;
      saveAppearance(localStorage, next);
      applyAppearance(next);
    };
    return el;
  };
  const title = (s: string) => s.replace(/(^|-)(\w)/g, (_, dash, c) => (dash ? " " : "") + c.toUpperCase());
  return h(
    "section",
    {},
    h("h1", { textContent: "Appearance" }),
    field("Theme", select("theme", [["system", "System"], ["light", "Light"], ["dark", "Dark"]])),
    field("Skin", select("skin", SKINS.map((s) => [s, title(s)]))),
    field("Font size", select("fontSize", [["small", "Small"], ["default", "Default"], ["large", "Large"]])),
  );
}
