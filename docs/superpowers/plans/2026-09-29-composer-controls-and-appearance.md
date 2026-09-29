# Composer controls and Appearance settings: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a composer bar with paperclip, Dictation, Profile, model, and Reasoning level controls. Add image paste. Add a Settings view with an Appearance tab (Theme, Skin, Font size).

**Architecture:**
- Rust (`src-tauri/src/`) keeps owning all gateway and SSH traffic and all credentials (ADR 0001).
- A new `picker.rs` holds the pure parsing and validation for Profiles and models, so it can be unit-tested.
- `main.rs` gains Profile-aware request routing: a `/p/<name>/` prefix and a per-Profile API key. It also gains the commands `list_profiles`, `set_profile`, `list_models`, and `transcribe`.
- The webview (`src/`) stays framework-free TypeScript:
  - `prefs.ts`: pure logic for what's remembered in webview storage, unit-tested with `node --test`
  - `dom.ts`: DOM helpers and icons
  - `appearance.ts`: Theme, Skin, and Font size, plus the Settings tab
  - `main.ts`: the chat view and composer

**Tech Stack:** Tauri 2.12 (wry 0.57 / WebKitGTK), Rust 2021 with reqwest, tokio, serde, and base64; TypeScript 7 with Vite 8, no UI framework; Node 22.18+ built-in test runner.

**Spec:** `docs/composer-and-appearance-spec.md`. Read `CONTEXT.md` for vocabulary. Read `docs/adr/0001-networking-in-rust.md` for why nothing in the webview talks to the network.

## Global Constraints

- No new npm dependencies. The only new Rust dependency is `webkit2gtk = { version = "2", features = ["v2_40"] }`, for Linux only. It must resolve to the same crate Tauri already uses (2.0.x).
- All gateway and SSH traffic goes through Rust commands. The webview never calls `fetch`.
- Credentials (API keys, cookies, passwords) live only in Rust memory and the OS keyring. Webview `localStorage` may hold only Appearance settings, the last Profile per connection, and per-Session choices.
- A Profile name must pass `^[a-z0-9][a-z0-9_-]{0,63}$` in Rust (`picker::valid_profile`) before it's used in a URL path or the remote shell command.
- The app never changes the Gateway's default Profile. It never calls `POST /api/profiles/active` and never writes Hermes config.
- Attachment rules are enforced in `attach.rs`: at most 2 MB each. `png`/`jpg`/`jpeg`/`gif`/`webp` are sent as images, UTF-8 text is inlined, and anything else is refused.
- Reasoning level values are exactly `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`. "Default" means the field is omitted.
- Font sizes are Small 13px, Default 14px, and Large 16px. Theme defaults to System. Skins are `default`, `ares`, `mono`, `slate`, `poseidon`, `sisyphus`, `charizard`, `sienna`, `catppuccin`, `nous`, `geist-contrast`, and `zeus`.
- Never set `innerHTML` from server or user text. The icon SVGs are constant strings in `dom.ts` and are parsed with `DOMParser`.
- UI copy uses the `CONTEXT.md` terms: Profile, Session, Turn, Attachment, Dictation, Reasoning level, Theme, Skin.
- Verification commands, run from the repo root:
  - `(cd src-tauri && cargo test)`
  - `npm test`
  - `npm run build` (runs `tsc` and `vite build`)
- Manual runs use the mock gateway: `node mock/server.mjs`, then `npm run tauri dev`, then pair with `http://127.0.0.1:19119` and `http://127.0.0.1:18642`, user `tester` / `correct-horse-battery`, key `hd-test-key-4f9c2a7e1b`.
- Every commit message ends with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.

## Review Focus

1. **Keyring entries saved before Profiles existed** still load and keep working, with no forced re-pairing. The test is in Task 4 (`keyring_entries_from_before_profiles_still_load`).
2. **A Profile whose key is unknown, or that has none on the gateway** (mock `coder`): "Back to <previous>" returns to the previous Profile cleanly, and a key entered for one Profile is never used for another. The test is in Task 4 (`each_profile_keeps_its_own_key`), with a manual step in Task 11.
3. **Switching Profile while a reply is streaming:** the Turn stops, and nothing from the old Run lands in the new Profile's view. This is a manual step in Task 11.
4. **The model list failing or being slow** (`MOCK_NO_MODELS=1`, a slow SSH `session/new`): the composer still sends using Profile default, and the selector says why it's empty. This is a manual step in Task 12.
5. **Transcription taking longer than the 15 s idle timeout:** it still succeeds, because it runs on a client with no idle cutoff and a 2-minute total. This is a manual step in Task 13 with `MOCK_TRANSCRIBE_MS=20000`.

---

### Task 1: Microphone access in the webview (gate)

WebKitGTK ships with `enable-media-stream` off, and wry never turns it on, so `navigator.mediaDevices.getUserMedia` fails. This task turns it on, allows microphone permission requests, and proves that recording works. **If the check in Step 4 fails, stop and report back to the user before starting any other task.** Their decision is whether to drop Dictation or bundle a native recorder.

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/main.rs` (the `use` block, and `main()`)

**Interfaces:**
- Produces: a webview in which `getUserMedia({ audio: true })` and `MediaRecorder` work. Task 13 relies on this.

- [ ] **Step 1: Add the Linux-only dependency**

Append to `src-tauri/Cargo.toml`:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
# Dictation needs getUserMedia, which WebKitGTK ships switched off; same crate Tauri already uses.
webkit2gtk = { version = "2", features = ["v2_40"] }
```

- [ ] **Step 2: Turn on media streams and allow the microphone**

In `src-tauri/src/main.rs`, change the `tauri` import to:

```rust
use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{async_runtime, ipc::Channel, DragDropEvent, Manager, State, WindowEvent};
```

In `main()`, inside `.setup(|app| { ... })`, add this after `app.manage(...)` and before `Ok(())`:

```rust
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                window.with_webview(|webview| {
                    use webkit2gtk::{SettingsExt, WebViewExt};
                    if let Some(settings) = WebViewExt::settings(&webview.inner()) {
                        settings.set_enable_media_stream(true);
                    }
                })?;
            }
```

Chain this after `.setup(...)`:

```rust
        // Dictation is the only thing that asks; everything else keeps the platform default.
        .on_permission_request(|_, kind| match kind {
            PermissionKind::Microphone => PermissionResponse::Allow,
            _ => PermissionResponse::Default,
        })
```

- [ ] **Step 3: Build**

Run: `(cd src-tauri && cargo build)`
Expected: builds with no errors. `cargo tree -i webkit2gtk` shows a single `webkit2gtk v2.0.x`.

- [ ] **Step 4: Prove recording works (the gate)**

Run `node mock/server.mjs` and `npm run tauri dev`. Right-click in the window, choose **Inspect Element**, and paste this into the Console:

```js
const s = await navigator.mediaDevices.getUserMedia({ audio: true });
console.log("MediaRecorder:", typeof MediaRecorder, ["audio/webm;codecs=opus", "audio/ogg;codecs=opus", "audio/mp4"].map((t) => [t, MediaRecorder.isTypeSupported(t)]));
const r = new MediaRecorder(s); const parts = []; r.ondataavailable = (e) => parts.push(e.data);
r.onstop = () => console.log("recorded", r.mimeType, new Blob(parts).size, "bytes"); r.start(); setTimeout(() => r.stop(), 3000);
```

Expected: after about 3 s, `recorded audio/<something> <N> bytes` with N > 1000.
If `getUserMedia` rejects, `MediaRecorder` is `undefined`, or the size is 0: **STOP.** Report the exact console output to the user and wait for their decision.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs
git commit -m "Allow microphone access in the webview for Dictation

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Webview preferences module and its test runner

This task adds the pure logic for everything the webview remembers:
- Appearance settings
- the last Profile per connection
- each Session's Model choice and Reasoning level
- the reply label shown after a change

**Files:**
- Create: `src/prefs.ts`
- Create: `src/prefs.test.ts`
- Modify: `package.json` (the `scripts` block)
- Modify: `tsconfig.json`

**Interfaces:**
- Produces, all exported from `src/prefs.ts`:
  - `type ModelChoice = { provider: string; model: string }`
  - `type Choice = { model: ModelChoice | null; reasoning: string | null }`
  - `type Appearance = { theme: "system" | "light" | "dark"; skin: string; fontSize: "small" | "default" | "large" }`
  - `REASONING_LEVELS: string[]`, `SKINS: string[]`, `FONT_PX: { small: 13; default: 14; large: 16 }`, `DEFAULT_CHOICE: Choice`
  - `loadAppearance(store): Appearance`, `saveAppearance(store, a): void`
  - `choiceKey(connection, profile, session): string`, `loadChoice(store, key): Choice`, `saveChoice(store, key, c): void`
  - `changeLabel(previous: Choice | undefined, current: Choice): string`
  - `startProfile(store, connection, names: string[], gatewayDefault: string | null): string`, `saveProfile(store, connection, name): void`
  - `store` is any `{ getItem, setItem }`, for example `localStorage`.

- [ ] **Step 1: Wire the test runner**

In `package.json`, add this to `"scripts"`:

```json
    "test": "node --test src/*.test.ts"
```

In `tsconfig.json`, after `"include": ["src"]`, add this so `tsc` skips the Node-only test files:

```json
  "exclude": ["src/**/*.test.ts"]
```

(Remember the comma after the `include` line.)

- [ ] **Step 2: Write the failing test**

Create `src/prefs.test.ts`:

```ts
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  changeLabel, choiceKey, DEFAULT_CHOICE, loadAppearance, loadChoice, saveAppearance, saveChoice, saveProfile, startProfile,
} from "./prefs.ts";

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

test("appearance falls back to defaults and round-trips", () => {
  const store = memory();
  assert.deepEqual(loadAppearance(store), { theme: "system", skin: "default", fontSize: "default" });
  store.setItem("appearance", "{not json");
  assert.deepEqual(loadAppearance(store), { theme: "system", skin: "default", fontSize: "default" });
  store.setItem("appearance", JSON.stringify({ theme: "neon", skin: "nope", fontSize: "huge" }));
  assert.deepEqual(loadAppearance(store), { theme: "system", skin: "default", fontSize: "default" });
  saveAppearance(store, { theme: "dark", skin: "ares", fontSize: "large" });
  assert.deepEqual(loadAppearance(store), { theme: "dark", skin: "ares", fontSize: "large" });
});

test("choices are kept per connection, Profile, and Session", () => {
  const store = memory();
  const a = choiceKey("http://h:9119|http://h:8642", "orchestrator", "s1");
  const b = choiceKey("http://h:9119|http://h:8642", "coder", "s1");
  assert.deepEqual(loadChoice(store, a), DEFAULT_CHOICE);
  saveChoice(store, a, { model: { provider: "zai", model: "glm-5.3-flash" }, reasoning: "high" });
  assert.deepEqual(loadChoice(store, a), { model: { provider: "zai", model: "glm-5.3-flash" }, reasoning: "high" });
  assert.deepEqual(loadChoice(store, b), DEFAULT_CHOICE);
  store.setItem(a, JSON.stringify({ model: { provider: 1 }, reasoning: "turbo" }));
  assert.deepEqual(loadChoice(store, a), DEFAULT_CHOICE);
});

test("the reply label appears only where the choice changed", () => {
  const glm = { model: { provider: "zai", model: "glm-5.3-flash" }, reasoning: "high" };
  assert.equal(changeLabel(undefined, glm), "");
  assert.equal(changeLabel(glm, { ...glm }), "");
  assert.equal(changeLabel(DEFAULT_CHOICE, glm), "glm-5.3-flash · high");
  assert.equal(changeLabel(glm, DEFAULT_CHOICE), "Profile default · default reasoning");
});

test("the start Profile is the last used, then the Gateway's default, then default", () => {
  const store = memory();
  const names = ["default", "orchestrator", "coder"];
  assert.equal(startProfile(store, "c", names, "orchestrator"), "orchestrator");
  assert.equal(startProfile(store, "c", names, null), "default");
  assert.equal(startProfile(store, "c", ["coder"], null), "coder");
  saveProfile(store, "c", "coder");
  assert.equal(startProfile(store, "c", names, "orchestrator"), "coder");
  assert.equal(startProfile(store, "other", names, "orchestrator"), "orchestrator");
  saveProfile(store, "c", "gone");
  assert.equal(startProfile(store, "c", names, "orchestrator"), "orchestrator");
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `npm test`
Expected: FAIL with `ERR_MODULE_NOT_FOUND` for `./prefs.ts`.

- [ ] **Step 4: Write the implementation**

Create `src/prefs.ts`:

```ts
/** What this app remembers in webview storage. Never credentials: those stay in Rust and the keyring. */

export type ModelChoice = { provider: string; model: string };
/** What a Session's next Turns run on; `null` means the Profile's default. */
export type Choice = { model: ModelChoice | null; reasoning: string | null };
export type Appearance = { theme: "system" | "light" | "dark"; skin: string; fontSize: "small" | "default" | "large" };

export const REASONING_LEVELS = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `npm test && npm run build`
Expected: 4 passing tests, then a clean `tsc` and `vite build`.

- [ ] **Step 6: Commit**

```bash
git add src/prefs.ts src/prefs.test.ts package.json tsconfig.json
git commit -m "Add webview preferences: appearance, per-Session choices, start Profile

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `picker.rs`: Profile and model lists, and the Run body

This task adds the pure Rust helpers that later tasks wire up. At the end of it, `cargo build` warns about unused functions; that's expected.

**Files:**
- Create: `src-tauri/src/picker.rs`
- Modify: `src-tauri/src/main.rs` (add `mod picker;` after `mod gateway;`)

**Interfaces:**
- Produces, all in `crate::picker`:
  - `valid_profile(name: &str) -> Result<String, Error>`
  - `struct Profiles { names: Vec<String>, active: Option<String> }`, which is `Serialize` and becomes `{ names, active }` on the JS side
  - `parse_profile_list(text: &str) -> Profiles`
  - `from_dashboard(list: &Value, active: &Value) -> Profiles`
  - `api_segments<'a>(profile: Option<&'a str>, path: &[&'a str]) -> Vec<&'a str>`
  - `struct ModelChoice { provider: String, model: String }`, which is `Serialize + Deserialize + Clone + PartialEq`
  - `struct ModelGroup { provider: String, name: String, models: Vec<String> }`
  - `struct Models { default: Option<ModelChoice>, groups: Vec<ModelGroup> }`
  - `from_options(payload: &Value) -> Models`, `from_acp(state: &Value) -> Models`
  - `split_acp_id(id: &str) -> Option<ModelChoice>`, `acp_id(c: &ModelChoice) -> String`
  - `valid_reasoning(r: &str) -> Result<String, Error>`
  - `run_body(input: Value, session_id: Option<&str>, model: Option<&ModelChoice>, reasoning: Option<&str>) -> Value`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/picker.rs` containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn profile_names_follow_hermes_rules() {
        for good in ["default", "orchestrator", "a", "web-2_x"] {
            assert!(valid_profile(good).is_ok(), "{good}");
        }
        for bad in ["", "-x", "_x", "Coder", "a b", "a;rm -rf ~", "../x", "p/x"] {
            assert!(valid_profile(bad).is_err(), "{bad:?}");
        }
        assert!(valid_profile(&"a".repeat(65)).is_err());
    }

    #[test]
    fn profile_table_is_parsed() {
        let out = "\n Profile          Model                        Gateway\n ───────────────    ──────────    ─────\n  default         deepseek/deepseek-v4-flash   running\n ◆orchestrator    z-ai/glm-5.3-flash           running\n  coder           —                            stopped\n\n ⚠  Profile 'talos' shares its buzz credential with default\n";
        let expected = Profiles { names: vec!["default".into(), "orchestrator".into(), "coder".into()], active: Some("orchestrator".into()) };
        assert_eq!(parse_profile_list(out), expected);
        assert_eq!(parse_profile_list(&out.replace("◆orchestrator", "◆ orchestrator")), expected);
    }

    #[test]
    fn dashboard_profiles() {
        let list = json!({ "profiles": [{ "name": "default" }, { "name": "orchestrator" }, { "name": "Bad Name" }] });
        let got = from_dashboard(&list, &json!({ "active": "orchestrator", "current": "default" }));
        assert_eq!(got, Profiles { names: vec!["default".into(), "orchestrator".into()], active: Some("orchestrator".into()) });
        // A sticky default this gateway doesn't list is not offered as the start Profile.
        assert_eq!(from_dashboard(&list, &json!({ "active": "gone" })).active, None);
    }

    #[test]
    fn named_profiles_get_a_path_prefix() {
        assert_eq!(api_segments(Some("coder"), &["v1", "runs"]), ["p", "coder", "v1", "runs"]);
        assert_eq!(api_segments(Some("default"), &["v1", "runs"]), ["v1", "runs"]);
        assert_eq!(api_segments(None, &["api", "sessions"]), ["api", "sessions"]);
    }

    #[test]
    fn only_set_up_providers_with_models_are_offered() {
        let payload = json!({ "provider": "zai", "model": "glm-5.3-flash", "providers": [
            { "slug": "zai", "name": "Z.ai", "authenticated": true, "models": ["glm-5.3-flash", "glm-5.3"] },
            { "slug": "nous", "name": "Nous Portal", "authenticated": false, "models": ["hermes-5"] },
            { "slug": "moa", "name": "Mixture", "authenticated": true, "models": [] },
        ]});
        assert_eq!(from_options(&payload), Models {
            default: Some(ModelChoice { provider: "zai".into(), model: "glm-5.3-flash".into() }),
            groups: vec![ModelGroup { provider: "zai".into(), name: "Z.ai".into(), models: vec!["glm-5.3-flash".into(), "glm-5.3".into()] }],
        });
    }

    #[test]
    fn acp_models_are_grouped_by_provider() {
        let state = json!({ "currentModelId": "openrouter:deepseek/deepseek-v4-flash", "availableModels": [
            { "modelId": "openrouter:deepseek/deepseek-v4-flash", "name": "deepseek-v4-flash" },
            { "modelId": "anthropic:claude-opus-5-5", "name": "claude-opus-5-5" },
            { "modelId": "openrouter:z-ai/glm-5.3-flash", "name": "glm-5.3-flash" },
            { "modelId": "no-provider", "name": "x" },
        ]});
        let models = from_acp(&state);
        let default = models.default.clone().unwrap();
        assert_eq!(default, ModelChoice { provider: "openrouter".into(), model: "deepseek/deepseek-v4-flash".into() });
        let shape: Vec<(&str, usize)> = models.groups.iter().map(|g| (g.provider.as_str(), g.models.len())).collect();
        assert_eq!(shape, [("openrouter", 2), ("anthropic", 1)]);
        assert_eq!(acp_id(&default), "openrouter:deepseek/deepseek-v4-flash");
    }

    #[test]
    fn run_body_carries_the_model_choice() {
        assert_eq!(run_body(json!("hi"), None, None, None), json!({ "input": "hi" }));
        let m = ModelChoice { provider: "zai".into(), model: "glm-5.3-flash".into() };
        assert_eq!(
            run_body(json!("hi"), Some("s1"), Some(&m), Some("high")),
            json!({ "input": "hi", "session_id": "s1", "provider": "zai", "model": "glm-5.3-flash",
                    "model_options": { "reasoning_effort": "high" } })
        );
    }

    #[test]
    fn reasoning_levels_are_hermes_values() {
        assert!(valid_reasoning("xhigh").is_ok());
        assert!(valid_reasoning("turbo").is_err());
        assert!(valid_reasoning("").is_err());
    }
}
```

Add `mod picker;` to `src-tauri/src/main.rs`, after `mod gateway;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test picker)`
Expected: FAIL to compile with `cannot find function valid_profile` (and the others).

- [ ] **Step 3: Write the implementation**

Put this above the test module in `src-tauri/src/picker.rs`:

```rust
//! Profile and model lists for the composer's pickers, from either connection mode, and the
//! per-Run model overrides. Pure functions only: main.rs and acp.rs do the I/O.

use crate::gateway::Error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Hermes' own rule: lowercase letters, digits, `-`, `_`, starting with a letter or digit, at most
/// 64. It's also what makes a name safe as a URL segment and inside the remote shell command.
pub fn valid_profile(name: &str) -> Result<String, Error> {
    let ok = name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    match ok {
        true => Ok(name.to_owned()),
        false => Err(Error::Invalid(format!("{name:?} is not a Hermes profile name"))),
    }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Profiles {
    pub names: Vec<String>,
    /// The Gateway's default Profile (`hermes profile use`), if it's one of `names`.
    pub active: Option<String>,
}

/// `hermes profile list` prints a table: rows follow the `───` rule and end at a blank line, and
/// the sticky default is marked `◆`.
pub fn parse_profile_list(text: &str) -> Profiles {
    let mut names = Vec::new();
    let mut active = None;
    for line in text.lines().skip_while(|l| !l.contains('─')).skip(1).take_while(|l| !l.trim().is_empty()) {
        let line = line.trim_start();
        let Some(Ok(name)) = line.trim_start_matches('◆').split_whitespace().next().map(valid_profile) else { continue };
        if line.starts_with('◆') {
            active = Some(name.clone());
        }
        names.push(name);
    }
    Profiles { names, active }
}

/// Dashboard `GET /api/profiles` plus `GET /api/profiles/active`.
pub fn from_dashboard(list: &Value, active: &Value) -> Profiles {
    let names: Vec<String> = list["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| valid_profile(p["name"].as_str()?).ok())
        .collect();
    let active = active["active"].as_str().filter(|a| names.iter().any(|n| n == a)).map(str::to_owned);
    Profiles { names, active }
}

/// `/p/<name>/…` addresses a named Profile on a multiplexing gateway; `default` is the bare path.
pub fn api_segments<'a>(profile: Option<&'a str>, path: &[&'a str]) -> Vec<&'a str> {
    let prefix = profile.filter(|p| *p != "default").map(|p| ["p", p]);
    prefix.iter().flatten().copied().chain(path.iter().copied()).collect()
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct ModelChoice {
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ModelGroup {
    pub provider: String,
    pub name: String,
    pub models: Vec<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Models {
    /// What "Profile default" currently resolves to, when Hermes says.
    pub default: Option<ModelChoice>,
    pub groups: Vec<ModelGroup>,
}

/// `GET /api/model/options` lists every provider Hermes knows about; only set-up ones with
/// models are offered.
pub fn from_options(payload: &Value) -> Models {
    let groups = payload["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["authenticated"] == true)
        .filter_map(|p| {
            let provider = p["slug"].as_str()?.to_owned();
            let name = p["name"].as_str().unwrap_or(&provider).to_owned();
            let models: Vec<String> = p["models"].as_array()?.iter().filter_map(|m| m.as_str().map(str::to_owned)).collect();
            (!models.is_empty()).then_some(ModelGroup { provider, name, models })
        })
        .collect();
    let default = match (payload["provider"].as_str(), payload["model"].as_str()) {
        (Some(provider), Some(model)) => Some(ModelChoice { provider: provider.into(), model: model.into() }),
        _ => None,
    };
    Models { default, groups }
}

/// ACP model ids are `provider:model` (hermes acp_adapter/model_catalog.py `encode_model_choice`).
pub fn split_acp_id(id: &str) -> Option<ModelChoice> {
    let (provider, model) = id.split_once(':')?;
    Some(ModelChoice { provider: provider.to_owned(), model: model.to_owned() })
}

pub fn acp_id(c: &ModelChoice) -> String {
    format!("{}:{}", c.provider, c.model)
}

/// The `models` field of an ACP `session/new` answer: `{ availableModels, currentModelId }`.
pub fn from_acp(state: &Value) -> Models {
    let mut groups: Vec<ModelGroup> = Vec::new();
    let choices = state["availableModels"].as_array().into_iter().flatten().filter_map(|m| split_acp_id(m["modelId"].as_str()?));
    for choice in choices {
        match groups.iter().position(|g| g.provider == choice.provider) {
            Some(i) => groups[i].models.push(choice.model),
            None => groups.push(ModelGroup { name: choice.provider.clone(), provider: choice.provider, models: vec![choice.model] }),
        }
    }
    Models { default: state["currentModelId"].as_str().and_then(split_acp_id), groups }
}

/// The ladder the API server accepts (gateway/platforms/api_server.py `_REASONING_EFFORTS`).
const REASONING: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub fn valid_reasoning(r: &str) -> Result<String, Error> {
    match REASONING.contains(&r) {
        true => Ok(r.to_owned()),
        false => Err(Error::Invalid(format!("{r:?} is not a reasoning level"))),
    }
}

/// The `POST /v1/runs` body. Model and Reasoning level override the Profile's defaults for this
/// Run only; leaving them out means "Profile default".
pub fn run_body(input: Value, session_id: Option<&str>, model: Option<&ModelChoice>, reasoning: Option<&str>) -> Value {
    let mut body = json!({ "input": input });
    if let Some(id) = session_id {
        body["session_id"] = json!(id);
    }
    if let Some(m) = model {
        body["provider"] = json!(m.provider);
        body["model"] = json!(m.model);
    }
    if let Some(r) = reasoning {
        body["model_options"] = json!({ "reasoning_effort": r });
    }
    body
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `(cd src-tauri && cargo test picker)`
Expected: 8 passing tests. Dead-code warnings are fine at this stage.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/picker.rs src-tauri/src/main.rs
git commit -m "Add picker.rs: Profile and model list parsing, Run model overrides

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Profiles in Rust, plus mock Profiles

**Files:**
- Modify: `src-tauri/src/main.rs`
- Modify: `src-tauri/src/acp.rs`
- Modify: `mock/server.mjs`

**Interfaces:**
- Consumes: `picker::{valid_profile, from_dashboard, api_segments, parse_profile_list, Profiles}` (Task 3).
- Produces:
  - Tauri command `list_profiles() -> { names: string[], active: string | null }`
  - Tauri command `set_profile({ name: string }) -> void`
  - Both connection modes route every later request to the chosen Profile.
  - Over HTTP, `set_api_key` now stores the key for the *current* Profile.
  - `AppState::dashboard(method, path, query)` gains a `query: &[(&str, &str)]` parameter.
  - `acp::connect(host, profile: Option<&str>)`
  - `acp::list_profiles(host) -> Result<Profiles, Error>`

- [ ] **Step 1: Write the failing tests**

Append to the end of `src-tauri/src/main.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_profile_keeps_its_own_key() {
        let mut inner = Inner { creds: Creds { api_key: Some("root".into()), ..Creds::default() }, ..Inner::default() };
        inner.profile = Some("coder".into());
        assert_eq!(inner.api_key(), None, "never falls back to the default Profile's key");
        assert_eq!(inner.replace_key(Some("c".into())), None);
        assert_eq!(inner.api_key(), Some("c"));
        inner.profile = Some("default".into());
        assert_eq!(inner.api_key(), Some("root"));
        inner.profile = Some("coder".into());
        assert_eq!(inner.replace_key(None), Some("c".into()), "rollback of a rejected key");
        assert_eq!(inner.creds.api_key.as_deref(), Some("root"));
    }

    #[test]
    fn keyring_entries_from_before_profiles_still_load() {
        let old = r#"{"dashboard_url":"http://h:9119/","api_url":"http://h:8642/","api_key":"k","cookies":{}}"#;
        let creds: Creds = serde_json::from_str(old).unwrap();
        assert_eq!(creds.api_key.as_deref(), Some("k"));
        assert!(creds.profile_keys.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test tests::)`
Expected: FAIL to compile. The errors are `no field profile on type Inner` and `no method named api_key`.

- [ ] **Step 3: Add Profile state and key slots**

In `src-tauri/src/main.rs`:

In `struct Creds`, after `cookies`:

```rust
    /// Named Profiles' own API keys; `api_key` is the default Profile's.
    #[serde(default)]
    profile_keys: BTreeMap<String, String>,
```

In `struct Inner`, after `run_stop: bool,`:

```rust
    /// The Profile this app is showing (already `picker::valid_profile`); `None` until the chat
    /// view picks one. Over SSH, `None` runs plain `hermes acp`, meaning the Gateway's default.
    profile: Option<String>,
```

After `struct Inner`, add:

```rust
impl Inner {
    fn named_profile(&self) -> Option<&str> {
        self.profile.as_deref().filter(|p| *p != "default")
    }

    fn api_key(&self) -> Option<&str> {
        match self.named_profile() {
            Some(p) => self.creds.profile_keys.get(p).map(String::as_str),
            None => self.creds.api_key.as_deref(),
        }
    }

    /// Replaces the current Profile's API key, returning the old one (for rollback).
    fn replace_key(&mut self, key: Option<String>) -> Option<String> {
        match self.named_profile().map(str::to_owned) {
            Some(p) => match key {
                Some(k) => self.creds.profile_keys.insert(p, k),
                None => self.creds.profile_keys.remove(&p),
            },
            None => std::mem::replace(&mut self.creds.api_key, key),
        }
    }
}
```

- [ ] **Step 4: Route requests by Profile**

Replace `AppState::dashboard` and `AppState::api` with:

```rust
    /// Dashboard request; the sign-in cookie is only ever attached here.
    fn dashboard(&self, method: Method, path: &[&str], query: &[(&str, &str)]) -> Result<RequestBuilder, Error> {
        let inner = self.lock();
        let gateway = inner.gateway.as_ref().ok_or_else(not_configured)?;
        let mut url = endpoint(&gateway.dashboard, path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let request = self.client.request(method, url);
        Ok(match inner.creds.cookies.is_empty() {
            true => request,
            false => request.header(header::COOKIE, cookie_header(&inner.creds.cookies)),
        })
    }

    /// API server request for the current Profile; its API key is only ever attached here.
    fn api(&self, method: Method, path: &[&str], query: &[(&str, &str)]) -> Result<RequestBuilder, Error> {
        let inner = self.lock();
        let gateway = inner.gateway.as_ref().ok_or_else(not_configured)?;
        let key = inner.api_key().ok_or_else(|| Error::Unauthorized("No API key saved for this profile".into()))?;
        let mut url = endpoint(&gateway.api, &picker::api_segments(inner.profile.as_deref(), path));
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(self.client.request(method, url).bearer_auth(key))
    }
```

Update the three existing `state.dashboard(...)` calls, in `status`, `sign_in`, and `check_sign_in`, to pass `&[]` as the new last argument. For example: `state.dashboard(Method::GET, &["api", "status"], &[])?`.

In `save_creds`, change the empty check to:

```rust
        if creds.api_key.is_none() && creds.cookies.is_empty() && creds.profile_keys.is_empty() {
```

In `configure`, inside `if changed { ... }`, add `inner.profile = None;`. In `configure_ssh`, inside the first block, add `inner.profile = None;`.

In `set_api_key`, replace the first line and the rollback line:

```rust
    let previous = state.lock().replace_key(Some(key.trim().to_owned()));
```

```rust
            state.lock().replace_key(previous);
```

In `AppState::hermes`, replace the `host` line and the `connect` line:

```rust
        let (host, profile) = {
            let inner = self.lock();
            (inner.ssh_host.clone().ok_or_else(not_configured)?, inner.profile.clone())
        };
        let conn = acp::connect(&host, profile.as_deref()).await?;
```

- [ ] **Step 5: Add the commands**

Add these after `connect_ssh`:

```rust
/// Profiles to offer, and the Gateway's default Profile, which this app never changes.
#[tauri::command]
async fn list_profiles(state: State<'_, AppState>) -> Result<picker::Profiles, Error> {
    if state.ssh() {
        let host = state.lock().ssh_host.clone().ok_or_else(not_configured)?;
        return acp::list_profiles(&host).await;
    }
    let get = |path: &'static [&'static str]| state.dashboard(Method::GET, path, &[]);
    let list = json_body(send(&state.client, get(&["api", "profiles"])?).await?).await?;
    let active = json_body(send(&state.client, get(&["api", "profiles", "active"])?).await?).await?;
    Ok(picker::from_dashboard(&list, &active))
}

/// Switches Profile. Over HTTP, later requests go to `/p/<name>/` with that Profile's own key;
/// over SSH, Hermes restarts as `hermes -p <name> acp`.
#[tauri::command]
async fn set_profile(state: State<'_, AppState>, name: String) -> Result<(), Error> {
    let name = picker::valid_profile(&name)?;
    let changed = state.lock().profile.replace(name.clone()) != Some(name);
    if changed && state.ssh() {
        state.hermes.lock().await.take();
        state.acp_runs.lock().unwrap().clear();
    }
    Ok(())
}
```

Register `list_profiles` and `set_profile` in `tauri::generate_handler![...]`, after `connect_ssh`.

- [ ] **Step 6: Make the SSH side Profile-aware**

In `src-tauri/src/acp.rs`, replace `const REMOTE: &str = ...;` and its doc comment with:

```rust
/// Makes `hermes` findable: the installer puts it in ~/.local/bin, which non-interactive SSH
/// shells often leave off PATH.
// ponytail: POSIX-shell syntax; a fish/nushell login shell needs `ssh host sh -c ...` instead.
const HERMES: &str = "PATH=\"$HOME/.local/bin:$PATH\" exec hermes";

/// Prints the remote home (the cwd for new Sessions), then becomes Hermes for the given Profile.
/// `profile` has passed `picker::valid_profile`, which is what makes it safe in this command.
fn remote(profile: Option<&str>) -> String {
    let args = profile.map(|p| format!("-p {p} acp")).unwrap_or_else(|| "acp".into());
    format!("printf 'hermes-desktop-home %s\\n' \"$HOME\"; {HERMES} {args}")
}
```

Change the signature of `connect` to `pub async fn connect(host: &str, profile: Option<&str>) -> Result<Arc<Conn>, Error>`. Change its `.args([...])` line to:

```rust
        .args(["-T", "-o", "BatchMode=yes", "--", host, &remote(profile)])
```

Add this after `connect`:

```rust
/// `hermes profile list` on the host (a second, short SSH call), parsed.
pub async fn list_profiles(host: &str) -> Result<Profiles, Error> {
    let ssh = std::env::var_os("HERMES_DESKTOP_SSH").unwrap_or_else(|| "ssh".into());
    let mut command = Command::new(ssh);
    command
        .args(["-T", "-o", "BatchMode=yes", "--", host, &format!("{HERMES} profile list")])
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| Error::Unreachable(format!("No answer from {host} within 30 s")))?
        .map_err(|e| Error::Unreachable(format!("Could not run ssh: {e}")))?;
    if !out.status.success() {
        return Err(Error::Unreachable(exited(String::from_utf8_lossy(&out.stderr).into_owned())));
    }
    Ok(parse_profile_list(&String::from_utf8_lossy(&out.stdout)))
}
```

Add `use crate::picker::{parse_profile_list, Profiles};` to the `use` block in `acp.rs`. In the ignored test `lists_and_loads_sessions_from_real_hermes`, change `connect("localhost")` to `connect("localhost", None)`.

- [ ] **Step 7: Add Profiles to the mock gateway**

In `mock/server.mjs`, after the `const KEY = ...` line:

```js
// Each Profile has its own API key, never the default's; "coder" is listed but has none.
const PROFILES = ["default", "orchestrator", "coder"];
const PROFILE_KEYS = { default: KEY, orchestrator: env.MOCK_ORCH_KEY ?? "hd-orch-key-7d3e9a1c5b" };
```

In the Dashboard handler, before the final `send(res, 404, ...)`:

```js
  const signedIn = env.MOCK_AUTH === "off" || state.access[cookieJar(req).hermes_session_at] > Date.now();
  if (url.pathname.startsWith("/api/") && !signedIn) return send(res, 401, { detail: "Unauthorized" });
  if (route === "GET /api/profiles") {
    return send(res, 200, { profiles: PROFILES.map((name) => ({ name, is_default: name === "default" })) });
  }
  if (route === "GET /api/profiles/active") return send(res, 200, { active: "orchestrator", current: "default" });
```

In the API server handler, replace the `authorization` check line with:

```js
  // /p/<profile>/... addresses a named Profile, like a multiplexing gateway.
  let profile = "default";
  const prefixed = url.pathname.match(/^\/p\/([^/]+)(\/.*)$/);
  if (prefixed) {
    profile = decodeURIComponent(prefixed[1]);
    if (!PROFILES.includes(profile)) return send(res, 404, { error: "Unknown or unconfigured profile" });
    url.pathname = prefixed[2];
  }
  if (!PROFILE_KEYS[profile] || req.headers.authorization !== `Bearer ${PROFILE_KEYS[profile]}`) return send(res, 401, unauthorized);
```

Change `function touchSession(id, preview)` to `function touchSession(id, preview, profile = "default")`, and add `profile,` to the object it creates, next to `source: "api_server"`. In `POST /v1/runs`, call `touchSession(sessionId, text.slice(0, 60), profile)`. In `POST /api/sessions`, call `touchSession(sid, null, profile)`. In `GET /api/sessions`, filter by Profile:

```js
    const all = Object.values(state.sessions).filter((s) => (s.profile ?? "default") === profile).sort((x, y) => y.last_active - x.last_active);
```

Change `startRun(runId, sessionId, text, names)` to pass `profile` as a fifth argument. Change the signature to `function startRun(runId, sessionId, prompt, attachments, via)`, and change its first `say` to:

```js
    await say(`[${via}] You said: "${prompt.split("\n")[0].slice(0, 80)}". `);
```

Add a line to the mock's header comment: `Profiles: default (MOCK_KEY), orchestrator (hd-orch-key-7d3e9a1c5b), coder (no key).`

- [ ] **Step 8: Run the tests, then smoke-test the mock**

Run: `(cd src-tauri && cargo test) && node --check mock/server.mjs`
Expected: all tests pass, including the 2 new ones in `main.rs`.

Then start `node mock/server.mjs` and run:

```sh
curl -s -H 'Authorization: Bearer hd-orch-key-7d3e9a1c5b' http://127.0.0.1:18642/p/orchestrator/api/sessions | head -c 200; echo
curl -s -o /dev/null -w '%{http_code}\n' -H 'Authorization: Bearer hd-test-key-4f9c2a7e1b' http://127.0.0.1:18642/p/orchestrator/api/sessions
curl -s -o /dev/null -w '%{http_code}\n' -H 'Authorization: Bearer hd-test-key-4f9c2a7e1b' http://127.0.0.1:18642/p/nope/api/sessions
```

Expected: a JSON list, then `401`, then `404`.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/acp.rs mock/server.mjs
git commit -m "Route requests by Profile: /p/<name>/ with its own key, hermes -p over SSH

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Model list, and Model choice and Reasoning level on each Run

**Files:**
- Modify: `src-tauri/src/main.rs` (the `start_run` and `list_models` commands, and the handler list)
- Modify: `src-tauri/src/acp.rs` (the `Conn` fields, `request`, and a new `use_model`)
- Modify: `mock/server.mjs`

**Interfaces:**
- Consumes: `picker::{Models, ModelChoice, from_options, from_acp, acp_id, valid_reasoning, run_body}`.
- Produces:
  - Tauri command `list_models() -> { default: {provider, model} | null, groups: {provider, name, models: string[]}[] }`
  - `start_run` gains the arguments `model: {provider, model} | null` and `reasoning: string | null`
  - `Conn::models() -> Option<Value>`
  - `Conn::use_model(&self, session, Option<&ModelChoice>) -> Result<(), Error>`

- [ ] **Step 1: Cache models and switch them over SSH**

In `src-tauri/src/acp.rs`, add these to `struct Conn`, after `cwds`:

```rust
    /// `models` from the last `session/new` answer: the list and the Profile's default.
    models: Mutex<Option<Value>>,
    /// The model id this app last set on each Session.
    applied: Mutex<HashMap<String, String>>,
```

Initialize them in `connect` (`models: Mutex::default(), applied: Mutex::default(),`). Replace the last line of `Conn::request` with:

```rust
        let result = rx.await.unwrap_or_else(|_| Err(Error::Unreachable(self.closed_reason())));
        if method == "session/new" {
            if let Ok(created) = &result {
                *self.models.lock().unwrap() = Some(created["models"].clone());
            }
        }
        result
```

Add these to `impl Conn`:

```rust
    pub fn models(&self) -> Option<Value> {
        self.models.lock().unwrap().clone()
    }

    /// Over SSH the model is Session state: it's switched between Turns, and only when it changes.
    pub async fn use_model(&self, session: &str, model: Option<&ModelChoice>) -> Result<(), Error> {
        let applied = self.applied.lock().unwrap().get(session).cloned();
        let wanted = match model {
            Some(m) => acp_id(m),
            // Back to Profile default is needed only if this app moved the Session off it.
            None => match (&applied, self.models().and_then(|m| m["currentModelId"].as_str().map(str::to_owned))) {
                (Some(_), Some(default)) => default,
                _ => return Ok(()),
            },
        };
        if applied.as_deref() == Some(wanted.as_str()) {
            return Ok(());
        }
        self.request("session/set_model", json!({ "sessionId": session, "modelId": wanted })).await?;
        self.applied.lock().unwrap().insert(session.to_owned(), wanted);
        Ok(())
    }
```

Extend the picker import to `use crate::picker::{acp_id, parse_profile_list, ModelChoice, Profiles};`.

- [ ] **Step 2: Add `list_models` and extend `start_run`**

In `src-tauri/src/main.rs`, add this after `set_profile`:

```rust
/// Set-up providers and their models for the current Profile.
#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> Result<picker::Models, Error> {
    if state.ssh() {
        let hermes = state.hermes().await?;
        if let Some(models) = hermes.models() {
            return Ok(picker::from_acp(&models));
        }
        // ponytail: an unused Session per connection; Hermes never lists one with no messages.
        let created = hermes.request("session/new", json!({ "cwd": hermes.home, "mcpServers": [] })).await?;
        return Ok(picker::from_acp(&created["models"]));
    }
    let request = state.api(Method::GET, &["api", "model", "options"], &[])?;
    Ok(picker::from_options(&json_body(send(&state.client, request).await?).await?))
}
```

Register `list_models` in `generate_handler!`. Give `start_run` two new parameters after `files`:

```rust
    model: Option<picker::ModelChoice>,
    reasoning: Option<String>,
```

At the top of its body, add `let reasoning = reasoning.map(|r| picker::valid_reasoning(&r)).transpose()?;`. In the SSH branch, after the `let session = match session_id { ... };` block, add:

```rust
        hermes.use_model(&session, model.as_ref()).await?;
```

Replace the HTTP branch's body construction (from `let mut body = json!({ "input": input });` through the `session_id` `if`) with:

```rust
    let body = picker::run_body(input, session_id.as_deref(), model.as_ref(), reasoning.as_deref());
```

- [ ] **Step 3: Add model options to the mock and echo the choice**

In `mock/server.mjs`, in the API server handler, before `if (route === "GET /api/sessions")`, add:

```js
  if (route === "GET /api/model/options") {
    if (env.MOCK_NO_MODELS === "1") return send(res, 500, { error: { message: "Failed to list model options.", code: "model_options_failed" } });
    return send(res, 200, { provider: "mockai", model: "mock-large", providers: [
      { slug: "mockai", name: "Mock AI", authenticated: true, models: ["mock-large", "mock-small"] },
      { slug: "zai", name: "Z.ai", authenticated: true, models: ["glm-5.3-flash"] },
      { slug: "nous", name: "Nous Portal", authenticated: false, models: ["hermes-5"] },
    ] });
  }
```

In `POST /v1/runs`, replace the `startRun(...)` line with:

```js
    const model = body.model ? `${body.provider ?? "?"}/${body.model}` : "profile default";
    startRun(runId, sessionId, text, names, `${profile} · ${model} · reasoning ${body.model_options?.reasoning_effort ?? "default"}`);
```

- [ ] **Step 4: Build and test**

Run: `(cd src-tauri && cargo test) && node --check mock/server.mjs`
Expected: all pass.

Then, with the mock running:

```sh
curl -s -H 'Authorization: Bearer hd-test-key-4f9c2a7e1b' -d '{"input":"hi","provider":"zai","model":"glm-5.3-flash","model_options":{"reasoning_effort":"high"}}' http://127.0.0.1:18642/v1/runs
```

Expected: `{"run_id":"run_…","status":"started",…}`. The mock's own log prints the POST with status 202.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/main.rs src-tauri/src/acp.rs mock/server.mjs
git commit -m "List models and send Model choice and Reasoning level with each Run

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Attachments from the webview (paperclip and paste)

Picked and pasted files have no path the Rust side could read. The webview already holds their bytes, so it sends them as a data URL. Dropped files keep the path whitelist.

**Files:**
- Modify: `src-tauri/src/attach.rs`
- Modify: `src-tauri/src/main.rs` (`start_run`)

**Interfaces:**
- Produces:
  - `attach::Source`, deserialized untagged from `{ path }` (a dropped file) or `{ name, data_url }` (inline)
  - `attach::build_input(text: &str, files: &[Source]) -> Result<Value, String>`
  - `start_run`'s `files` argument becomes `Vec<attach::Source>`; the JS side sends `{ path }` or `{ name, data_url }` objects

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/attach.rs`, replace the test module's `temp` helper and its three tests with:

```rust
    fn temp(name: &str, bytes: &[u8]) -> Source {
        let path = std::env::temp_dir().join(format!("hd-attach-test-{name}"));
        std::fs::write(&path, bytes).unwrap();
        Source::Dropped { path }
    }

    fn inline(name: &str, data_url: &str) -> Source {
        Source::Inline { name: name.into(), data_url: data_url.into() }
    }

    #[test]
    fn text_is_inlined_with_a_fence_longer_than_its_content() {
        let input = build_input("see this", &[temp("notes.md", b"a ``` b")]).unwrap();
        assert_eq!(input, json!("see this\n\nAttached file `hd-attach-test-notes.md`:\n````\na ``` b\n````"));
    }

    #[test]
    fn images_become_data_url_parts() {
        let input = build_input("look", &[temp("pic.PNG", &[0x89, b'P', b'N', b'G'])]).unwrap();
        assert_eq!(input[0]["content"][0], json!({ "type": "text", "text": "look" }));
        assert_eq!(input[0]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBORw=="));
    }

    #[test]
    fn binary_and_oversized_files_are_refused() {
        assert!(build_input("", &[temp("blob.bin", &[0, 159, 146, 150])]).unwrap_err().contains("only text and image"));
        let big = temp("big.txt", &vec![b'a'; MAX_BYTES as usize + 1]);
        assert!(build_input("", &[big]).unwrap_err().contains("larger than 2 MB"));
    }

    #[test]
    fn inline_files_follow_the_same_rules() {
        let pasted = inline("Pasted image 1.png", "data:image/png;base64,iVBORw==");
        let picked = inline("todo.txt", &format!("data:text/plain;base64,{}", STANDARD.encode("buy milk")));
        let input = build_input("", &[picked, pasted]).unwrap();
        assert_eq!(input[0]["content"][0]["text"], json!("Attached file `todo.txt`:\n```\nbuy milk\n```"));
        assert_eq!(input[0]["content"][1]["image_url"]["url"], json!("data:image/png;base64,iVBORw=="));
        // FileReader writes an empty file as a bare "data:".
        assert_eq!(build_input("", &[inline("empty.txt", "data:")]).unwrap(), json!("Attached file `empty.txt`:\n```\n\n```"));
        let big = format!("data:text/plain;base64,{}", STANDARD.encode(vec![b'a'; MAX_BYTES as usize + 1]));
        assert!(build_input("", &[inline("big.txt", &big)]).unwrap_err().contains("larger than 2 MB"));
        assert!(build_input("", &[inline("x.txt", "not a data url")]).unwrap_err().contains("not a base64 data URL"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `(cd src-tauri && cargo test attach)`
Expected: FAIL to compile with `cannot find type Source`.

- [ ] **Step 3: Write the implementation**

In `src-tauri/src/attach.rs`, update the module doc comment and the imports, and replace `build_input` with the following. `image_mime` stays.

```rust
//! Builds a Run `input` from the typed text and Attachments. There is no upload endpoint on the
//! API server, so text files ride inline and images ride as `image_url` data-URL parts.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Where an Attachment's bytes come from: a path dropped on the window (checked against the
/// drop list), or bytes the webview already holds (the paperclip picker, a pasted image).
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Source {
    Dropped { path: PathBuf },
    Inline { name: String, data_url: String },
}

fn read(file: &Source) -> Result<(String, Vec<u8>), String> {
    let too_big = |name: &str| format!("{name} is larger than 2 MB");
    match file {
        Source::Dropped { path } => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let size = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?.len();
            if size > MAX_BYTES {
                return Err(too_big(&name));
            }
            Ok((name.clone(), std::fs::read(path).map_err(|e| format!("{name}: {e}"))?))
        }
        Source::Inline { name, data_url } => {
            let data = match data_url.split_once(',') {
                Some((head, data)) if head.starts_with("data:") && head.ends_with(";base64") => data,
                _ if data_url == "data:" => "",
                _ => return Err(format!("{name}: not a base64 data URL")),
            };
            let bytes = STANDARD.decode(data).map_err(|e| format!("{name}: {e}"))?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(too_big(name));
            }
            Ok((name.clone(), bytes))
        }
    }
}

pub fn build_input(text: &str, files: &[Source]) -> Result<Value, String> {
    let mut message = text.to_owned();
    let mut images = Vec::new();
    for file in files {
        let (name, bytes) = read(file)?;
        if let Some(mime) = image_mime(Path::new(&name)) {
            let url = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
            images.push(json!({ "type": "image_url", "image_url": { "url": url } }));
        } else if let Some(content) = String::from_utf8(bytes).ok().filter(|s| !s.contains('\0')) {
            let mut fence = "```".to_owned();
            while content.contains(&fence) {
                fence.push('`');
            }
            if !message.is_empty() {
                message.push_str("\n\n");
            }
            message.push_str(&format!("Attached file `{name}`:\n{fence}\n{content}\n{fence}"));
        } else {
            return Err(format!("{name}: only text and image files can be attached"));
        }
    }
    if images.is_empty() {
        return Ok(Value::String(message));
    }
    let mut content = vec![json!({ "type": "text", "text": message })];
    content.extend(images);
    Ok(json!([{ "role": "user", "content": content }]))
}
```

In `src-tauri/src/main.rs` `start_run`, change `files: Vec<PathBuf>` to `files: Vec<attach::Source>`. Replace the stray-file check (the first `if let Some(stray) = ...` block) with:

```rust
    let stray = {
        let dropped = state.dropped.lock().unwrap();
        files.iter().find_map(|f| match f {
            attach::Source::Dropped { path } if !dropped.contains(path) => Some(path.display().to_string()),
            _ => None,
        })
    };
    if let Some(stray) = stray {
        return Err(Error::Invalid(format!("{stray} was not dropped into this window")));
    }
```

If `PathBuf` is no longer used in `main.rs` except for `settings_path` and `dropped`, leave the import alone; those still use it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `(cd src-tauri && cargo test)`
Expected: all pass, including `inline_files_follow_the_same_rules`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/attach.rs src-tauri/src/main.rs
git commit -m "Accept Attachments the webview holds as data URLs

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: The transcription command

**Files:**
- Modify: `src-tauri/src/gateway.rs` (`client`, plus a new `slow_client`)
- Modify: `src-tauri/src/main.rs` (the `AppState` field, a new `transcribe` command, setup, and the handler list)
- Modify: `mock/server.mjs`

**Interfaces:**
- Produces: Tauri command `transcribe({ dataUrl: string }) -> string`, the transcript. It may be empty when no speech was heard.

- [ ] **Step 1: Add a client with no idle cutoff**

In `src-tauri/src/gateway.rs`, replace `pub fn client()` with:

```rust
fn builder() -> reqwest::ClientBuilder {
    Client::builder()
        // Never route credentials through a proxy host or follow a redirect off the gateway.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
}

pub fn client() -> Client {
    builder().read_timeout(IDLE_TIMEOUT).build().expect("static client config")
}

/// For a reply the server works on silently (speech-to-text): no idle cutoff, 2 min in total.
pub fn slow_client() -> Client {
    builder().timeout(Duration::from_secs(120)).build().expect("static client config")
}
```

- [ ] **Step 2: Add the command**

In `src-tauri/src/main.rs`, add `slow_client: reqwest::Client,` to `AppState`, after `client`. In setup, add `slow_client: gateway::slow_client(),`. Add this after `list_models`:

```rust
/// Dictation: the Dashboard transcribes with the current Profile's speech-to-text settings.
#[tauri::command]
async fn transcribe(state: State<'_, AppState>, data_url: String) -> Result<String, Error> {
    let profile = state.lock().profile.clone();
    let query: Vec<(&str, &str)> = profile.as_deref().map(|p| vec![("profile", p)]).unwrap_or_default();
    let request = state.dashboard(Method::POST, &["api", "audio", "transcribe"], &query)?.json(&json!({ "data_url": data_url }));
    let body = json_body(send(&state.slow_client, request).await?).await?;
    Ok(body["transcript"].as_str().unwrap_or_default().to_owned())
}
```

Register `transcribe` in `generate_handler!`.

- [ ] **Step 3: Add transcription to the mock**

In `mock/server.mjs`, in the Dashboard handler, after the `/api/profiles/active` route:

```js
  if (route === "POST /api/audio/transcribe") {
    const body = await readJson(req);
    if (!body?.data_url?.startsWith("data:audio/") && !body?.data_url?.startsWith("data:video/webm")) {
      return send(res, 400, { detail: "Payload must be an audio recording" });
    }
    await new Promise((resolve) => setTimeout(resolve, Number(env.MOCK_TRANSCRIBE_MS ?? 800)));
    const profileName = url.searchParams.get("profile") ?? "default";
    return send(res, 200, { ok: true, transcript: `hello from the mock microphone (${profileName})`, provider: "mock" });
  }
```

Add a line to the header comment: `MOCK_TRANSCRIBE_MS delays speech-to-text (try 20000 to outlast the 15 s idle timeout).`

- [ ] **Step 4: Build and test**

Run: `(cd src-tauri && cargo test) && node --check mock/server.mjs`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/gateway.rs src-tauri/src/main.rs mock/server.mjs
git commit -m "Add transcribe command for Dictation via the Dashboard

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: DOM helpers, icons, and themeable CSS

This is a refactor with one visible change: Theme and Skin now apply at boot. There are no new controls yet.

**Files:**
- Create: `src/dom.ts`
- Create: `src/appearance.ts` (only `applyAppearance` in this task)
- Modify: `src/main.ts` (remove `h`, `input`, `field`; import them and call `applyAppearance`)
- Modify: `src/style.css` (full rewrite)

**Interfaces:**
- Consumes: `prefs.ts` (`loadAppearance`, `FONT_PX`, `Appearance`).
- Produces:
  - `dom.ts`: `h`, `input`, `field` (same signatures as today in `main.ts`), and `icon(name: IconName): SVGSVGElement`, where `IconName` is `"paperclip" | "mic" | "user" | "cpu" | "brain" | "settings" | "chevron"`
  - `appearance.ts`: `applyAppearance(a?: Appearance): void`
  - CSS classes that later tasks use: `.composer-box`, `.toolbar`, `.spacer`, `.icon-btn`, `.picker`, `.menu`, `.model-label`, `.settings`, `aside footer`, `.chip img`

- [ ] **Step 1: Create `src/dom.ts`**

```ts
/** Tiny DOM helpers. Text always goes in as text; the only markup parsed is the constant icon set below. */

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const el = Object.assign(document.createElement(tag), props);
  el.append(...children);
  return el;
}

export function input(props: Partial<HTMLInputElement>): HTMLInputElement {
  return h("input", { required: true, spellcheck: false, autocomplete: "off", ...props });
}

export function field(label: string, control: HTMLElement): HTMLLabelElement {
  return h("label", {}, h("span", { textContent: label }), control);
}

// Lucide icons (ISC license, lucide.dev), 24×24 stroke paths.
const ICONS = {
  paperclip: '<path d="m16 6-8.414 8.586a2 2 0 0 0 2.829 2.829l8.414-8.586a4 4 0 1 0-5.657-5.657l-8.379 8.551a6 6 0 1 0 8.485 8.485l8.379-8.551"/>',
  mic: '<path d="M12 19v3"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><rect x="9" y="2" width="6" height="13" rx="3"/>',
  user: '<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>',
  cpu: '<rect width="16" height="16" x="4" y="4" rx="2"/><rect width="6" height="6" x="9" y="9" rx="1"/><path d="M15 2v2M15 20v2M2 15h2M2 9h2M20 15h2M20 9h2M9 2v2M9 20v2"/>',
  brain:
    '<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4"/>',
  settings:
    '<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
};
export type IconName = keyof typeof ICONS;

export function icon(name: IconName, size = 16): SVGSVGElement {
  const markup = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name]}</svg>`;
  return document.importNode(new DOMParser().parseFromString(markup, "image/svg+xml").documentElement as unknown as SVGSVGElement, true);
}
```

- [ ] **Step 2: Create `src/appearance.ts`**

```ts
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
```

- [ ] **Step 3: Use them from `main.ts`**

In `src/main.ts`:
- Delete the `h`, `input`, and `field` function definitions.
- Add `import { field, h, input } from "./dom";` and `import { applyAppearance } from "./appearance";` below the Tauri imports.
- Add `applyAppearance();` right above the `app` constant.

- [ ] **Step 4: Rewrite `src/style.css`**

Replace the whole file with:

```css
/* Theme: base colors (light here, .dark below). Skin: --accent and --accent-text only. */
:root {
  --font-size: 14px;
  --bg: #f4f4f7; --surface: #ffffff; --text: #1d1d24; --muted: #6b6b7b; --line: #d9d9e3;
  --hover: rgb(0 0 0 / 0.05); --notice: #fff4d6; --notice-line: #e8c96b; --error: #b3261e;
  --code-bg: rgb(0 0 0 / 0.07); --pre-bg: #1d1d24; --pre-text: #f4f4f7;
  --accent: #b8860b; --accent-text: #8b6508; --on-accent: #ffffff;
  --accent-bg: color-mix(in srgb, var(--accent) 12%, transparent);
  --accent-bg-strong: color-mix(in srgb, var(--accent) 20%, transparent);
  font: var(--font-size)/1.45 system-ui, sans-serif;
  color: var(--text);
  background: var(--bg);
}
:root.dark {
  color-scheme: dark;
  --bg: #111118; --surface: #1a1b23; --text: #e6e6ee; --muted: #9a9aaa; --line: #2c2d38;
  --hover: rgb(255 255 255 / 0.06); --notice: #3a3218; --notice-line: #6b5a22; --error: #f2b8b5;
  --code-bg: rgb(255 255 255 / 0.08); --pre-bg: #0b0b10;
  --accent: #ffd700; --accent-text: #ffd700; --on-accent: #111118;
}
/* Skins: accent palettes from hermes-webui (static/style.css). zeus keeps the default gold. */
:root[data-skin="ares"] { --accent: #c0392b; --accent-text: #922b21; }
:root.dark[data-skin="ares"] { --accent: #ff4444; --accent-text: #ff4444; }
:root[data-skin="mono"] { --accent: #666666; --accent-text: #555555; }
:root.dark[data-skin="mono"] { --accent: #cccccc; --accent-text: #cccccc; }
:root[data-skin="slate"] { --accent: #475569; --accent-text: #334155; }
:root.dark[data-skin="slate"] { --accent: #94a3b8; --accent-text: #94a3b8; }
:root[data-skin="poseidon"] { --accent: #0369a1; --accent-text: #025080; }
:root.dark[data-skin="poseidon"] { --accent: #0ea5e9; --accent-text: #0ea5e9; }
:root[data-skin="sisyphus"] { --accent: #7c3aed; --accent-text: #6d28d9; }
:root.dark[data-skin="sisyphus"] { --accent: #a78bfa; --accent-text: #a78bfa; }
:root[data-skin="charizard"] { --accent: #ea580c; --accent-text: #c2410c; }
:root.dark[data-skin="charizard"] { --accent: #fb923c; --accent-text: #fb923c; }
:root[data-skin="sienna"] { --accent: #d97757; --accent-text: #a55237; }
:root.dark[data-skin="sienna"] { --accent: #e0896d; --accent-text: #e6a88a; }
:root[data-skin="catppuccin"] { --accent: #8839ef; --accent-text: #8839ef; }
:root.dark[data-skin="catppuccin"] { --accent: #cba6f7; --accent-text: #cba6f7; }
:root[data-skin="nous"] { --accent: #4682b4; --accent-text: #2c5f88; }
:root.dark[data-skin="nous"] { --accent: #4682b4; --accent-text: #7eb6e0; }
:root[data-skin="geist-contrast"] { --accent: #0070f3; --accent-text: #005bd1; }
:root.dark[data-skin="geist-contrast"] { --accent: #fff175; --accent-text: #f5e65f; }

* { box-sizing: border-box; }
[hidden] { display: none !important; }
body { margin: 0; height: 100vh; display: flex; flex-direction: column; }
#app { flex: 1; min-height: 0; }
#banner { background: var(--notice); border-bottom: 1px solid var(--notice-line); padding: 6px 12px; }
h1, h2 { margin: 0 0 12px; font-weight: 600; }
h1 { font-size: 1.43rem; }
h2 { font-size: 1.07rem; }
button {
  font: inherit; border: 0; border-radius: 6px; padding: 7px 14px;
  background: var(--accent); color: var(--on-accent); cursor: pointer;
}
button:disabled { opacity: 0.45; cursor: default; }
button.secondary { background: var(--surface); color: var(--accent-text); border: 1px solid var(--line); }
button.link { background: none; color: var(--muted); padding: 0 4px; }
input, textarea, select {
  font: inherit; color: inherit; padding: 7px 9px; border: 1px solid var(--line); border-radius: 6px; background: var(--surface);
}
input, textarea { width: 100%; }
label { display: block; margin-bottom: 10px; }
label span { display: block; font-weight: 500; margin-bottom: 3px; }
.row { display: flex; gap: 8px; align-items: center; }
.error { color: var(--error); white-space: pre-wrap; }
.error:empty { display: none; }
.loading, .empty { color: var(--muted); text-align: center; margin-top: 40px; }
.card { max-width: 460px; margin: 48px auto; background: var(--surface); padding: 24px; border-radius: 10px; border: 1px solid var(--line); }
dialog { background: var(--surface); color: var(--text); border: 1px solid var(--line); border-radius: 10px; padding: 24px; width: 420px; border-top: 4px solid var(--error); }
dialog::backdrop { background: rgb(0 0 0 / 0.35); }

.chat { display: grid; grid-template-columns: 260px minmax(0, 1fr); height: 100%; position: relative; }
aside { border-right: 1px solid var(--line); background: var(--surface); padding: 12px; display: flex; flex-direction: column; min-height: 0; }
aside .row { justify-content: space-between; margin-bottom: 8px; }
aside h2 { margin: 0; }
aside footer { border-top: 1px solid var(--line); padding-top: 8px; margin-top: 8px; }
.sessions { list-style: none; margin: 0; padding: 0; flex: 1; overflow-y: auto; }
.sessions li { display: flex; justify-content: space-between; gap: 6px; padding: 7px 8px; border-radius: 6px; cursor: pointer; }
.sessions li button { white-space: nowrap; flex-shrink: 0; }
.sessions li span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sessions li:hover { background: var(--hover); }
.sessions li.current { background: var(--accent-bg-strong); font-weight: 500; }
main { display: flex; flex-direction: column; min-height: 0; }
.turns { flex: 1; overflow-y: auto; padding: 16px 20px; }
.turn { margin-bottom: 18px; }
.user, .reply { white-space: pre-wrap; overflow-wrap: anywhere; }
.user { background: var(--accent-bg); padding: 8px 12px; border-radius: 8px; margin-bottom: 8px; }
.turn[data-status="not-sent"] .user { opacity: 0.6; border: 1px dashed var(--error); }
.reply:empty { display: none; }
.model-label { color: var(--muted); font-size: 0.8rem; margin-top: 4px; }
pre { background: var(--pre-bg); color: var(--pre-text); padding: 10px; border-radius: 6px; overflow-x: auto; white-space: pre; }
code { font: 0.9rem ui-monospace, monospace; background: var(--code-bg); padding: 1px 4px; border-radius: 4px; }
pre code { background: none; padding: 0; }
.tool { font-size: 0.9rem; color: var(--muted); background: var(--surface); border: 1px solid var(--line); border-radius: 6px; padding: 3px 8px; margin: 3px 0; display: inline-block; }
.tool[data-state="failed"] { color: var(--error); }
.notice:empty { display: none; }
.notice { background: var(--notice); padding: 6px 10px; border-radius: 6px; }
.status { display: flex; gap: 10px; align-items: center; margin-top: 6px; color: var(--muted); }
.turn[data-status="failed"] .status, .turn[data-status="not-sent"] .status { color: var(--error); font-weight: 500; }
.chip { display: inline-flex; align-items: center; gap: 4px; background: var(--surface); border: 1px solid var(--line); border-radius: 12px; padding: 2px 8px; margin: 4px 4px 0 0; font-size: 0.9rem; }
.chip img { width: 28px; height: 28px; object-fit: cover; border-radius: 4px; }
.pending { padding: 0 20px; }
.pending .error { margin: 4px 0 0; }

.composer { padding: 12px 20px 16px; }
.composer-box { border: 1px solid var(--line); border-radius: 10px; background: var(--surface); padding: 8px 10px; }
.composer-box textarea { border: 0; padding: 4px 2px; resize: none; background: transparent; outline: none; }
.toolbar { display: flex; align-items: center; gap: 2px; margin-top: 6px; flex-wrap: wrap; }
.spacer { flex: 1; }
.icon-btn { display: inline-flex; align-items: center; gap: 6px; background: none; color: var(--muted); padding: 5px 8px; border-radius: 6px; }
.icon-btn:hover:not(:disabled) { background: var(--hover); color: var(--text); }
.icon-btn[data-recording] { color: var(--error); }
.picker { position: relative; display: inline-flex; align-items: center; gap: 4px; color: var(--muted); padding: 0 6px; border-radius: 6px; }
.picker:hover { background: var(--hover); }
.picker select { border: 0; background: transparent; padding: 5px 2px; cursor: pointer; appearance: none; font-size: 0.93rem; }
.picker.profile select { color: var(--accent-text); font-weight: 500; }
.picker .icon-btn { padding: 5px 2px; color: inherit; font-size: 0.93rem; }
.menu {
  position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 10; width: 320px; max-height: 360px; overflow-y: auto;
  background: var(--surface); border: 1px solid var(--line); border-radius: 8px; box-shadow: 0 8px 24px rgb(0 0 0 / 0.18); padding: 6px;
}
.menu input { margin-bottom: 6px; }
.menu h3 { font-size: 0.8rem; color: var(--muted); margin: 8px 6px 2px; font-weight: 600; }
.menu button { display: block; width: 100%; text-align: left; background: none; color: var(--text); padding: 5px 8px; }
.menu button:hover, .menu button[aria-selected="true"] { background: var(--accent-bg-strong); }
.menu p { color: var(--muted); margin: 8px 6px; }

.settings { display: grid; grid-template-columns: 200px minmax(0, 1fr); height: 100%; }
.settings nav { border-right: 1px solid var(--line); background: var(--surface); padding: 12px; display: flex; flex-direction: column; gap: 4px; }
.settings nav button { text-align: left; background: none; color: var(--text); }
.settings nav button[aria-current="page"] { background: var(--accent-bg-strong); }
.settings nav .secondary { margin-top: auto; }
.settings section { padding: 24px 32px; max-width: 560px; }
.settings select { min-width: 220px; }

.drop { position: absolute; inset: 0; display: grid; place-items: center; background: var(--accent-bg-strong); border: 3px dashed var(--accent); font-size: 1.3rem; font-weight: 600; color: var(--accent-text); pointer-events: none; }
```

- [ ] **Step 5: Verify**

Run: `npm test && npm run build`
Expected: a clean build.

Then run `npm run tauri dev` against the mock. In devtools, run `localStorage.setItem("appearance", JSON.stringify({theme:"dark",skin:"poseidon",fontSize:"large"})); location.reload()`.
Expected:
- a dark window with blue accents and larger text
- the Pairing card, chat, dialogs, and code blocks all readable
- running `localStorage.removeItem("appearance"); location.reload()` returns it to System

- [ ] **Step 6: Commit**

```bash
git add src/dom.ts src/appearance.ts src/main.ts src/style.css
git commit -m "Themeable CSS with hermes-webui skins; DOM helpers and icons module

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: The Settings view and the Appearance tab

**Files:**
- Modify: `src/appearance.ts` (add `appearanceTab`)
- Modify: `src/main.ts` (the gear footer in `showChat`, and a new `showSettings`)

**Interfaces:**
- Consumes: `dom.ts` (`h`, `field`, `icon`), `prefs.ts` (`SKINS`, `saveAppearance`).
- Produces: `appearanceTab(): HTMLElement`, and `showSettings()` in `main.ts`.

- [ ] **Step 1: Add the tab**

Append to `src/appearance.ts`:

```ts
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
```

Extend its imports to:

```ts
import { field, h } from "./dom";
import { FONT_PX, SKINS, loadAppearance, saveAppearance, type Appearance } from "./prefs";
```

- [ ] **Step 2: Add the gear and the view**

In `src/main.ts`, import `appearanceTab` next to `applyAppearance`, and add `icon` to the `./dom` import. In `showChat`, change the `h("aside", ...)` call so it ends with a footer, after `ui.sessions`:

```ts
      h("aside", {}, h("div", { className: "row" }, h("h2", { textContent: "Sessions" }),
        h("button", { className: "secondary", textContent: "New chat", onclick: newChat })), ui.sessionsError, ui.sessions,
        h("footer", {}, h("button", { className: "icon-btn", title: "Settings", onclick: showSettings }, icon("settings"), "Settings"))),
```

Add this after `showChat`:

```ts
/** Settings replaces the chat view until closed; the chat keeps running underneath. */
function showSettings() {
  const chat = app.querySelector<HTMLElement>(".chat")!;
  const view = h(
    "div",
    { className: "settings" },
    h("nav", {}, h("h2", { textContent: "Settings" }), h("button", { textContent: "Appearance", ariaCurrent: "page" }),
      h("button", { className: "secondary", textContent: "Back to chat", onclick: () => { view.remove(); chat.hidden = false; } })),
    appearanceTab(),
  );
  chat.hidden = true;
  app.append(view);
}
```

- [ ] **Step 3: Verify**

Run: `npm run build`, then `npm run tauri dev` against the mock, and pair.
Expected:
- **Settings** sits at the bottom of the Sessions panel.
- It opens a view with **Appearance** highlighted.
- Changing Theme, Skin, or Font size applies immediately.
- **Back to chat** returns with the conversation intact.
- A reply that was streaming before opening Settings is still complete afterwards.
- Restarting the app keeps the chosen appearance.

- [ ] **Step 4: Commit**

```bash
git add src/appearance.ts src/main.ts
git commit -m "Add Settings view with Appearance tab: Theme, Skin, Font size

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: Composer bar, paperclip, and image paste

**Files:**
- Modify: `src/main.ts`

**Interfaces:**
- Consumes: `start_run`'s new `files: ({ path } | { name, data_url })[]` (Task 6), and `icon` (Task 8).
- Produces:
  - `type Attachment = { path: string } | { name: string; data_url: string }`
  - `Turn.files: Attachment[]`
  - `ui.toolbar: HTMLElement` and `ui.composerError: HTMLElement`
  - `dataUrl(blob): Promise<string>`; Task 13 uses `ui.composerError` and `dataUrl`
  - Toolbar order: `[paperclip, <hidden file input>, …later controls…, spacer, send]`. Later tasks insert their controls with `ui.toolbar.insertBefore(el, ui.spacer)`.

- [ ] **Step 1: Switch Attachments from paths to objects**

In `src/main.ts`:
- Add `type Attachment = { path: string } | { name: string; data_url: string };` next to the other types.
- Add `const attachmentName = (a: Attachment) => ("path" in a ? basename(a.path) : a.name);` next to `basename`.
- In `interface Turn`, change `files: string[];` to `files: Attachment[];`.
- Change `let pending: string[] = [];` to `let pending: Attachment[] = [];`.
- Change `addTurn(text: string, files: string[])` to `addTurn(text: string, files: Attachment[])`. Replace its `if (files.length) user.append(...)` line with:

```ts
  if (files.length) user.append(h("div", { className: "files" }, ...files.map((f) => chip(f))));
```

Add this helper above `renderPending`:

```ts
/** An Attachment chip: a thumbnail for images the webview holds, a paperclip otherwise. */
function chip(a: Attachment, onRemove?: () => void): HTMLElement {
  const thumb = "data_url" in a && a.data_url.startsWith("data:image/") ? h("img", { src: a.data_url, alt: "" }) : "📎";
  const remove = onRemove ? [h("button", { className: "link", textContent: "×", title: "Remove attachment", onclick: onRemove })] : [];
  return h("span", { className: "chip" }, thumb, attachmentName(a), ...remove);
}
```

Replace `renderPending` with:

```ts
function renderPending() {
  ui!.pending.replaceChildren(
    ...pending.map((a) => chip(a, () => {
      pending = pending.filter((p) => p !== a);
      renderPending();
    })),
    ui!.composerError,
  );
}
```

In the drag-drop handler, replace the `pending.push(...)` line with:

```ts
    pending.push(...payload.paths.filter((p) => !pending.some((a) => "path" in a && a.path === p)).map((path) => ({ path })));
```

In `send()`, add `ui!.composerError.textContent = "";` after `pending = [];`.

- [ ] **Step 2: Build the composer bar**

In the `ui` type, add `composerError: HTMLElement; toolbar: HTMLElement; spacer: HTMLElement;`. In `showChat`'s `ui = { ... }`, add:

```ts
    composerError: h("p", { className: "error" }),
    toolbar: h("div", { className: "toolbar" }),
    spacer: h("span", { className: "spacer" }),
```

Change the textarea placeholder to `"Message Hermes. Drop, paste, or attach files."`. Then, after `const { input: box, send: button } = ui;`, add:

```ts
  const picker = h("input", { type: "file", multiple: true, hidden: true });
  picker.onchange = async () => {
    await addFiles([...(picker.files ?? [])]);
    picker.value = "";
  };
  const paperclip = h("button", { type: "button", className: "icon-btn", title: "Attach files", onclick: () => picker.click() }, icon("paperclip"));
  paperclip.disabled = !runs;
  ui.toolbar.append(paperclip, picker, ui.spacer, button);
  box.onpaste = (e) => void pasteImages(e);
```

Replace `h("div", { className: "composer" }, box, button)` in the layout with:

```ts
h("div", { className: "composer" }, h("div", { className: "composer-box" }, box, ui.toolbar))
```

- [ ] **Step 3: Add the file and paste handling**

Add this below `renderPending`:

```ts
const MAX_BYTES = 2 * 1024 * 1024; // attach.rs enforces it; checked here only to avoid reading huge files
const SENDABLE = ["image/png", "image/jpeg", "image/gif", "image/webp"];
let pasted = 0;

const dataUrl = (blob: Blob) =>
  new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });

async function addFiles(files: File[]) {
  for (const file of files) {
    if (file.size > MAX_BYTES) ui!.composerError.textContent = `Not attached: ${file.name} is larger than 2 MB`;
    else pending.push({ name: file.name, data_url: await dataUrl(file) });
  }
  renderPending();
}

/** Screenshots are often over 2 MB or an unsendable type: re-encode as JPEG, shrinking until one fits. */
async function fitImage(blob: Blob): Promise<Blob> {
  if (blob.size <= MAX_BYTES && SENDABLE.includes(blob.type)) return blob;
  const bitmap = await createImageBitmap(blob);
  for (const scale of [1, 0.75, 0.5, 0.35]) {
    const canvas = h("canvas", { width: Math.round(bitmap.width * scale), height: Math.round(bitmap.height * scale) });
    const ctx = canvas.getContext("2d")!;
    ctx.fillStyle = "#fff"; // JPEG has no transparency
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const jpeg = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.85));
    if (jpeg && jpeg.size <= MAX_BYTES) return jpeg;
  }
  throw new Error("the pasted image is too large even after compressing");
}

/** Pasted images become Attachments; any text in the clipboard still pastes as usual. */
async function pasteImages(e: ClipboardEvent) {
  const images = [...(e.clipboardData?.items ?? [])]
    .filter((i) => i.kind === "file" && i.type.startsWith("image/"))
    .map((i) => i.getAsFile())
    .filter((f): f is File => f !== null);
  if (!images.length || !runs) return;
  for (const image of images) {
    try {
      const fitted = await fitImage(image);
      const ext = fitted.type === "image/jpeg" ? "jpg" : fitted.type.split("/")[1];
      pending.push({ name: `Pasted image ${++pasted}.${ext}`, data_url: await dataUrl(fitted) });
    } catch (err) {
      ui!.composerError.textContent = `Not attached: ${err instanceof Error ? err.message : String(err)}`;
    }
  }
  renderPending();
}
```

- [ ] **Step 4: Verify**

Run: `npm run build`, then `npm run tauri dev` against the mock.
Expected:
- **Paperclip:** pick a `.txt` file and a small `.png`. Both appear as chips (the PNG with a thumbnail). Send, and the mock replies "I received 2 attachment(s)". Picking a 3 MB file shows "Not attached: … larger than 2 MB".
- **Paste a small screenshot** (for example `gnome-screenshot -c -a`, or Print Screen to clipboard): a "Pasted image 1.png" chip with a thumbnail appears. Send, and the mock reports 1 attachment.
- **Paste a full-screen screenshot on a large display,** or any image over 2 MB: a `.jpg` chip appears. Check it's below 2 MB in devtools with `pending.at(-1).data_url.length * 0.75`.
- **Paste plain text:** it lands in the box as before, with no chip.
- **Drag-and-drop** still works.
- **Retry** on a failed Turn (`fail` keyword) resends the same Attachments.

- [ ] **Step 5: Commit**

```bash
git add src/main.ts
git commit -m "Composer bar with paperclip picker and image paste

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: Profile selector

**Files:**
- Modify: `src/main.ts`

**Interfaces:**
- Consumes: `list_profiles` and `set_profile` (Task 4); `prefs.ts` (`startProfile`, `saveProfile`); `icon("user")`.
- Produces:
  - Globals `connection: string` and `profile: string`, which Task 12 uses for choice keys
  - `useProfile(name, previous)`
  - `refreshModels()`, called from `useProfile`; Task 12 defines its body, and this task adds an empty stub

- [ ] **Step 1: Track the connection and the Profile**

In `src/main.ts`, next to `let runs = false;`:

```ts
let connection = ""; // which Gateway or SSH host; Profiles and per-Session choices are remembered per connection
let profile = "default";
const connectionId = (dash: string, api: string) => `${new URL(dash).origin}|${new URL(api).origin}`;
```

Set it in these places:
- In `boot()`, after `noteSaved(init.keyring);`, add `connection = init.ssh_host ? \`ssh:${init.ssh_host}\` : init.dashboard_url && init.api_url ? connectionId(init.dashboard_url, init.api_url) : "";`.
- In `showPairing`'s Pair submit handler, before `showChat(accepted.runs)`, add `connection = connectionId(dashUrl.value, apiUrl.value);`.
- In `sshForm.onsubmit`, before `showChat(true, true)`, add `connection = \`ssh:${sshHost.value.trim()}\`;`.

Import `saveProfile` and `startProfile` from `./prefs`.

- [ ] **Step 2: Let the Re-auth prompt be cancellable**

Change the `reauth` signature and return type to:

```ts
/** Blocks until the rejected credential is replaced. For a new key, resolves with whether chat is supported;
 *  with `cancel`, the prompt can also be left, resolving `null`. */
function reauth(kind: "sign-in" | "key", reason: string, cancel?: string): Promise<boolean | undefined | null> {
```

Replace the secondary "Use a different gateway" button with:

```ts
      h("div", { className: "row" }, submit, h("button", {
        type: "button",
        className: "secondary",
        textContent: cancel ?? "Use a different gateway",
        onclick: () => {
          dialog.close();
          if (cancel) return resolve(null);
          showPairing();
        },
      })),
```

Change `setRuns` to accept the wider type:

```ts
function setRuns(next: boolean | undefined | null) {
  if (typeof next === "boolean") runs = next;
  updateComposer();
}
```

In `connect()`, change `supported = (await reauth(...))!;` to `supported = (await reauth("key", "The API server did not accept the saved API key.")) ?? false;`.

- [ ] **Step 3: Add the selector and switching**

In the `ui` type, add `profile: HTMLSelectElement;`. In `ui = { ... }`, add `profile: h("select", { title: "Profile" }),`. After the toolbar line from Task 10, add:

```ts
  ui.profile.onchange = () => void useProfile(ui!.profile.value, profile);
  ui.toolbar.insertBefore(h("span", { className: "picker profile", title: "Profile" }, icon("user"), ui.profile), ui.spacer);
```

In `showChat`, replace the trailing `refreshSessions();` call with `void setupProfiles();`. Add these functions after `refreshSessions`:

```ts
/** Fills the Profile selector and opens this connection's starting Profile. */
async function setupProfiles() {
  let list: { names: string[]; active: string | null };
  try {
    list = await invoke("list_profiles");
  } catch (e) {
    ui!.sessionsError.textContent = `Couldn't list profiles: ${asError(e).message}`;
    list = { names: ["default"], active: null };
  }
  if (!list.names.length) list.names = ["default"];
  ui!.profile.replaceChildren(...list.names.map((n) => h("option", { value: n, textContent: n })));
  const start = startProfile(localStorage, connection, list.names, list.active);
  await useProfile(start, start === "default" || !list.names.includes("default") ? null : "default");
}

/** Switches the whole view to a Profile: its Sessions, its API key, and a new chat. */
async function useProfile(name: string, previous: string | null) {
  leave();
  try {
    await invoke("set_profile", { name });
    if (!overSsh) {
      const supported = await profileKey(name, previous);
      if (supported === null) return void (previous && (await useProfile(previous, null)));
      setRuns(supported);
    }
  } catch (e) {
    ui!.sessionsError.textContent = `Couldn't open the ${name} profile: ${asError(e).message}`;
    if (previous) await useProfile(previous, null);
    return;
  }
  profile = name;
  ui!.profile.value = name;
  saveProfile(localStorage, connection, name);
  ui!.sessionsError.textContent = "";
  newChat();
  await Promise.all([refreshSessions(), refreshModels()]);
}

/** Over HTTP each named Profile has its own API key: asks once, and the keyring keeps it. `null` = went back. */
async function profileKey(name: string, previous: string | null): Promise<boolean | null> {
  try {
    return await invoke<boolean>("capabilities");
  } catch (e) {
    if (asError(e).kind !== "unauthorized") throw e;
    const reason = `The ${name} profile has its own API key (API_SERVER_KEY in that profile's .env). Enter it once; it's saved with your other credentials.`;
    const supported = await reauth("key", reason, previous ? `Back to ${previous}` : undefined);
    return supported === null ? null : supported ?? false;
  }
}

async function refreshModels() {} // filled in by the model picker
```

- [ ] **Step 4: Verify**

Run: `npm run build`, then `npm run tauri dev` against a fresh mock (`rm -f /tmp/hermes-mock-state.json; node mock/server.mjs`).
Expected:
- **First launch after Pairing:** the selector shows `default`, `orchestrator`, `coder`. `orchestrator` is selected (the mock's Gateway default), and a key prompt appears. Enter `hd-orch-key-7d3e9a1c5b`, and the chat opens.
- **Profile-scoped Sessions:** sending "hi" gets a reply starting `[orchestrator · …]`. The Sessions panel shows only orchestrator's Sessions. Switching to `default` shows none of them.
- **No key on the gateway:** switch to `coder`. The key prompt shows **Back to default**. Enter anything, and it says "The API server rejected this key". Click **Back to default**, and the selector shows `default` again with default's Sessions (Review Focus 2).
- **Switching mid-stream:** in `default`, send "slow tell me a story" and switch to `orchestrator` mid-reply. The reply stops as Stopped, and the orchestrator view shows no text from it (Review Focus 3).
- **Remembered:** restart the app. It opens on the last Profile, with no key prompt (the key came from the keyring).
- **Over SSH** (`HERMES_DESKTOP_SSH=$PWD/mock/fake-ssh npm run tauri dev`, host `localhost`, needs a local `hermes`): the selector lists your real profiles. Switching reconnects, and the Sessions panel changes.

- [ ] **Step 5: Commit**

```bash
git add src/main.ts
git commit -m "Profile selector: per-Profile Sessions and API keys, remembered per connection

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 12: Model and Reasoning level pickers, per-Session memory, and reply labels

**Files:**
- Modify: `src/main.ts`

**Interfaces:**
- Consumes: `list_models` and `start_run(model, reasoning)` (Task 5); `prefs.ts` (`Choice`, `ModelChoice`, `DEFAULT_CHOICE`, `REASONING_LEVELS`, `choiceKey`, `loadChoice`, `saveChoice`, `changeLabel`); `connection` and `profile` (Task 11).
- Produces: `Turn.choice: Choice`.

- [ ] **Step 1: State and types**

Add these near the other types:

```ts
type Models = { default: ModelChoice | null; groups: { provider: string; name: string; models: string[] }[] };
```

Add this near the other globals:

```ts
let models: Models = { default: null, groups: [] };
let choice: Choice = DEFAULT_CHOICE; // what the next Turn in this view runs on
let lastChoice: Choice | undefined; // what the previous Turn ran on, for the reply label
```

Import `changeLabel`, `choiceKey`, `DEFAULT_CHOICE`, `loadChoice`, `REASONING_LEVELS`, `saveChoice`, `type Choice`, and `type ModelChoice` from `./prefs`. In `interface Turn`, add `choice: Choice;`. In `addTurn`, add `choice: DEFAULT_CHOICE,` to the turn object.

- [ ] **Step 2: The pickers**

In the `ui` type, add `modelPicker: HTMLElement; modelLabel: HTMLElement; menu: HTMLElement; reasoning: HTMLSelectElement;`. In `ui = { ... }`, add:

```ts
    modelPicker: h("span", { className: "picker model" }),
    modelLabel: h("span"),
    menu: h("div", { className: "menu" }),
    reasoning: h("select", {}, h("option", { value: "", textContent: "Default" }),
      ...REASONING_LEVELS.map((r) => h("option", { value: r, textContent: r }))),
```

After the Profile picker insertion from Task 11, add:

```ts
  ui.modelPicker.append(h("button", { type: "button", className: "icon-btn", title: "Model", onclick: toggleModelMenu }, icon("cpu"), ui.modelLabel, icon("chevron", 12)));
  const reasoningPicker = h("span", { className: "picker", title: "Reasoning level" }, icon("brain"), ui.reasoning);
  if (overSsh) {
    ui.reasoning.disabled = true;
    reasoningPicker.title = "Over SSH, the reasoning level is set in the profile's config";
  }
  ui.reasoning.onchange = () => setChoice({ ...choice, reasoning: ui!.reasoning.value || null });
  ui.toolbar.insertBefore(ui.modelPicker, ui.spacer);
  ui.toolbar.insertBefore(reasoningPicker, ui.spacer);
```

Replace the `refreshModels` stub from Task 11 and add the helpers:

```ts
async function refreshModels() {
  try {
    models = await invoke<Models>("list_models");
    ui!.modelPicker.title = "Model";
  } catch (e) {
    models = { default: null, groups: [] };
    ui!.modelPicker.title = `Couldn't list models: ${asError(e).message}. Runs use the profile's default.`;
  }
  renderChoice();
}

const modelLabel = (m: ModelChoice | null) => m?.model ?? `Profile default${models.default ? ` (${models.default.model})` : ""}`;

function renderChoice() {
  ui!.modelLabel.textContent = modelLabel(choice.model);
  ui!.reasoning.value = choice.reasoning ?? "";
}

/** Takes effect from the next Turn, and is saved per Session once the Session exists. */
function setChoice(next: Choice) {
  choice = next;
  if (sessionId) saveChoice(localStorage, choiceKey(connection, profile, sessionId), choice);
  renderChoice();
}

/** The CPU picker: set-up providers only, grouped, filtered by the search box. */
function toggleModelMenu() {
  const { menu, modelPicker } = ui!;
  if (menu.isConnected) return menu.remove();
  const search = h("input", { type: "search", placeholder: "Search models", required: false });
  const list = h("div");
  const item = (label: string, m: ModelChoice | null) => {
    const b = h("button", { type: "button", textContent: label, onclick: () => (setChoice({ ...choice, model: m }), menu.remove()) });
    b.ariaSelected = String(JSON.stringify(m) === JSON.stringify(choice.model));
    return b;
  };
  const render = () => {
    const q = search.value.trim().toLowerCase();
    const groups = models.groups
      .map((g) => ({ ...g, models: g.models.filter((m) => `${g.name} ${m}`.toLowerCase().includes(q)) }))
      .filter((g) => g.models.length);
    list.replaceChildren(
      ...(q ? [] : [item(modelLabel(null), null)]),
      ...groups.flatMap((g) => [h("h3", { textContent: g.name }), ...g.models.map((m) => item(m, { provider: g.provider, model: m }))]),
      ...(groups.length ? [] : [h("p", { textContent: q ? "No matching models" : modelPicker.title })]),
    );
  };
  search.oninput = render;
  search.onkeydown = (e) => void (e.key === "Escape" && menu.remove());
  render();
  menu.replaceChildren(search, list);
  modelPicker.append(menu);
  search.focus();
}

document.addEventListener("pointerdown", (e) => {
  if (ui && !ui.modelPicker.contains(e.target as Node)) ui.menu.remove();
});
```

- [ ] **Step 3: Send the choice and remember it per Session**

In `send()`, after `const turn = addTurn(text, pending);`:

```ts
  turn.choice = choice;
  const label = changeLabel(lastChoice, choice);
  lastChoice = choice;
  if (label) turn.replyEl.after(h("div", { className: "model-label", textContent: label }));
```

In `beginRun`, change the `start_run` invoke to:

```ts
    run = await invoke<RunStarted>("start_run", {
      sessionId, text: turn.text, files: turn.files, model: turn.choice.model, reasoning: turn.choice.reasoning,
    });
```

After `sessionId = run.session_id;`, add `saveChoice(localStorage, choiceKey(connection, profile, run.session_id), choice);`.

In `newChat()`, after `sessionId = null;`, add `choice = DEFAULT_CHOICE; lastChoice = undefined; renderChoice();`. In `openSession(id)`, after `sessionId = id;`, add:

```ts
  choice = loadChoice(localStorage, choiceKey(connection, profile, id));
  lastChoice = choice;
  renderChoice();
```

- [ ] **Step 4: Verify**

Run: `npm test && npm run build`, then `npm run tauri dev` against the mock (Profile `default`).
Expected:
- **The list:** the model menu shows "Profile default (mock-large)" first, then **Mock AI** (mock-large, mock-small) and **Z.ai** (glm-5.3-flash). Nous Portal is absent. Typing `glm` filters to Z.ai only. Escape and clicking outside both close it.
- **Switching mid-Session:** send "hi", and the reply starts `[default · profile default · reasoning default]` with no label. Pick glm-5.3-flash and reasoning high, then send "again". The reply starts `[default · zai/glm-5.3-flash · reasoning high]`, and the label under it reads `glm-5.3-flash · high`. A third send with no change has no label.
- **During a stream:** change the model while a `slow` reply streams. That reply finishes, and the next Turn uses the new model.
- **Remembered per Session:** open another Session, then return. The selectors show glm-5.3-flash / high again. Restarting the app keeps that. A new chat resets to Profile default / Default.
- **Model list failure:** restart the mock with `MOCK_NO_MODELS=1`. The menu shows only "Profile default" plus "Couldn't list models: …", and sending still works (Review Focus 4).
- **Over SSH** (fake-ssh): the menu lists your real providers. Picking a model and sending works, and Hermes' reply reflects it. The brain selector is disabled with its tooltip.

- [ ] **Step 5: Commit**

```bash
git add src/main.ts
git commit -m "Model and Reasoning level pickers, remembered per Session, labelled on change

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 13: Dictation

**Files:**
- Modify: `src/main.ts`

**Interfaces:**
- Consumes: `transcribe({ dataUrl })` (Task 7), `ui.composerError` and `dataUrl` (Task 10), and the mic access proven in Task 1.

- [ ] **Step 1: Add the mic button**

In the `ui` type, add `mic: HTMLButtonElement; micTime: HTMLElement;`. In `ui = { ... }`, add:

```ts
    mic: h("button", { type: "button", className: "icon-btn", title: "Dictate" }),
    micTime: h("span"),
```

After `ui.toolbar.append(paperclip, picker, ui.spacer, button);` from Task 10, add:

```ts
  ui.mic.append(icon("mic"), ui.micTime);
  ui.mic.onclick = () => void toggleDictation();
  // Speech-to-text is a Dashboard feature; over SSH there is no Dashboard.
  ui.mic.hidden = overSsh || !navigator.mediaDevices?.getUserMedia;
  ui.toolbar.insertBefore(ui.mic, picker.nextSibling);
```

- [ ] **Step 2: Record and transcribe**

Add this after `pasteImages`:

```ts
let recorder: MediaRecorder | null = null;
const clock = (ms: number) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;

/** Dictation: click to record, click again to insert the transcript at the cursor. */
async function toggleDictation() {
  if (recorder) return recorder.stop();
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (e) {
    ui!.composerError.textContent = `Microphone unavailable: ${e instanceof Error ? e.message : String(e)}`;
    return;
  }
  const rec = new MediaRecorder(stream);
  const chunks: Blob[] = [];
  const started = Date.now();
  const tick = setInterval(() => (ui!.micTime.textContent = clock(Date.now() - started)), 250);
  const limit = setTimeout(() => rec.stop(), 120_000); // ponytail: 2 min cap per note; chunked transcription if longer dictation matters
  recorder = rec;
  ui!.mic.dataset.recording = "";
  ui!.mic.title = "Stop and transcribe";
  ui!.composerError.textContent = "";
  rec.ondataavailable = (e) => chunks.push(e.data);
  rec.onstop = async () => {
    clearInterval(tick);
    clearTimeout(limit);
    stream.getTracks().forEach((t) => t.stop());
    recorder = null;
    delete ui!.mic.dataset.recording;
    ui!.mic.title = "Dictate";
    ui!.micTime.textContent = "…";
    ui!.mic.disabled = true;
    try {
      const text = await invoke<string>("transcribe", { dataUrl: await dataUrl(new Blob(chunks, { type: rec.mimeType })) });
      const box = ui!.input;
      if (text) box.setRangeText(text, box.selectionStart, box.selectionEnd, "end");
      box.focus();
    } catch (e) {
      ui!.composerError.textContent = `Dictation failed: ${asError(e).message}`;
    } finally {
      ui!.micTime.textContent = "";
      ui!.mic.disabled = false;
    }
  };
  rec.start();
}
```

- [ ] **Step 3: Verify**

Run: `npm run build`, then `npm run tauri dev` against the mock.
Expected:
- **Recording:** type "Note: ", place the cursor at the end, and click the mic. It turns red with a running timer. Click again, and "…" shows briefly. Then `hello from the mock microphone (default)` is inserted at the cursor. The mic works again straight after.
- **Profile-aware:** switch to `orchestrator` and dictate again. The transcript ends `(orchestrator)`.
- **Failure:** stop the mock and dictate. "Dictation failed: …" appears, and the box text is unchanged.
- **Slow transcription:** restart the mock with `MOCK_TRANSCRIBE_MS=20000` and dictate. After about 20 s the transcript appears, with no timeout (Review Focus 5).
- **Auto-stop:** the recording stops on its own at 2:00.
- **Over SSH:** the mic is hidden.
- **Release build:** run `npm run tauri build -- --no-bundle`, then `src-tauri/target/release/hermes-desktop`. The mic still records; the production custom-scheme page must still count as a secure context. If the mic is hidden there, `navigator.mediaDevices` is missing in release: report it rather than working around it.

- [ ] **Step 4: Commit**

```bash
git add src/main.ts
git commit -m "Dictation: record, transcribe via the Dashboard, insert at the cursor

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```

---

### Task 14: README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the docs**

In `README.md`:

1. Change the opening sentence's feature list to: "pairing and sign-in, streaming chat with tool progress and Stop, a Sessions panel, Profile, model, and Reasoning level pickers, Dictation, drag-and-drop, picked, and pasted attachments, and Appearance settings".

2. Add this section before `## Where things are stored`:

```markdown
## Profiles, models, and reasoning

The bar under the message box picks the **Profile** (person icon), model (CPU icon), and Reasoning level (brain icon).

- Switching Profile shows that Profile's Sessions and starts a new chat.
- The app opens on the Profile you last used, or on the Gateway's default Profile (`hermes profile use`). It never changes that default itself.
- Over HTTP, a named Profile is reached at `/p/<name>/…` and needs its own `API_SERVER_KEY` (from that Profile's `.env`). The app asks for it the first time and keeps it in the keyring.
- Over SSH, switching restarts Hermes as `hermes -p <name> acp`.
- The model list shows only providers you've set up.
- The model and Reasoning level belong to each Session, can change partway through it, and take effect from the next Turn. A small label marks the first reply after a change.
- Over SSH, the Reasoning level comes from the Profile's config and can't be changed here.

## Dictation

The microphone records until you click it again, or for up to 2 minutes. The Dashboard's speech-to-text (`POST /api/audio/transcribe`) then turns the recording into text, which is inserted at the cursor for you to edit. This uses the Profile's own voice settings. It isn't available over SSH.
```

3. In `## Where things are stored`, add this bullet after the keyring bullet:

```markdown
- Named Profiles' API keys are stored in that same keyring entry.
- The webview's own storage holds Appearance settings (Theme, Skin, Font size), the last Profile used on each connection, and each Session's model and Reasoning level. It never holds credentials.
```

4. Replace the `## Attachments` intro line with: "The API server has no upload endpoint, so Attachments ride inside the Run request. Drop files on the window, pick them with the paperclip, or paste an image."

   Then add this bullet: "Pasted images over 2 MB, or in a format that can't be sent, are re-encoded as JPEG and scaled down until they fit."

5. In `## Testing against the mock gateway`, add after the user/key line:

```markdown
Profiles: `default` (the key above), `orchestrator` (`hd-orch-key-7d3e9a1c5b`), and `coder` (no key, so switching to it is refused). Replies start with `[profile · model · reasoning]` so you can see what a Run was sent with. `MOCK_NO_MODELS=1` makes the model list fail; `MOCK_TRANSCRIBE_MS` delays speech-to-text.
```

6. In `## Build and run (development)`, add `npm test                             # webview preferences (Node 22.18+)` to the command block.

- [ ] **Step 2: Final check and commit**

Run: `(cd src-tauri && cargo test) && npm test && npm run build`
Expected: everything passes.

```bash
git add README.md
git commit -m "README: Profiles, models, reasoning, Dictation, pasted attachments, Appearance

Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>"
```
