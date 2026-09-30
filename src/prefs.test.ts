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
  saveChoice(store, b, { model: null, reasoning: "ultra" }); // the top of the gateway's ladder
  assert.deepEqual(loadChoice(store, b), { model: null, reasoning: "ultra" });
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
