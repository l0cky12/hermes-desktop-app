/** What this app remembers in webview storage. Never credentials: those stay in Rust and the keyring. */

export type ModelChoice = { provider: string; model: string };
/** What a Session's next Turns run on; `null` means the Profile's default. */
export type Choice = { model: ModelChoice | null; reasoning: string | null };
export type Appearance = { theme: "system" | "light" | "dark"; skin: string; fontSize: "small" | "default" | "large" };

export const REASONING_LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"];
export const SKINS = [
  "default", "ares", "mono", "slate", "poseidon", "sisyphus", "charizard", "sienna", "catppuccin", "nous", "geist-contrast", "zeus",
];
export const FONT_PX = { small: 13, default: 14, large: 16 } as const;
export const DEFAULT_CHOICE: Choice = { model: null, reasoning: null };

type Store = { getItem(key: string): string | null; setItem(key: string, value: string): void };

function read(store: Store, key: string): unknown {
  try {
    return JSON.parse(store.getItem(key) ?? "null");
  } catch {
    return null;
  }
}

export function loadAppearance(store: Store): Appearance {
  const raw = read(store, "appearance") as Partial<Appearance> | null;
  return {
    theme: raw?.theme === "light" || raw?.theme === "dark" ? raw.theme : "system",
    skin: SKINS.includes(String(raw?.skin)) ? String(raw?.skin) : "default",
    fontSize: raw?.fontSize === "small" || raw?.fontSize === "large" ? raw.fontSize : "default",
  };
}

export const saveAppearance = (store: Store, a: Appearance) => store.setItem("appearance", JSON.stringify(a));

/** A Session id only means something on one connection and Profile. */
export const choiceKey = (connection: string, profile: string, session: string) => `choice:${connection}:${profile}:${session}`;

export function loadChoice(store: Store, key: string): Choice {
  const raw = read(store, key) as { model?: Partial<ModelChoice> | null; reasoning?: unknown } | null;
  const m = raw?.model;
  return {
    model: typeof m?.provider === "string" && typeof m.model === "string" ? { provider: m.provider, model: m.model } : null,
    reasoning: REASONING_LEVELS.includes(String(raw?.reasoning)) ? String(raw?.reasoning) : null,
  };
}

export const saveChoice = (store: Store, key: string, c: Choice) => store.setItem(key, JSON.stringify(c));

/** The muted label under a reply: only on the first reply after the choice changed. */
export function changeLabel(previous: Choice | undefined, current: Choice): string {
  if (!previous || JSON.stringify(previous) === JSON.stringify(current)) return "";
  return `${current.model?.model ?? "Profile default"} · ${current.reasoning ?? "default reasoning"}`;
}

/** Last Profile used on this connection, else the Gateway's default Profile, else `default`, else the first listed. */
export function startProfile(store: Store, connection: string, names: string[], gatewayDefault: string | null): string {
  const saved = String(read(store, `profile:${connection}`));
  return [saved, gatewayDefault ?? "", "default"].find((n) => names.includes(n)) ?? names[0] ?? "default";
}

export const saveProfile = (store: Store, connection: string, name: string) =>
  store.setItem(`profile:${connection}`, JSON.stringify(name));
