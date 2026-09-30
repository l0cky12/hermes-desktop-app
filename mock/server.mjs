// Test-only mock of the Hermes gateway surfaces Hermes Desktop uses. Not shipped with the app.
// Wire formats mirror hermes-agent: hermes_cli/dashboard_auth/{routes,middleware}.py (Dashboard)
// and gateway/platforms/api_server{,_runs}.py (API server).
//
//   node mock/server.mjs        Dashboard on :19119, API server on :18642
//
// Prompt keywords script the reply: "tool" (tool events), "idle" (12 s silence, so a 10 s
// keepalive fires mid-stream), "slow" (long, slow reply for kill tests), "approval", "fail".
// MOCK_TRANSCRIBE_MS delays speech-to-text (try 20000 to outlast the 15 s idle timeout).
// Profiles: default (MOCK_KEY), orchestrator (hd-orch-key-7d3e9a1c5b), coder (no key).
import http from "node:http";
import { randomBytes } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";

const env = process.env;
const DASH_PORT = Number(env.MOCK_DASH_PORT ?? 19119);
const API_PORT = Number(env.MOCK_API_PORT ?? 18642);
const USER = env.MOCK_USER ?? "tester";
const PASS = env.MOCK_PASS ?? "correct-horse-battery";
const KEY = env.MOCK_KEY ?? "hd-test-key-4f9c2a7e1b";
// Each Profile has its own API key, never the default's; "coder" is listed but has none.
const PROFILES = ["default", "orchestrator", "coder"];
const PROFILE_KEYS = { default: KEY, orchestrator: env.MOCK_ORCH_KEY ?? "hd-orch-key-7d3e9a1c5b" };
const STATE_FILE = env.MOCK_STATE ?? "/tmp/hermes-mock-state.json";
const KEEPALIVE_MS = 10_000;
const TERMINAL = new Set(["run.completed", "run.failed", "run.cancelled"]);

// Sessions and sign-in tokens persist across restarts (state.db and stateless HMAC tokens do);
// Runs live in memory only, exactly like the real API server.
const state = existsSync(STATE_FILE)
  ? JSON.parse(readFileSync(STATE_FILE, "utf8"))
  : { sessions: {}, messages: {}, access: {}, refresh: {} };
const save = () => writeFileSync(STATE_FILE, JSON.stringify(state, null, 1));
const runs = new Map();

const newId = (prefix) => prefix + randomBytes(8).toString("hex");
const now = () => Date.now() / 1000;

function send(res, status, body, headers = {}) {
  res.writeHead(status, { "content-type": "application/json", ...headers });
  res.end(JSON.stringify(body));
}

async function readJson(req) {
  let raw = "";
  for await (const chunk of req) raw += chunk;
  try {
    return JSON.parse(raw || "{}");
  } catch {
    return null;
  }
}

function serve(port, name, handler) {
  http
    .createServer((req, res) => {
      res.on("finish", () => console.log(`[mock:${name}] ${req.method} ${req.url.split("?")[0]} -> ${res.statusCode}`));
      handler(req, res, new URL(req.url, "http://mock")).catch((e) => {
        console.error(e);
        send(res, 500, { detail: String(e) });
      });
    })
    .listen(port, "127.0.0.1", () => console.log(`[mock:${name}] listening on http://127.0.0.1:${port}`));
}

// ---- Dashboard (:9119 in production) ----

function cookieJar(req) {
  return Object.fromEntries(
    (req.headers.cookie ?? "").split(/;\s*/).filter(Boolean).map((c) => [c.slice(0, c.indexOf("=")), c.slice(c.indexOf("=") + 1)]),
  );
}

function issueSession() {
  const at = newId("at_");
  const rt = newId("rt_");
  state.access[at] = Date.now() + Number(env.MOCK_AT_TTL_MS ?? 12 * 3600_000);
  state.refresh[rt] = true;
  save();
  return [
    `hermes_session_at=${at}; HttpOnly; SameSite=Lax; Path=/`,
    `hermes_session_rt=${rt}; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`,
    "hermes_session_provider=basic; SameSite=Lax; Path=/",
  ];
}

const me = () => ({ user_id: USER, email: "", display_name: USER, org_id: "", provider: "basic", expires_at: null });

serve(DASH_PORT, "dashboard", async (req, res, url) => {
  const route = `${req.method} ${url.pathname}`;
  if (route === "GET /api/status") {
    return send(res, 200, {
      version: "mock",
      gateway_running: true,
      auth_required: env.MOCK_AUTH !== "off",
      auth_providers: ["basic"],
      auth_flows: ["cookie"],
    });
  }
  if (route === "POST /auth/password-login") {
    const body = await readJson(req);
    if (!body) return send(res, 422, { detail: "Invalid JSON body" });
    if (body.provider !== "basic") return send(res, 404, { detail: "Unknown provider" });
    if (body.username !== USER || body.password !== PASS) return send(res, 401, { detail: "Invalid credentials" });
    return send(res, 200, { ok: true, next: "/" }, { "set-cookie": issueSession() });
  }
  if (route === "GET /api/auth/me") {
    const jar = cookieJar(req);
    if (state.access[jar.hermes_session_at] > Date.now()) return send(res, 200, me());
    if (state.refresh[jar.hermes_session_rt]) {
      // Expired access token: rotate both; the old refresh token is now dead (reuse detection).
      delete state.refresh[jar.hermes_session_rt];
      delete state.access[jar.hermes_session_at];
      return send(res, 200, me(), { "set-cookie": issueSession() });
    }
    return send(
      res,
      401,
      { error: "unauthenticated", detail: "Unauthorized", reason: "no valid session", login_url: "/login" },
      { "set-cookie": ["hermes_session_at=; Max-Age=0; Path=/", "hermes_session_rt=; Max-Age=0; Path=/"] },
    );
  }
  const signedIn = env.MOCK_AUTH === "off" || state.access[cookieJar(req).hermes_session_at] > Date.now();
  if (url.pathname.startsWith("/api/") && !signedIn) return send(res, 401, { detail: "Unauthorized" });
  if (route === "GET /api/profiles") {
    return send(res, 200, { profiles: PROFILES.map((name) => ({ name, is_default: name === "default" })) });
  }
  if (route === "GET /api/profiles/active") return send(res, 200, { active: "orchestrator", current: "default" });
  if (route === "POST /api/audio/transcribe") {
    const body = await readJson(req);
    if (!body?.data_url?.startsWith("data:audio/") && !body?.data_url?.startsWith("data:video/webm")) {
      return send(res, 400, { detail: "Payload must be an audio recording" });
    }
    await new Promise((resolve) => setTimeout(resolve, Number(env.MOCK_TRANSCRIBE_MS ?? 800)));
    const profileName = url.searchParams.get("profile") ?? "default";
    return send(res, 200, { ok: true, transcript: `hello from the mock microphone (${profileName})`, provider: "mock" });
  }
  if (url.pathname.startsWith("/api/plugins/kanban/") || url.pathname.startsWith("/api/skills")) {
    return work(req, res, url);
  }
  send(res, 404, { detail: "Not Found" });
});

// Board and Skills, shaped like plugins/kanban/dashboard/plugin_api.py and web_routers/skills.py.
// In memory only; "MOCK_KANBAN=off" answers 404 like a disabled Kanban plugin.
const task = (id, status, title, extra = {}) =>
  ({ id, title, body: `Body of ${title}.`, assignee: null, status, priority: 0, tenant: null, created_at: Math.floor(now()) - 3600, ...extra });
const board = [
  task("t_2c64562e", "todo", "Register cerberus in orchestrator SOUL.md roster", { assignee: "daedalus", priority: 5 }),
  task("t_91bc558f", "ready", "Dotfiles: center Wi-Fi share QR code", { assignee: "backend-developer" }),
  task("t_922275a1", "blocked", "Build and validate cerberus specialist profile", { assignee: "hephaestus", priority: 10, tenant: "homelab" }),
  task("t_53d2c4f2", "done", "Review Phase 2 Terraform architecture", { assignee: "code-reviewer", priority: 90 }),
  task("t_0ld0ld00", "archived", "An archived task"),
];
const comments = { t_922275a1: [{ id: 1, task_id: "t_922275a1", author: "dashboard", body: "Waiting on the profile build.", created_at: Math.floor(now()) - 600 }] };
const skills = [
  { name: "unslop", description: "Cut AI tells from any writing.", category: null },
  { name: "grill-me", description: "Interview before building medium/larger projects.", category: null },
  { name: "codex", description: "Delegate coding to OpenAI Codex.", category: "autonomous-ai-agents" },
  { name: "hermes-agent", description: "Use, configure, and extend Hermes.", category: "autonomous-ai-agents" },
  { name: "ad-cs-certificate-request", description: "Request AD CS certificates.", category: "devops" },
];
// Each Profile's own skills.disabled; no `profile` is the Dashboard's own (default).
const disabled = { default: new Set(["codex"]), orchestrator: new Set(["unslop", "ad-cs-certificate-request"]), coder: new Set() };

async function work(req, res, url) {
  const path = url.pathname.replace("/api/plugins/kanban", "kanban");
  const route = `${req.method} ${path.replace(/t_[0-9a-z]+/, ":id")}`;
  const id = path.match(/t_[0-9a-z]+/)?.[0];
  if (path.startsWith("kanban") && env.MOCK_KANBAN === "off") return send(res, 404, { detail: "Not Found" });
  const off = disabled[url.searchParams.get("profile") ?? "default"];
  if (path.startsWith("/api/skills") && !off) return send(res, 404, { detail: "Profile does not exist." });
  switch (route) {
    case "GET kanban/board": {
      const archived = url.searchParams.get("include_archived") === "true";
      const names = ["triage", "todo", "scheduled", "ready", "running", "blocked", "review", "done", ...(archived ? ["archived"] : [])];
      return send(res, 200, { columns: names.map((name) => ({ name, tasks: board.filter((t) => t.status === name) })), tenants: [], assignees: [] });
    }
    case "GET kanban/assignees":
      return send(res, 200, { assignees: ["daedalus", "forge", "hephaestus"].map((name) => ({ name, on_disk: true, counts: {} })) });
    case "GET kanban/tasks/:id": {
      const found = board.find((t) => t.id === id);
      return found ? send(res, 200, { task: found, comments: comments[id] ?? [], events: [], runs: [] }) : send(res, 404, { detail: `task ${id} not found` });
    }
    case "POST kanban/tasks": {
      const body = await readJson(req);
      if (!body?.title) return send(res, 422, { detail: "title is required" });
      const created = task(`t_${randomBytes(4).toString("hex")}`, "todo", body.title, { body: body.body ?? null, assignee: body.assignee ?? null, priority: body.priority ?? 0, tenant: body.tenant ?? null });
      board.push(created);
      return send(res, 200, { task: created });
    }
    case "POST kanban/tasks/:id/comments": {
      const body = await readJson(req);
      if (!body?.body?.trim()) return send(res, 400, { detail: "body is required" });
      (comments[id] ??= []).push({ id: Date.now(), task_id: id, author: "dashboard", body: body.body, created_at: Math.floor(now()) });
      return send(res, 200, { ok: true });
    }
    case "POST kanban/dispatch": {
      const ready = board.filter((t) => t.status === "ready" && t.assignee);
      if (url.searchParams.get("dry_run") !== "true") for (const t of ready) t.status = "running";
      return send(res, 200, { reclaimed: 0, promoted: 0, spawned: ready.map((t) => [t.id, t.assignee, "/tmp/ws"]), skipped_unassigned: [], skipped_locked: false });
    }
    case "GET /api/skills":
      return send(res, 200, skills.map((s) => ({ ...s, enabled: !off.has(s.name) })));
    case "GET /api/skills/content": {
      const found = skills.find((s) => s.name === url.searchParams.get("name"));
      return found ? send(res, 200, { name: found.name, content: `---\nname: ${found.name}\ndescription: ${found.description}\n---\n\n# ${found.name}\n\nMock SKILL.md.\n`, path: "/mock" })
        : send(res, 404, { detail: "Skill not found." });
    }
    case "PUT /api/skills/toggle": {
      const body = await readJson(req);
      const found = skills.find((s) => s.name === body?.name);
      if (!found) return send(res, 404, { detail: "Skill not found." });
      const enabled = body.name === "hermes-agent" || !!body.enabled;
      if (enabled) off.delete(found.name);
      else off.add(found.name);
      return send(res, 200, { ok: true, name: found.name, enabled });
    }
  }
  send(res, 404, { detail: "Not Found" });
}

// ---- API server (:8642 in production) ----

const unauthorized = { error: { message: "Invalid gateway API key (API_SERVER_KEY)", type: "gateway_auth_error", code: "gateway_auth_failed" } };
const notFound = (what) => ({ error: { message: `${what} not found`, type: "invalid_request_error", code: "not_found" } });

function touchSession(id, preview, profile = "default") {
  state.sessions[id] ??= {
    id, source: "api_server", profile, title: null, model: "mock", started_at: now(), ended_at: null,
    message_count: 0, tool_call_count: 0, parent_session_id: null, preview: null, pinned: false, archived: false,
  };
  state.messages[id] ??= [];
  const session = state.sessions[id];
  session.last_active = now();
  session.preview ??= preview;
  return session;
}

function addMessage(sessionId, role, content) {
  const list = state.messages[sessionId];
  if (!list) return; // deleted while the Run was going
  list.push({ id: list.length + 1, session_id: sessionId, role, content, tool_call_id: null, tool_calls: null, timestamp: now() });
  state.sessions[sessionId].message_count = list.length;
  state.sessions[sessionId].last_active = now();
  save();
}

function describeInput(input) {
  const content = typeof input === "string" ? input : input.at(-1)?.content;
  const parts = typeof content === "string" ? [{ type: "text", text: content }] : Array.isArray(content) ? content : [];
  const text = parts.filter((p) => p.type === "text").map((p) => p.text).join("\n");
  const images = parts.filter((p) => p.type === "image_url").length;
  const names = [...text.matchAll(/Attached file `([^`]+)`/g)].map((m) => m[1]);
  for (let i = 1; i <= images; i++) names.push(`image ${i}`);
  return { content, text, names };
}

function startRun(runId, sessionId, prompt, attachments, via) {
  const run = { runId, sessionId, status: "running", events: [], subscribers: new Set(), stop: false, wake: null, createdAt: now() };
  runs.set(runId, run);
  const emit = (event, fields = {}) => {
    const payload = { event, run_id: runId, timestamp: now(), ...fields, seq: run.events.length + 1 };
    run.events.push(payload);
    if (TERMINAL.has(event)) run.status = event.slice(4);
    for (const s of run.subscribers) s.push(payload);
  };
  const sleep = (ms) =>
    new Promise((resolve) => {
      const timer = setTimeout(resolve, ms);
      run.wake = () => (clearTimeout(timer), resolve());
    });
  const tokenMs = /slow/i.test(prompt) ? 350 : 90;
  let reply = "";
  const say = async (text) => {
    for (const token of text.match(/\S+\s*/g) ?? []) {
      if (run.stop) return;
      emit("message.delta", { delta: token });
      reply += token;
      await sleep(tokenMs);
    }
  };
  (async () => {
    await say(`[${via}] You said: "${prompt.split("\n")[0].slice(0, 80)}". `);
    if (attachments.length) await say(`I received ${attachments.length} attachment(s): ${attachments.join(", ")}. `);
    if (/tool/i.test(prompt) && !run.stop) {
      emit("tool.started", { tool: "terminal", preview: "ls -la ~/projects" });
      await sleep(1500);
      if (!run.stop) emit("tool.completed", { tool: "terminal", duration: 1.5, error: false, preview: "total 3" });
    }
    if (/approval/i.test(prompt) && !run.stop) {
      emit("approval.request", {
        command: "rm -rf ./build", description: "recursive delete of ./build", pattern_key: "rm_rf",
        request_id: "apr_1", choices: ["once", "session", "always", "deny"], allow_permanent: true, allow_session: true,
      });
      await sleep(5000);
      if (!run.stop) emit("approval.responded", { choice: "deny", request_id: "apr_1", resolved: true });
    }
    if (/idle/i.test(prompt)) {
      await say("Pausing for 12 seconds now. ");
      if (!run.stop) await sleep(12_000);
    }
    if (/fail/i.test(prompt) && !run.stop) {
      return emit("run.failed", { completed: false, partial: true, interrupted: false, error: "Mock provider error (HTTP 529)" });
    }
    await say(
      "Here is a streamed reply so you can watch tokens arrive one at a time. It has `inline code` and a block:\n```\necho hello\n```\nDone." +
        (/slow/i.test(prompt) ? " And a few more words to keep this stream going.".repeat(5) : ""),
    );
    if (run.stop) return emit("run.cancelled", { completed: false, partial: true, interrupted: true });
    addMessage(sessionId, "assistant", reply);
    emit("run.completed", {
      completed: true, partial: false, interrupted: false, output: reply,
      usage: { input_tokens: 12, output_tokens: reply.split(" ").length, total_tokens: 12 + reply.split(" ").length },
    });
  })();
}

function streamEvents(res, run, lastSeq) {
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
  let lastWrite = Date.now();
  const write = (text) => {
    res.write(text);
    lastWrite = Date.now();
  };
  const close = () => {
    write(": stream closed\n\n");
    res.end();
  };
  const subscriber = {
    push: (payload) => {
      write(`id: ${payload.seq}\ndata: ${JSON.stringify(payload)}\n\n`);
      if (TERMINAL.has(payload.event)) close();
    },
  };
  write(": open\n\n");
  const keepalive = setInterval(() => {
    if (Date.now() - lastWrite < KEEPALIVE_MS) return;
    console.log(`[mock:api] : keepalive -> ${run.runId}`);
    write(": keepalive\n\n");
  }, 250);
  res.on("close", () => {
    clearInterval(keepalive);
    run.subscribers.delete(subscriber);
  });
  for (const payload of run.events) if (payload.seq > lastSeq) subscriber.push(payload);
  if (run.status === "running") run.subscribers.add(subscriber);
  else if (!res.writableEnded) close();
}

serve(API_PORT, "api", async (req, res, url) => {
  if (req.headers.origin) return res.writeHead(403).end(); // non-allowlisted Origin: bare 403, like the real server
  if (Number(req.headers["content-length"]) > 10_000_000) { // MAX_REQUEST_BYTES
    return send(res, 413, { error: { message: "Request body too large.", type: "invalid_request_error", param: null, code: "body_too_large" } });
  }
  // /p/<profile>/... addresses a named Profile, like a multiplexing gateway.
  let profile = "default";
  const prefixed = url.pathname.match(/^\/p\/([^/]+)(\/.*)$/);
  if (prefixed) {
    profile = decodeURIComponent(prefixed[1]);
    if (!PROFILES.includes(profile)) return send(res, 404, { error: "Unknown or unconfigured profile" });
    url.pathname = prefixed[2];
  }
  if (!PROFILE_KEYS[profile] || req.headers.authorization !== `Bearer ${PROFILE_KEYS[profile]}`) return send(res, 401, unauthorized);
  const [, a, b, id, sub] = url.pathname.split("/").map(decodeURIComponent);
  const route = `${req.method} /${a}/${b}${id ? "/:id" : ""}${sub ? `/${sub}` : ""}`;

  if (route === "GET /v1/capabilities") {
    const runsOn = env.MOCK_NO_RUNS !== "1";
    return send(res, 200, {
      object: "hermes.api_server.capabilities", platform: "hermes-agent", model: "mock",
      auth: { type: "bearer", required: true },
      features: {
        run_submission: runsOn, run_events_sse: runsOn, run_status: true, run_stop: true,
        tool_progress_events: true, approval_events: true, session_resources: true,
      },
      endpoints: {
        runs: { method: "POST", path: "/v1/runs" },
        run_events: { method: "GET", path: "/v1/runs/{run_id}/events" },
        run_stop: { method: "POST", path: "/v1/runs/{run_id}/stop" },
        sessions: { method: "GET", path: "/api/sessions" },
      },
    });
  }
  if (route === "POST /v1/runs") {
    const body = await readJson(req);
    if (!body || !(typeof body.input === "string" || Array.isArray(body.input))) {
      return send(res, 400, { error: { message: "input is required", type: "invalid_request_error", code: "missing_input" } });
    }
    const runId = newId("run_");
    const sessionId = body.session_id ?? runId;
    const { content, text, names } = describeInput(body.input);
    touchSession(sessionId, text.slice(0, 60), profile);
    addMessage(sessionId, "user", content);
    const model = body.model ? `${body.provider ?? "?"}/${body.model}` : "profile default";
    startRun(runId, sessionId, text, names, `${profile} · ${model} · reasoning ${body.model_options?.reasoning_effort ?? "default"}`);
    return send(res, 202, { run_id: runId, status: "started", replayed: false });
  }
  if (a === "v1" && b === "runs" && id) {
    const run = runs.get(id);
    if (!run) return send(res, 404, notFound(`Run ${id}`));
    if (route === "GET /v1/runs/:id") {
      return send(res, 200, { object: "hermes.run", run_id: id, status: run.status, session_id: run.sessionId, created_at: run.createdAt });
    }
    if (route === "GET /v1/runs/:id/events") {
      const lastSeq = Number(req.headers["last-event-id"] ?? url.searchParams.get("last_seq") ?? -1);
      return streamEvents(res, run, Number.isFinite(lastSeq) ? lastSeq : -1);
    }
    if (route === "POST /v1/runs/:id/stop") {
      run.stop = true;
      run.wake?.();
      return send(res, 200, { run_id: id, status: "stopping" });
    }
  }
  if (req.method === "GET" && url.pathname === "/api/model/options") {
    if (env.MOCK_NO_MODELS === "1") return send(res, 500, { error: { message: "Failed to list model options.", code: "model_options_failed" } });
    return send(res, 200, { provider: "mockai", model: "mock-large", providers: [
      { slug: "mockai", name: "Mock AI", authenticated: true, models: ["mock-large", "mock-small"] },
      { slug: "zai", name: "Z.ai", authenticated: true, models: ["glm-5.3-flash"] },
      { slug: "nous", name: "Nous Portal", authenticated: false, models: ["hermes-5"] },
    ] });
  }
  if (route === "GET /api/sessions") {
    const limit = Math.min(Number(url.searchParams.get("limit") ?? 50), 200);
    const all = Object.values(state.sessions).filter((s) => (s.profile ?? "default") === profile).sort((x, y) => y.last_active - x.last_active);
    // Like include_pinned=True: pinned Sessions past the limit are back-filled after the page.
    const data = [...all.slice(0, limit), ...all.slice(limit).filter((s) => s.pinned)];
    return send(res, 200, { object: "list", data, limit, offset: 0, has_more: all.length > limit });
  }
  if (route === "POST /api/sessions") {
    const body = (await readJson(req)) ?? {};
    const sid = body.id ?? newId("sess_");
    if (state.sessions[sid]) return send(res, 409, { error: { message: "Session already exists", code: "session_exists" } });
    const session = touchSession(sid, null, profile);
    session.title = body.title ?? null;
    save();
    return send(res, 201, { object: "hermes.session", session });
  }
  if (a === "api" && b === "sessions" && id) {
    const session = state.sessions[id];
    if (!session) return send(res, 404, notFound(`Session ${id}`));
    if (route === "GET /api/sessions/:id") return send(res, 200, { object: "hermes.session", session });
    if (route === "PATCH /api/sessions/:id") {
      const body = (await readJson(req)) ?? {};
      if ("pinned" in body && typeof body.pinned !== "boolean") return send(res, 400, { error: { message: "'pinned' must be a boolean", code: "invalid_session_field" } });
      if ("title" in body) session.title = body.title;
      if ("pinned" in body) session.pinned = body.pinned;
      save();
      return send(res, 200, { object: "hermes.session", session });
    }
    if (route === "DELETE /api/sessions/:id") {
      delete state.sessions[id];
      delete state.messages[id];
      save();
      return send(res, 200, { object: "hermes.session.deleted", id, deleted: true });
    }
    if (route === "GET /api/sessions/:id/messages") {
      const data = state.messages[id];
      return send(res, 200, { object: "list", session_id: id, data, pagination: { limit: 500, offset: 0, order: "latest", returned: data.length } });
    }
  }
  send(res, 404, notFound(url.pathname));
});
