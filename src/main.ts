import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { field, h, icon, input } from "./dom";
import { appearanceTab, applyAppearance } from "./appearance";
import {
  changeLabel, choiceKey, DEFAULT_CHOICE, loadChoice, REASONING_LEVELS, saveChoice, saveProfile, startProfile,
  type Choice, type ModelChoice,
} from "./prefs";

type GatewayError = { kind: "unreachable" | "unauthorized" | "not_found" | "http" | "invalid"; message: string };
type Init = { dashboard_url: string | null; api_url: string | null; ssh_host: string | null; keyring: boolean };
type Status = { auth_required: boolean; auth_providers: string[] };
type RunStarted = { run_id: string; session_id: string };
type RunEvent = { event: string; seq?: number; [field: string]: unknown };
type StreamMsg = { type: "event"; data: RunEvent } | { type: "dropped"; message: string };
type Session = { id: string; title?: string | null; preview?: string | null };
type Message = { role: string; content: unknown; tool_calls?: { function?: { name?: string } }[] | null };
type Attachment = { path: string } | { name: string; data_url: string };
type Models = { default: ModelChoice | null; groups: { provider: string; name: string; models: string[] }[] };
type TurnStatus = "streaming" | "done" | "stopped" | "failed" | "not-sent";

interface Turn {
  text: string;
  files: Attachment[];
  runId?: string;
  lastSeq: number;
  reply: string;
  status: TurnStatus;
  root: HTMLElement;
  tools: HTMLElement;
  replyEl: HTMLElement;
  notice: HTMLElement;
  statusEl: HTMLElement;
  choice: Choice;
}

applyAppearance();
const app = document.querySelector<HTMLDivElement>("#app")!;
const banner = document.querySelector<HTMLDivElement>("#banner")!;
const dialog = document.querySelector<HTMLDialogElement>("#reauth")!;
dialog.addEventListener("cancel", (e) => e.preventDefault()); // re-auth can't be dismissed, only completed

const asError = (e: unknown): GatewayError =>
  typeof e === "object" && e !== null && "kind" in e ? (e as GatewayError)
  : { kind: "invalid", message: e instanceof Error ? e.message : String(e) };

const basename = (path: string) => path.split(/[\\/]/).pop() ?? path;
const attachmentName = (a: Attachment) => ("path" in a ? basename(a.path) : a.name);

function noteSaved(saved: boolean) {
  if (!saved) {
    banner.hidden = false;
    banner.textContent =
      "Keyring unavailable: credentials are kept in memory only and will not be saved. You will need to sign in again next launch.";
  }
}

// ---- Startup, connect screen, pairing, re-auth ----

async function boot() {
  app.replaceChildren(h("p", { className: "loading", textContent: "Opening the OS keyring… (unlock it if your system asks)" }));
  const init = await invoke<Init>("init");
  noteSaved(init.keyring);
  connection = init.ssh_host ? `ssh:${init.ssh_host}` : init.dashboard_url && init.api_url ? connectionId(init.dashboard_url, init.api_url) : "";
  if (init.ssh_host) await connectSsh();
  else if (init.dashboard_url && init.api_url) await connect();
  else showPairing();
}

async function connect() {
  app.replaceChildren(h("p", { className: "loading", textContent: "Connecting to the gateway…" }));
  try {
    const status = await invoke<Status>("status");
    signInRequired = status.auth_required;
    if (status.auth_required) {
      try {
        await invoke("check_sign_in");
      } catch (e) {
        if (asError(e).kind !== "unauthorized") throw e;
        await reauth("sign-in", "Your sign-in has expired or was rejected by the dashboard.");
      }
    }
    let supported: boolean;
    try {
      supported = await invoke<boolean>("capabilities");
    } catch (e) {
      if (asError(e).kind !== "unauthorized") throw e;
      supported = (await reauth("key", "The API server did not accept the saved API key.")) ?? false;
    }
    showChat(supported);
  } catch (e) {
    showConnect(asError(e));
  }
}

async function connectSsh() {
  app.replaceChildren(h("p", { className: "loading", textContent: "Starting Hermes over SSH…" }));
  try {
    await invoke("connect_ssh");
    showChat(true, true);
  } catch (e) {
    showConnect(asError(e), connectSsh);
  }
}

function showConnect(error: GatewayError, retry = connect) {
  app.replaceChildren(
    h(
      "section",
      { className: "card" },
      h("h1", { textContent: "Can't reach Hermes" }),
      h("p", { className: "error", textContent: error.message }),
      h("div", { className: "row" }, h("button", { textContent: "Retry", onclick: retry }),
        h("button", { className: "secondary", textContent: "Change gateway", onclick: () => showPairing() })),
    ),
  );
}

function showPairing() {
  const dashUrl = input({ placeholder: "http://hermes.lan:9119" });
  const apiUrl = input({ placeholder: "http://hermes.lan:8642" });
  let apiEdited = false;
  apiUrl.oninput = () => (apiEdited = true);
  const probeResult = h("p");
  const step2 = h("div", { hidden: true });
  const check = h("button", { type: "button", textContent: "Check gateway" });
  dashUrl.oninput = () => {
    step2.hidden = true;
    try {
      const u = new URL(dashUrl.value);
      if (!apiEdited) apiUrl.value = `${u.protocol}//${u.hostname}:8642`;
    } catch {
      /* not a URL yet */
    }
  };
  apiUrl.addEventListener("input", () => (step2.hidden = true));

  check.onclick = async () => {
    probeResult.className = "";
    probeResult.textContent = "Checking…";
    try {
      await invoke("configure", { dashboardUrl: dashUrl.value, apiUrl: apiUrl.value });
      showStep2(await invoke<Status>("status"));
    } catch (e) {
      probeResult.className = "error";
      probeResult.textContent = asError(e).message;
    }
  };

  function showStep2(status: Status) {
    const providers = status.auth_providers.join(", ") || "none listed";
    probeResult.textContent = status.auth_required
      ? `Gateway reachable. Sign-in required. Providers: ${providers}.`
      : "Gateway reachable. No sign-in required.";
    const user = input({ autocomplete: "username" });
    const pass = input({ type: "password" });
    const key = input({ type: "password" });
    const authError = h("p", { className: "error" });
    const keyError = h("p", { className: "error" });
    const formError = h("p", { className: "error" });
    const pair = h("button", { textContent: "Pair" });
    const needsBasic = status.auth_required;
    const basicOffered = status.auth_providers.includes("basic");
    const form = h(
      "form",
      {},
      ...(needsBasic && basicOffered ? [field("Username", user), field("Password", pass), authError] : []),
      ...(needsBasic && !basicOffered
        ? [h("p", { className: "error", textContent: `This client supports only the "basic" provider; the gateway offers: ${providers}.` })]
        : []),
      field("API server key (API_SERVER_KEY)", key),
      keyError,
      formError,
      pair,
    );
    pair.disabled = needsBasic && !basicOffered;
    form.onsubmit = async (e) => {
      e.preventDefault();
      for (const p of [authError, keyError, formError]) p.textContent = "";
      pair.disabled = true;
      try {
        if (needsBasic) {
          try {
            noteSaved(await invoke<boolean>("sign_in", { username: user.value, password: pass.value }));
            await invoke("check_sign_in");
          } catch (e) {
            if (asError(e).kind !== "unauthorized") throw e;
            authError.textContent = "Wrong username or password.";
            return;
          }
        }
        let accepted: { runs: boolean; saved: boolean };
        try {
          accepted = await invoke("set_api_key", { key: key.value });
        } catch (e) {
          if (asError(e).kind !== "unauthorized") throw e;
          keyError.textContent = "The API server rejected this key.";
          return;
        }
        noteSaved(accepted.saved);
        await invoke("finish_pairing");
        connection = connectionId(dashUrl.value, apiUrl.value);
        showChat(accepted.runs);
      } catch (e) {
        formError.textContent = asError(e).message;
      } finally {
        pair.disabled = false;
      }
    };
    step2.replaceChildren(form);
    step2.hidden = false;
  }

  const sshHost = input({ placeholder: "liam@hermes.lan" });
  const sshResult = h("p");
  const sshConnect = h("button", { textContent: "Connect over SSH" });
  const sshForm = h(
    "form",
    {},
    h("h2", { textContent: "Or run Hermes over SSH" }),
    h("p", { textContent: "Runs hermes acp on that host. Uses your SSH keys or ssh-agent; password prompts aren't supported. Set ports and usernames in ~/.ssh/config if you need to." }),
    field("SSH host", sshHost),
    sshConnect,
    sshResult,
  );
  sshForm.onsubmit = async (e) => {
    e.preventDefault();
    sshConnect.disabled = true;
    sshResult.className = "";
    sshResult.textContent = "Connecting…";
    try {
      await invoke("configure_ssh", { host: sshHost.value });
      connection = `ssh:${sshHost.value.trim()}`;
      showChat(true, true);
    } catch (e) {
      sshResult.className = "error";
      sshResult.textContent = asError(e).message;
    } finally {
      sshConnect.disabled = false;
    }
  };

  app.replaceChildren(
    h(
      "section",
      { className: "card" },
      h("h1", { textContent: "Pair with your Hermes gateway" }),
      field("Dashboard URL", dashUrl),
      field("API server URL", apiUrl),
      check,
      probeResult,
      step2,
      sshForm,
    ),
  );
}

/** Blocks until the rejected credential is replaced. For a new key, resolves with whether chat is supported;
 *  with `cancel`, the prompt can also be left, resolving `null`. */
function reauth(kind: "sign-in" | "key", reason: string, cancel?: string): Promise<boolean | undefined | null> {
  return new Promise((resolve) => {
    const user = input({ autocomplete: "username" });
    const pass = input({ type: "password" });
    const key = input({ type: "password" });
    const error = h("p", { className: "error" });
    const submit = h("button", { textContent: kind === "sign-in" ? "Sign in" : "Save key" });
    const form = h(
      "form",
      {},
      h("h2", { textContent: kind === "sign-in" ? "Sign in again" : "Enter a new API key" }),
      h("p", { textContent: reason }),
      ...(kind === "sign-in" ? [field("Username", user), field("Password", pass)] : [field("API server key", key)]),
      error,
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
    );
    form.onsubmit = async (e) => {
      e.preventDefault();
      error.textContent = "";
      submit.disabled = true;
      try {
        let accepted: boolean | undefined;
        if (kind === "sign-in") {
          noteSaved(await invoke<boolean>("sign_in", { username: user.value, password: pass.value }));
          await invoke("check_sign_in");
        } else {
          const result = await invoke<{ runs: boolean; saved: boolean }>("set_api_key", { key: key.value });
          noteSaved(result.saved);
          accepted = result.runs;
        }
        dialog.close();
        resolve(accepted);
      } catch (e) {
        const x = asError(e);
        error.textContent =
          x.kind !== "unauthorized" ? x.message
          : kind === "sign-in" ? "Wrong username or password."
          : "The API server rejected this key.";
      } finally {
        submit.disabled = false;
      }
    };
    dialog.replaceChildren(form);
    dialog.showModal();
  });
}

// ---- Chat ----

let runs = false; // the API server accepts Runs and streams their events
let connection = ""; // which Gateway or SSH host; Profiles and per-Session choices are remembered per connection
let profile = "default";
const connectionId = (dash: string, api: string) => `${new URL(dash).origin}|${new URL(api).origin}`;
let signInRequired = true; // the Dashboard asks for Sign-in; without it, a 401 can't be fixed by signing in
let overSsh = false; // Hermes over SSH (hermes acp): Approvals can be answered, Sessions can't be deleted
let sessionId: string | null = null;
let active: Turn | null = null;
let pending: Attachment[] = [];
let models: Models = { default: null, groups: [] };
let choice: Choice = DEFAULT_CHOICE; // what the next Turn in this view runs on
let lastChoice: Choice | undefined; // what the previous Turn ran on, for the reply label
let ui: {
  sessions: HTMLUListElement;
  sessionsError: HTMLElement;
  turns: HTMLElement;
  pending: HTMLElement;
  input: HTMLTextAreaElement;
  send: HTMLButtonElement;
  drop: HTMLElement;
  composerError: HTMLElement;
  toolbar: HTMLElement;
  spacer: HTMLElement;
  profile: HTMLSelectElement;
  modelPicker: HTMLElement;
  modelLabel: HTMLElement;
  menu: HTMLElement;
  reasoning: HTMLSelectElement;
  mic: HTMLButtonElement;
  paperclip: HTMLButtonElement;
  micTime: HTMLElement;
} | null = null;

function showChat(supported: boolean, ssh = false) {
  runs = supported;
  overSsh = ssh;
  const unavailable = "Chat unavailable: this API server does not advertise run submission with event streaming.";
  ui = {
    sessions: h("ul", { className: "sessions" }),
    sessionsError: h("p", { className: "error" }),
    turns: h("div", { className: "turns" }),
    pending: h("div", { className: "pending" }),
    input: h("textarea", { placeholder: runs ? "Message Hermes. Drop, paste, or pick attachments." : unavailable, rows: 3 }),
    send: h("button", { textContent: "Send" }),
    drop: h("div", { className: "drop", hidden: true, textContent: runs ? "Drop to attach" : unavailable }),
    composerError: h("p", { className: "error" }),
    toolbar: h("div", { className: "toolbar" }),
    spacer: h("span", { className: "spacer" }),
    mic: h("button", { type: "button", className: "icon-btn", title: "Dictate" }),
    micTime: h("span"),
    paperclip: h("button", { type: "button", className: "icon-btn", title: "Add attachments" }, icon("paperclip")),
    profile: h("select", { title: "Profile" }),
    modelPicker: h("span", { className: "picker model" }),
    modelLabel: h("span"),
    menu: h("div", { className: "menu" }),
    reasoning: h("select", {}, h("option", { value: "", textContent: "Default" }),
      ...REASONING_LEVELS.map((r) => h("option", { value: r, textContent: r }))),
  };
  const { input: box, send: button } = ui;
  const picker = h("input", { type: "file", multiple: true, hidden: true });
  picker.onchange = async () => {
    await addFiles([...(picker.files ?? [])]);
    picker.value = "";
  };
  ui.paperclip.onclick = () => picker.click();
  ui.toolbar.append(ui.paperclip, picker, ui.spacer, button);
  ui.mic.append(icon("mic"), ui.micTime);
  ui.mic.onclick = () => void toggleDictation();
  // Speech-to-text is a Dashboard feature; over SSH there is no Dashboard.
  ui.mic.hidden = overSsh || !navigator.mediaDevices?.getUserMedia;
  ui.toolbar.insertBefore(ui.mic, picker.nextSibling);
  box.onpaste = (e) => void pasteImages(e);
  ui.profile.onchange = () => void useProfile(ui!.profile.value, profile);
  ui.toolbar.insertBefore(h("span", { className: "picker profile", title: "Profile" }, icon("user"), ui.profile), ui.spacer);
  ui.modelPicker.append(h("button", { type: "button", className: "icon-btn", title: "Model", onclick: toggleModelMenu }, icon("cpu"), ui.modelLabel, icon("chevron", 12)));
  const reasoningPicker = h("span", { className: "picker", title: "Reasoning level" }, icon("brain"), ui.reasoning);
  if (overSsh) {
    ui.reasoning.disabled = true;
    reasoningPicker.title = "Over SSH, the reasoning level is set in the profile's config";
  }
  ui.reasoning.onchange = () => setChoice({ ...choice, reasoning: ui!.reasoning.value || null });
  ui.toolbar.insertBefore(ui.modelPicker, ui.spacer);
  ui.toolbar.insertBefore(reasoningPicker, ui.spacer);
  box.onkeydown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!active) send();
    }
  };
  button.onclick = () => (active ? stop() : send());
  mountShell(
    h(
      "div",
      { className: "chat" },
      h("aside", {}, h("div", { className: "row" }, h("h2", { textContent: "Sessions" }),
        h("button", { className: "secondary", textContent: "New chat", onclick: newChat })), ui.sessionsError, ui.sessions,
        h("footer", {}, h("button", { className: "icon-btn", title: "Settings", onclick: showSettings }, icon("settings"), "Settings"))),
      h("main", {}, ui.turns, ui.pending, h("div", { className: "composer" }, h("div", { className: "composer-box" }, box, ui.toolbar))),
      ui.drop,
    ),
  );
  renderPending(); // puts the composer error line on screen
  newChat();
  updateComposer();
  void setupProfiles();
}

/** Settings takes the chat view's place in the shell until closed (or another rail view is picked); the chat keeps running underneath. */
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
  chat.after(view);
}

getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (!ui) return;
  ui.drop.hidden = payload.type === "drop" || payload.type === "leave";
  if (payload.type === "drop" && runs) {
    pending.push(...payload.paths.filter((p) => !pending.some((a) => "path" in a && a.path === p)).map((path) => ({ path })));
    renderPending();
  }
});

/** An Attachment chip: a thumbnail for images the webview holds, a paperclip otherwise. */
function chip(a: Attachment, onRemove?: () => void): HTMLElement {
  const thumb = "data_url" in a && a.data_url.startsWith("data:image/") ? h("img", { src: a.data_url, alt: "" }) : "📎";
  const remove = onRemove ? [h("button", { className: "link", textContent: "×", title: "Remove attachment", onclick: onRemove })] : [];
  return h("span", { className: "chip" }, thumb, attachmentName(a), ...remove);
}

function renderPending() {
  ui!.pending.replaceChildren(
    ...pending.map((a) => chip(a, () => {
      pending = pending.filter((p) => p !== a);
      renderPending();
    })),
    ui!.composerError,
  );
}

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
      ui!.composerError.textContent = `Not attached: ${asError(err).message}`;
    }
  }
  renderPending();
}

let recorder: MediaRecorder | null = null;
const clock = (ms: number) => `${Math.floor(ms / 60_000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;

/** Dictation: click to record, click again to insert the transcript at the cursor. */
async function toggleDictation() {
  if (recorder) return recorder.stop();
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  } catch (e) {
    ui!.composerError.textContent = `Microphone unavailable: ${asError(e).message}`;
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

function updateComposer() {
  ui!.send.textContent = active ? "Stop" : "Send";
  ui!.send.disabled = !runs || (active !== null && !active.runId);
  ui!.input.disabled = !runs;
  ui!.paperclip.disabled = !runs;
}

const FENCE = /```(?:[^\n`]*\n)?([\s\S]*?)```/;

/** Fenced code and `inline code` only; everything else is plain text. Never innerHTML. */
function renderText(el: HTMLElement, text: string) {
  el.replaceChildren(
    ...text.split(new RegExp(FENCE, "g")).flatMap((part, i) =>
      i % 2
        ? [h("pre", {}, h("code", { textContent: part }))]
        : part.split(/`([^`\n]+)`/).map((s, j) => (j % 2 ? h("code", { textContent: s }) : document.createTextNode(s))),
    ),
  );
}

function contentText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((part) => (part?.type === "text" ? String(part.text) : part?.type === "image_url" ? "[image]" : ""))
    .filter(Boolean)
    .join("\n");
}

function scrollToEnd() {
  ui!.turns.scrollTop = ui!.turns.scrollHeight;
}

function addTurn(text: string, files: Attachment[]): Turn {
  ui!.turns.querySelector(".empty")?.remove();
  const user = h("div", { className: "user" });
  renderText(user, text);
  if (files.length) user.append(h("div", { className: "files" }, ...files.map((f) => chip(f))));
  const turn: Turn = {
    text,
    files,
    lastSeq: -1,
    reply: "",
    status: "done",
    root: h("article", { className: "turn" }),
    tools: h("div", { className: "tools" }),
    replyEl: h("div", { className: "reply" }),
    notice: h("p", { className: "notice" }),
    statusEl: h("div", { className: "status" }),
    choice: DEFAULT_CHOICE,
  };
  turn.root.append(...(text || files.length ? [user] : []), turn.tools, turn.replyEl, turn.notice, turn.statusEl);
  ui!.turns.append(turn.root);
  scrollToEnd();
  return turn;
}

function appendReply(turn: Turn, text: string) {
  turn.reply += text;
  renderText(turn.replyEl, turn.reply.trimStart());
  scrollToEnd();
}

function addTool(turn: Turn, tool: string, preview: string, state: "running" | "done") {
  const chip = h("div", { className: "tool", textContent: `${state === "running" ? "⏳" : "✓"} ${tool}${preview ? ` · ${preview}` : ""}` });
  chip.dataset.tool = tool;
  chip.dataset.state = state;
  turn.tools.append(chip);
  scrollToEnd();
}

function finishTool(turn: Turn, tool: string, duration: unknown, failed: boolean) {
  const running = [...turn.tools.querySelectorAll<HTMLElement>('[data-state="running"]')].filter((c) => c.dataset.tool === tool);
  const chip = running.at(-1);
  if (!chip) return;
  chip.dataset.state = failed ? "failed" : "done";
  chip.textContent = `${failed ? "✗" : "✓"} ${tool}${typeof duration === "number" ? ` · ${duration.toFixed(1)} s` : ""}`;
}

function setStatus(turn: Turn, status: TurnStatus, message = "") {
  turn.status = status;
  turn.root.dataset.status = status;
  if (status !== "streaming") turn.notice.textContent = "";
  if (status === "streaming") turn.statusEl.replaceChildren(h("span", { className: "typing", textContent: "Hermes is working…" }));
  else if (status === "done") turn.statusEl.replaceChildren();
  else turn.statusEl.replaceChildren(h("span", { textContent: message }), h("button", { className: "secondary", textContent: "Retry", onclick: () => retry(turn) }));
  if (active === turn && status !== "streaming") active = null;
  updateComposer();
}

async function send() {
  const text = ui!.input.value.trim();
  if (!text && !pending.length) return;
  const turn = addTurn(text, pending);
  turn.choice = choice;
  const label = changeLabel(lastChoice, choice);
  lastChoice = choice;
  if (label) turn.replyEl.after(h("div", { className: "model-label", textContent: label }));
  pending = [];
  ui!.composerError.textContent = "";
  renderPending();
  ui!.input.value = "";
  await beginRun(turn);
}

/** Starts a fresh Run for the Turn in the open Session (or a new Session). */
async function beginRun(turn: Turn) {
  active = turn;
  turn.runId = undefined;
  turn.lastSeq = -1;
  turn.reply = "";
  turn.replyEl.replaceChildren();
  turn.tools.replaceChildren();
  setStatus(turn, "streaming");
  let run: RunStarted;
  try {
    run = await invoke<RunStarted>("start_run", {
      sessionId, text: turn.text, files: turn.files, model: turn.choice.model, reasoning: turn.choice.reasoning,
    });
  } catch (e) {
    const x = asError(e);
    setStatus(turn, "not-sent", `Not sent: ${x.message}`);
    if (x.kind === "unauthorized") setRuns(await reauth("key", "The API server rejected the saved API key."));
    return;
  }
  if (turn.status !== "streaming") {
    invoke("stop_run", { runId: run.run_id }); // the Turn was left while the Run was being created
    return;
  }
  turn.runId = run.run_id;
  sessionId = run.session_id;
  saveChoice(localStorage, choiceKey(connection, profile, run.session_id), choice);
  updateComposer();
  try {
    await openStream(turn);
  } catch (e) {
    setStatus(turn, "failed", `Connection lost: ${asError(e).message}`);
  }
}

function setRuns(next: boolean | undefined | null) {
  if (typeof next === "boolean") runs = next;
  updateComposer();
}

async function openStream(turn: Turn) {
  const channel = new Channel<StreamMsg>();
  channel.onmessage = (msg) => onStream(turn, msg);
  await invoke("stream_run", { runId: turn.runId, lastSeq: turn.lastSeq, onEvent: channel });
}

/** Reattach to the same Run where it left off; start a new Run only if the server lost it. */
async function retry(turn: Turn) {
  if (active) return;
  if (turn.status === "failed" && turn.runId) {
    active = turn;
    setStatus(turn, "streaming");
    try {
      await openStream(turn);
      return;
    } catch (e) {
      const x = asError(e);
      if (x.kind !== "not_found") return setStatus(turn, "failed", `Connection lost: ${x.message}`);
    }
  }
  // ponytail: a Run that finished and was reaped while we were disconnected is re-run here.
  await beginRun(turn);
}

function onStream(turn: Turn, msg: StreamMsg) {
  if (turn.status !== "streaming") return;
  if (msg.type === "dropped") return setStatus(turn, "failed", `Connection lost: ${msg.message}`);
  const ev = msg.data;
  if (typeof ev.seq === "number") turn.lastSeq = ev.seq;
  switch (ev.event) {
    case "message.delta":
      return appendReply(turn, String(ev.delta ?? ""));
    case "message.interim":
      if (!ev.already_streamed && ev.text) appendReply(turn, (turn.reply ? "\n\n" : "") + String(ev.text ?? ""));
      return;
    case "tool.started":
      return addTool(turn, String(ev.tool), String(ev.preview ?? ""), "running");
    case "tool.completed":
      return finishTool(turn, String(ev.tool), ev.duration, ev.error === true);
    case "approval.request":
      if (Array.isArray(ev.options)) {
        const options = ev.options as { id: string; name: string }[];
        const answer = (optionId: string) => {
          turn.notice.replaceChildren();
          invoke("answer_permission", { runId: turn.runId, requestId: ev.request_id, optionId });
        };
        turn.notice.replaceChildren(
          h("span", { textContent: `Approval needed: ${String(ev.description)} ` }),
          ...options.map((o) => h("button", { className: "secondary", textContent: o.name, onclick: () => answer(o.id) })),
        );
        return;
      }
      turn.notice.textContent = `Waiting for approval: ${String(ev.description ?? ev.command ?? "a command")}. This client can't answer approvals. Stop, or answer from another Hermes client; it is denied automatically when the approval times out.`;
      return;
    case "approval.responded":
      turn.notice.textContent = "";
      return;
    case "run.completed":
      if (!turn.reply && typeof ev.output === "string") appendReply(turn, ev.output);
      setStatus(turn, "done");
      return void refreshSessions();
    case "run.failed":
      turn.runId = undefined;
      setStatus(turn, "failed", `Run failed: ${String(ev.error ?? "unknown error")}`);
      return void refreshSessions();
    case "run.cancelled":
    case "run.interrupted":
      turn.runId = undefined;
      return setStatus(turn, "stopped", "Stopped by the server");
  }
}

function stop() {
  const turn = active;
  if (!turn?.runId) return;
  setStatus(turn, "stopped", "Stopped");
  return invoke("stop_run", { runId: turn.runId });
}

/** Leaving a Session stops its streaming Turn so nothing keeps writing into a closed view. */
function leave() {
  if (active?.runId) return stop();
  if (active) setStatus(active, "stopped", "Stopped");
}

function markCurrent() {
  for (const li of ui!.sessions.children) li.classList.toggle("current", (li as HTMLElement).dataset.id === sessionId);
}

function newChat() {
  leave();
  sessionId = null;
  choice = DEFAULT_CHOICE;
  lastChoice = DEFAULT_CHOICE; // a choice made before the first message is a change too
  renderChoice();
  ui!.turns.replaceChildren(h("p", { className: "empty", textContent: "New chat. Type a message, or drop attachments here." }));
  markCurrent();
}

async function openSession(id: string) {
  leave();
  sessionId = id;
  choice = loadChoice(localStorage, choiceKey(connection, profile, id));
  lastChoice = choice;
  renderChoice();
  markCurrent();
  ui!.turns.replaceChildren(h("p", { className: "empty", textContent: "Loading…" }));
  try {
    const messages = await invoke<Message[]>("session_messages", { id });
    if (sessionId !== id) return;
    ui!.turns.replaceChildren(h("p", { className: "empty", textContent: "This session has no messages yet." }));
    let turn: Turn | null = null;
    for (const m of messages) {
      const text = contentText(m.content);
      if (m.role === "user") turn = addTurn(text, []);
      if (m.role !== "assistant") continue;
      turn ??= addTurn("", []);
      for (const call of m.tool_calls ?? []) addTool(turn, call.function?.name ?? "tool", "", "done");
      if (text) appendReply(turn, (turn.reply ? "\n\n" : "") + text);
    }
  } catch (e) {
    ui!.turns.replaceChildren(h("p", { className: "error", textContent: `Couldn't load this session: ${asError(e).message}` }));
  }
}

async function removeSession(id: string) {
  try {
    await invoke("delete_session", { id });
  } catch (e) {
    ui!.sessionsError.textContent = `Couldn't delete the session: ${asError(e).message}`;
    return;
  }
  if (id === sessionId) newChat();
  await refreshSessions();
}

async function refreshSessions() {
  let sessions: Session[];
  try {
    sessions = await invoke<Session[]>("list_sessions");
  } catch (e) {
    const x = asError(e);
    ui!.sessionsError.textContent = `Couldn't load sessions: ${x.message}`;
    if (x.kind === "unauthorized") setRuns(await reauth("key", "The API server rejected the saved API key."));
    return;
  }
  ui!.sessionsError.textContent = "";
  ui!.sessions.replaceChildren(
    ...sessions.map((s) => {
      const del = h("button", { className: "link", textContent: "Delete" });
      del.onclick = (e) => {
        e.stopPropagation();
        if (del.dataset.armed) return void removeSession(s.id);
        del.dataset.armed = "1";
        del.textContent = "Confirm delete";
        setTimeout(() => {
          delete del.dataset.armed;
          del.textContent = "Delete";
        }, 3000);
      };
      const li = h("li", { onclick: () => openSession(s.id) }, h("span", { textContent: s.title || s.preview || s.id }), ...(overSsh ? [] : [del]));
      li.dataset.id = s.id;
      return li;
    }),
  );
  markCurrent();
}

/** Fills the Profile selector and opens this connection's starting Profile. */
async function setupProfiles() {
  let list: { names: string[]; gateway_default: string | null };
  try {
    list = await invoke("list_profiles");
  } catch (e) {
    ui!.sessionsError.textContent = `Couldn't list profiles: ${asError(e).message}`;
    list = { names: ["default"], gateway_default: null };
  }
  if (!list.names.length) list.names = ["default"];
  ui!.profile.replaceChildren(...list.names.map((n) => h("option", { value: n, textContent: n })));
  const start = startProfile(localStorage, connection, list.names, list.gateway_default);
  await useProfile(start, start === "default" || !list.names.includes("default") ? null : "default");
}

/** Switches the whole view to a Profile: its Sessions, its API key, and a new chat, or `session` when going back. */
async function useProfile(name: string, previous: string | null, session: string | null = null) {
  const back = sessionId; // where "Back to <previous>" returns
  await leave()?.catch(() => {}); // the Stop must reach the old Profile before requests switch to the new one
  try {
    await invoke("set_profile", { name });
    if (!overSsh) {
      const supported = await profileKey(name, previous);
      if (supported === null) return void (previous && (await useProfile(previous, null, back)));
      setRuns(supported);
    }
  } catch (e) {
    ui!.sessionsError.textContent = `Couldn't open the ${name} profile: ${asError(e).message}`;
    if (previous) await useProfile(previous, null, back);
    return;
  }
  if (name !== profile) {
    pending = []; // Attachments stay with the Profile they were added under
    renderPending();
  }
  profile = name;
  ui!.profile.value = name;
  saveProfile(localStorage, connection, name);
  ui!.sessionsError.textContent = "";
  if (session) void openSession(session);
  else newChat();
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
    const fallback = modelLabel(null).toLowerCase().includes(q) ? [item(modelLabel(null), null)] : [];
    list.replaceChildren(
      ...fallback,
      ...groups.flatMap((g) => [h("h3", { textContent: g.name }), ...g.models.map((m) => item(m, { provider: g.provider, model: m }))]),
      ...(groups.length || (q && fallback.length) ? [] : [h("p", { textContent: q ? "No matching models" : modelPicker.title })]),
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

// ---- Shell: icon rail and views ----

type View = { el: HTMLElement; show?: () => void; hide?: () => void };
type ViewName = "chat" | "kanban" | "skills" | "logs";

const LABELS: Record<ViewName, string> = { chat: "Chat", kanban: "Kanban", skills: "Skills", logs: "Logs" };

let current: View | null = null;

// Escape closes the Task drawer unless a dialog is open (dialogs handle their own Escape).
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || document.querySelector("dialog[open]")) return;
  for (const drawer of document.querySelectorAll<HTMLElement>(".drawer")) drawer.hidden = true;
});

/** Replaces the app with the rail and its four views; Chat is shown first. */
function mountShell(chat: HTMLElement) {
  current?.hide?.();
  const views: Record<ViewName, View> = { chat: { el: chat }, kanban: kanbanView(), skills: skillsView(), logs: logsView() };
  const buttons = (Object.keys(views) as ViewName[]).map((name) => {
    const button = h("button", { className: "rail-item", title: LABELS[name], onclick: () => select(name) });
    button.setAttribute("aria-label", LABELS[name]);
    button.dataset.view = name;
    button.append(icon(name));
    return button;
  });
  function select(name: ViewName) {
    current?.hide?.();
    app.querySelector(".settings")?.remove();
    for (const [n, v] of Object.entries(views)) v.el.hidden = n !== name;
    for (const b of buttons) b.setAttribute("aria-current", String(b.dataset.view === name));
    current = views[name];
    current.show?.();
  }
  app.replaceChildren(h("div", { className: "shell" }, h("nav", { className: "rail" }, ...buttons), ...Object.values(views).map((v) => v.el)));
  select("chat");
}

/** Runs a Board or Skills command; a rejected Sign-in cookie gets the usual Re-auth prompt, then one retry. */
async function call<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    if (asError(e).kind !== "unauthorized" || overSsh) throw e;
    if (!signInRequired) throw { kind: "unauthorized", message: "Sign-in is required on the Dashboard for Kanban and Skills" };
    await reauth("sign-in", "The dashboard rejected your sign-in.");
    return await invoke<T>(cmd, args);
  }
}

const when = (epochSeconds: number | null | undefined) =>
  epochSeconds ? new Date(epochSeconds * 1000).toLocaleString() : "";

function select(options: string[], all: string): HTMLSelectElement {
  return h("select", {}, h("option", { value: "", textContent: all }), ...options.map((o) => h("option", { value: o, textContent: o })));
}

/** Keeps the chosen value when the options change. */
function setOptions(el: HTMLSelectElement, options: string[], all: string) {
  const value = el.value;
  el.replaceChildren(h("option", { value: "", textContent: all }), ...options.map((o) => h("option", { value: o, textContent: o })));
  el.value = options.includes(value) ? value : "";
}

// ---- Kanban ----

type Task = { id: string; title: string; body: string | null; assignee: string | null; status: string; priority: number; tenant: string | null; created_at: number | null };
type Comment = { author: string; body: string; created_at: number };
type Detail = { task: Task; comments: Comment[] };

const STATUSES = ["triage", "todo", "scheduled", "ready", "running", "blocked", "review", "done"];
const HIDDEN_WHEN_EMPTY = new Set(["scheduled", "review"]);
const title = (status: string) => status[0].toUpperCase() + status.slice(1);

function kanbanView(): View {
  let tasks: Task[] = [];
  let assignees: string[] = [];
  let status = ""; // the status chip filter, "" for all
  let timer: number | undefined;

  const search = input({ type: "search", placeholder: "Search tasks", required: false });
  const assignee = select([], "All assignees");
  const tenant = select([], "All tenants");
  const archived = h("input", { type: "checkbox" });
  const chips = h("div", { className: "chips" });
  const visible = h("p", { className: "muted" });
  const error = h("p", { className: "error" });
  const newTitle = input({ placeholder: "New task", required: false });
  const columns = h("div", { className: "columns" });
  const drawer = h("aside", { className: "drawer", hidden: true });
  for (const el of [search, assignee, tenant, archived]) el.addEventListener("input", render);

  newTitle.onkeydown = (e) => {
    if (e.key !== "Enter" || !newTitle.value.trim()) return;
    e.preventDefault();
    create({ title: newTitle.value.trim() }, error).then((ok) => ok && (newTitle.value = ""));
  };

  async function create(task: Record<string, unknown>, errorEl: HTMLElement): Promise<boolean> {
    errorEl.textContent = "";
    try {
      await call("kanban_create", { task });
    } catch (e) {
      errorEl.textContent = `Couldn't create the task: ${asError(e).message}`;
      return false;
    }
    await load();
    return true;
  }

  function moreForm() {
    const dialog = h("dialog", { className: "form" });
    const name = input({ value: newTitle.value });
    const body = h("textarea", { rows: 6 });
    const who = select(assignees, "Unassigned");
    const priority = input({ type: "number", value: "0", required: false });
    const tenantIn = input({ required: false });
    const formError = h("p", { className: "error" });
    const form = h("form", {}, h("h2", { textContent: "New task" }), field("Title", name), field("Body", body), field("Assignee", who),
      h("div", { className: "row" }, field("Priority", priority), field("Tenant", tenantIn)), formError,
      h("div", { className: "row" }, h("button", { textContent: "Create" }),
        h("button", { type: "button", className: "secondary", textContent: "Cancel", onclick: () => dialog.close() })));
    form.onsubmit = async (e) => {
      e.preventDefault();
      const task = { title: name.value.trim(), body: body.value || null, assignee: who.value || null,
        priority: priority.value === "" ? null : Number(priority.value), tenant: tenantIn.value.trim() || null };
      if (await create(task, formError)) {
        newTitle.value = "";
        dialog.close();
      }
    };
    dialog.onclose = () => dialog.remove();
    dialog.append(form);
    document.body.append(dialog);
    dialog.showModal();
  }

  /** The non-empty parts of a Dispatch result, one line each. */
  function summarize(result: Record<string, unknown>): string {
    const lines = Object.entries(result).flatMap(([key, value]) => {
      const label = key.replaceAll("_", " ");
      if (typeof value === "number" && value > 0) return [`${label}: ${value}`];
      if (Array.isArray(value) && value.length) return [`${label}: ${value.map((v) => (Array.isArray(v) ? v.slice(0, 2).join(" → ") : String(v))).join(", ")}`];
      if (value === true) return [label];
      return [];
    });
    return lines.join("\n") || "Nothing to do.";
  }

  async function dispatch(dryRun: boolean, errorEl: HTMLElement): Promise<string | null> {
    errorEl.textContent = "";
    try {
      const result = summarize(await call("kanban_dispatch", { dryRun }));
      if (!dryRun) await load();
      return result;
    } catch (e) {
      errorEl.textContent = `Dispatch failed: ${asError(e).message}`;
      return null;
    }
  }

  async function preview() {
    const summary = await dispatch(true, error);
    if (summary === null) return;
    const dialog = h("dialog", { className: "form" });
    const text = h("pre", { className: "plain", textContent: summary });
    const dialogError = h("p", { className: "error" });
    const now = h("button", { textContent: "Dispatch now" });
    now.onclick = async () => {
      now.disabled = true;
      const done = await dispatch(false, dialogError);
      now.disabled = false;
      if (done !== null) {
        text.textContent = done;
        now.hidden = true;
      }
    };
    dialog.append(h("h2", { textContent: "Dispatch preview" }), text, dialogError,
      h("div", { className: "row" }, now, h("button", { className: "secondary", textContent: "Close", onclick: () => dialog.close() })));
    dialog.onclose = () => dialog.remove();
    document.body.append(dialog);
    dialog.showModal();
  }

  const lastDispatch = h("pre", { className: "plain muted" });
  const run = h("button", { className: "secondary", textContent: "Run dispatcher" });
  run.onclick = async () => {
    run.disabled = true;
    const done = await dispatch(false, error);
    run.disabled = false;
    if (done !== null) lastDispatch.textContent = done;
  };

  async function load() {
    try {
      // Profiles rarely change: fetched once, not on every 30 s refresh.
      if (!assignees.length) assignees = await call<string[]>("kanban_assignees").catch(() => []);
      tasks = await call<Task[]>("kanban_board");
      error.textContent = "";
    } catch (e) {
      error.textContent = `Couldn't load the board: ${asError(e).message}`;
    }
    render();
  }

  function render() {
    const names = (pick: (t: Task) => string | null) => [...new Set(tasks.map(pick).filter((v): v is string => !!v))].sort();
    setOptions(assignee, [...new Set([...assignees, ...names((t) => t.assignee)])].sort(), "All assignees");
    setOptions(tenant, names((t) => t.tenant), "All tenants");
    const needle = search.value.trim().toLowerCase();
    const shown = tasks.filter((t) =>
      (archived.checked || t.status !== "archived") &&
      (!assignee.value || t.assignee === assignee.value) &&
      (!tenant.value || t.tenant === tenant.value) &&
      (!needle || `${t.title}\n${t.body ?? ""}`.toLowerCase().includes(needle)));

    const counts = new Map<string, number>();
    for (const t of shown) counts.set(t.status, (counts.get(t.status) ?? 0) + 1);
    const chip = (label: string, value: string) => {
      const b = h("button", { className: "chip-button", textContent: label, onclick: () => ((status = status === value ? "" : value), render()) });
      b.setAttribute("aria-pressed", String(status === value));
      return b;
    };
    chips.replaceChildren(chip(`${shown.length} All`, ""),
      ...[...STATUSES, "archived"].filter((s) => counts.get(s)).map((s) => chip(`${counts.get(s)} ${title(s)}`, s)));

    const inView = shown.filter((t) => !status || t.status === status);
    visible.textContent = `${inView.length} visible task${inView.length === 1 ? "" : "s"}`;
    const lanes = [...STATUSES, ...(archived.checked ? ["archived"] : [])]
      .filter((s) => !status || s === status)
      .filter((s) => !HIDDEN_WHEN_EMPTY.has(s) || counts.get(s));
    columns.replaceChildren(...lanes.map((lane) => {
      const cards = inView.filter((t) => t.status === lane);
      return h("section", { className: "column" },
        h("header", {}, h("span", { textContent: title(lane) }), h("span", { className: "count", textContent: String(cards.length) })),
        ...(cards.length ? cards.map(card) : [h("p", { className: "empty-lane", textContent: "Empty" })]));
    }));
  }

  function card(t: Task): HTMLElement {
    const body = (t.body ?? "").trim();
    return h("article", { className: "card-task", tabIndex: 0, onclick: () => openDrawer(t.id), onkeydown: (e) => e.key === "Enter" && openDrawer(t.id) },
      h("div", { className: "row" }, h("code", { textContent: t.id }), h("span", { className: "badge", textContent: `P${t.priority}` })),
      h("h3", { textContent: t.title }),
      ...(body ? [h("p", { className: "snippet", textContent: body.length > 140 ? `${body.slice(0, 140)}…` : body })] : []),
      ...(t.assignee ? [h("p", { className: "assignee", textContent: `@${t.assignee}` })] : []));
  }

  async function openDrawer(id: string) {
    drawer.hidden = false;
    drawer.replaceChildren(h("p", { className: "loading", textContent: "Loading…" }));
    let detail: Detail;
    try {
      detail = await call<Detail>("kanban_task", { id });
    } catch (e) {
      drawer.replaceChildren(closeButton(), h("p", { className: "error", textContent: `Couldn't load ${id}: ${asError(e).message}` }));
      return;
    }
    const t = detail.task;
    const text = h("textarea", { rows: 3, placeholder: "Add a comment" });
    const commentError = h("p", { className: "error" });
    const add = h("button", { textContent: "Add comment" });
    add.onclick = async () => {
      if (!text.value.trim()) return;
      add.disabled = true;
      try {
        await call("kanban_comment", { id, text: text.value });
        await openDrawer(id);
      } catch (e) {
        commentError.textContent = `Couldn't add the comment: ${asError(e).message}`;
        add.disabled = false;
      }
    };
    const meta = [t.id, title(t.status), t.assignee ? `@${t.assignee}` : "Unassigned", `P${t.priority}`, ...(t.tenant ? [t.tenant] : [])];
    drawer.replaceChildren(
      h("div", { className: "row" }, h("h2", { textContent: t.title }), closeButton()),
      h("p", { className: "muted", textContent: meta.join(" · ") }),
      h("div", { className: "task-body", textContent: t.body || "No description." }),
      h("h2", { textContent: `Comments (${detail.comments.length})` }),
      ...detail.comments.map((c) => h("div", { className: "comment" },
        h("p", { className: "muted", textContent: `${c.author} · ${when(c.created_at)}` }), h("div", { className: "task-body", textContent: c.body }))),
      text, commentError, add,
    );
  }

  function closeButton() {
    const b = h("button", { className: "link", textContent: "×", title: "Close", onclick: () => (drawer.hidden = true) });
    b.setAttribute("aria-label", "Close");
    return b;
  }

  const refresh = h("button", { className: "link", textContent: "↻", title: "Refresh", onclick: load });
  refresh.setAttribute("aria-label", "Refresh");
  const el = h("div", { className: "kanban", hidden: true },
    h("aside", { className: "panel" },
      h("div", { className: "row" }, h("h2", { textContent: "Kanban" }), refresh),
      search, assignee, tenant, h("label", { className: "check" }, archived, " Include archived"), chips,
      h("div", { className: "row" }, h("button", { className: "secondary", textContent: "Preview dispatcher", onclick: preview }), run),
      lastDispatch,
      h("div", { className: "row" }, newTitle, h("button", { className: "link", textContent: "More…", onclick: moreForm })),
      visible, error),
    h("main", {}, h("div", { className: "board-head" }, h("h1", { textContent: "Board" }), h("span", { className: "badge", textContent: "Default" })), columns),
    drawer);
  return {
    el,
    show() {
      load();
      timer = window.setInterval(load, 30_000);
    },
    hide() {
      window.clearInterval(timer);
    },
  };
}

// ---- Skills ----

type Skill = { name: string; description: string; category: string | null; enabled: boolean; essential: boolean };

/** The last Skills list loaded this run, per connection and Profile, so re-opening Skills shows it at once. */
const skillsCache = new Map<string, Skill[]>();

function skillsView(): View {
  let key = ""; // connection and Profile the list is for
  let skills: Skill[] = [];
  let chosen: string | null = null;
  // A refresh that overlaps a toggle may predate its write, so it's dropped if a list is showing.
  let settled = 0; // toggles finished
  let pending = 0; // toggles in flight
  const collapsed = new Set<string>();
  const search = input({ type: "search", placeholder: "Search skills…", required: false });
  const list = h("div", { className: "skill-list" });
  const error = h("p", { className: "error" });
  const placeholder = h("p", { className: "empty", textContent: "Pick a skill to read its SKILL.md." });
  const detail = h("main", { className: "skill-detail" }, placeholder);
  search.oninput = render;

  function toggle(skill: Skill, errorEl: HTMLElement): HTMLInputElement {
    const box = h("input", { type: "checkbox", checked: skill.enabled, disabled: skill.essential, className: "switch" });
    box.setAttribute("role", "switch");
    box.setAttribute("aria-label", `${skill.enabled ? "Disable" : "Enable"} ${skill.name}`);
    box.title = skill.essential ? "Essential: this skill can't be disabled" : skill.enabled ? "Enabled" : "Disabled";
    box.onclick = (e) => e.stopPropagation();
    box.onchange = async () => {
      const enabled = box.checked;
      box.disabled = true;
      errorEl.textContent = "";
      pending++;
      try {
        await call("skill_toggle", { name: skill.name, enabled });
        skill.enabled = enabled;
      } catch (e) {
        box.checked = !enabled;
        errorEl.textContent = `Couldn't ${enabled ? "enable" : "disable"} ${skill.name}: ${asError(e).message}`;
      }
      settled++;
      pending--;
      render();
      if (chosen === skill.name) renderHeader();
    };
    return box;
  }

  function render() {
    const needle = search.value.trim().toLowerCase();
    const shown = skills.filter((s) => !needle || `${s.name} ${s.description}`.toLowerCase().includes(needle));
    const groups = new Map<string, Skill[]>();
    for (const s of shown) groups.set(s.category ?? "general", [...(groups.get(s.category ?? "general") ?? []), s]);
    const order = [...groups.keys()].sort((a, b) => (a === "general" ? -1 : b === "general" ? 1 : a.localeCompare(b)));
    list.replaceChildren(...order.flatMap((group) => {
      const members = groups.get(group)!.sort((a, b) => a.name.localeCompare(b.name));
      const open = !collapsed.has(group) || !!needle;
      const head = h("button", { className: "group", textContent: `${open ? "▾" : "▸"} ${group.toUpperCase()} (${members.length})`,
        onclick: () => (collapsed.has(group) ? collapsed.delete(group) : collapsed.add(group), render()) });
      head.setAttribute("aria-expanded", String(open));
      return [head, ...(open ? members.map((s) => {
        const row = h("div", { className: "skill-row", tabIndex: 0, onclick: () => choose(s.name), onkeydown: (e) => e.key === "Enter" && choose(s.name) },
          toggle(s, error), h("span", { className: "name", textContent: s.name }), h("span", { className: "desc", textContent: s.description }));
        row.classList.toggle("disabled", !s.enabled);
        row.classList.toggle("current", s.name === chosen);
        return row;
      }) : [])];
    }));
  }

  const header = h("div", { className: "row skill-head" });
  const content = h("pre", { className: "plain skill-md" });
  const detailError = h("p", { className: "error" });

  function renderHeader() {
    const skill = skills.find((s) => s.name === chosen);
    if (!skill) return;
    header.replaceChildren(toggle(skill, detailError), h("h2", { textContent: skill.name }),
      h("span", { className: "muted", textContent: `${skill.category ?? "general"}${skill.essential ? " · essential" : ""}` }));
  }

  async function choose(name: string) {
    chosen = name;
    render();
    renderHeader();
    detailError.textContent = "";
    content.textContent = "Loading…";
    detail.replaceChildren(header, detailError, content);
    try {
      const text = await call<string>("skill_content", { name });
      if (chosen === name) content.textContent = text;
    } catch (e) {
      if (chosen === name) content.textContent = "";
      detailError.textContent = `Couldn't read ${name}: ${asError(e).message}`;
    }
  }

  /** Shows the last list for this connection and Profile (or a loading line), then refreshes it. */
  function show() {
    const wanted = `${connection}|${profile}`;
    if (wanted !== key) {
      key = wanted;
      chosen = null;
      detail.replaceChildren(placeholder);
    }
    skills = skillsCache.get(key) ?? [];
    if (skillsCache.has(key)) render();
    else list.replaceChildren(h("p", { className: "empty", textContent: "Loading skills…" }));
    void refresh(key);
  }

  async function refresh(forKey: string) {
    const seen = settled;
    try {
      const fresh = await call<Skill[]>("skills_list");
      if ((settled !== seen || pending) && skillsCache.has(forKey)) return;
      // Unchanged: keep the shown rows (and the objects their switches update) rather than redraw.
      const same = JSON.stringify(skillsCache.get(forKey)) === JSON.stringify(fresh);
      if (!same) skillsCache.set(forKey, fresh);
      if (forKey !== key) return;
      error.textContent = "";
      if (same) return;
      skills = fresh;
    } catch (e) {
      if (forKey !== key) return;
      error.textContent = `Couldn't load skills: ${asError(e).message}`;
    }
    render();
    renderHeader();
  }

  const el = h("div", { className: "skills", hidden: true }, h("aside", { className: "panel" }, search, error, list), detail);
  return { el, show };
}

// ---- Logs (Client log) ----

type LogLine = { time: number; source: string; text: string };
const SOURCES = ["gateway", "ssh", "acp", "kanban", "keyring", "settings"];

function logsView(): View {
  let lines: LogLine[] = [];
  const source = select(SOURCES, "All sources");
  const search = input({ type: "search", placeholder: "Search the log", required: false });
  const out = h("div", { className: "log" });
  const format = (l: LogLine) => `${new Date(l.time).toLocaleTimeString([], { hour12: false })}.${String(l.time % 1000).padStart(3, "0")}  ${l.source.padEnd(8)}  ${l.text}`;
  const matches = (l: LogLine) => (!source.value || l.source === source.value) && (!search.value || l.text.toLowerCase().includes(search.value.toLowerCase()));
  const row = (l: LogLine) => h("div", { className: `log-line ${l.source}`, textContent: format(l) });
  const atEnd = () => out.scrollHeight - out.scrollTop - out.clientHeight < 24;

  function render() {
    out.replaceChildren(...lines.filter(matches).map(row));
    out.scrollTop = out.scrollHeight;
  }
  source.oninput = render;
  search.oninput = render;

  const channel = new Channel<LogLine>();
  channel.onmessage = (line) => {
    lines.push(line);
    if (lines.length > 2000 && matches(lines.shift()!)) out.firstElementChild?.remove();
    if (!matches(line)) return;
    const follow = atEnd();
    out.append(row(line));
    if (follow) out.scrollTop = out.scrollHeight; // scrolled up to read? stay put
  };
  invoke<LogLine[]>("client_log", { onLine: channel }).then((held) => {
    lines = [...held, ...lines];
    render();
  });

  const copy = h("button", { className: "secondary", textContent: "Copy all" });
  copy.onclick = async () => {
    await navigator.clipboard.writeText(lines.map(format).join("\n"));
    copy.textContent = "Copied";
    setTimeout(() => (copy.textContent = "Copy all"), 1500);
  };
  const clear = h("button", { className: "secondary", textContent: "Clear", onclick: async () => {
    await invoke("clear_client_log");
    lines = [];
    render();
  } });
  const el = h("div", { className: "logs", hidden: true },
    h("div", { className: "row toolbar" }, h("h2", { textContent: "Client log" }), source, search, copy, clear),
    h("p", { className: "muted", textContent: "What this app did while talking to Hermes: requests, commands, and connection events. Never message content or secrets." }),
    out);
  return { el, show: () => (out.scrollTop = out.scrollHeight) };
}

boot();
