import { Channel, invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { field, h, icon, input } from "./dom";
import { appearanceTab, applyAppearance } from "./appearance";

type GatewayError = { kind: "unreachable" | "unauthorized" | "not_found" | "http" | "invalid"; message: string };
type Init = { dashboard_url: string | null; api_url: string | null; ssh_host: string | null; keyring: boolean };
type Status = { auth_required: boolean; auth_providers: string[] };
type RunStarted = { run_id: string; session_id: string };
type RunEvent = { event: string; seq?: number; [field: string]: unknown };
type StreamMsg = { type: "event"; data: RunEvent } | { type: "dropped"; message: string };
type Session = { id: string; title?: string | null; preview?: string | null };
type Message = { role: string; content: unknown; tool_calls?: { function?: { name?: string } }[] | null };
type Attachment = { path: string } | { name: string; data_url: string };
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
}

applyAppearance();
const app = document.querySelector<HTMLDivElement>("#app")!;
const banner = document.querySelector<HTMLDivElement>("#banner")!;
const dialog = document.querySelector<HTMLDialogElement>("#reauth")!;
dialog.addEventListener("cancel", (e) => e.preventDefault()); // re-auth can't be dismissed, only completed

const asError = (e: unknown): GatewayError =>
  typeof e === "object" && e !== null && "kind" in e ? (e as GatewayError) : { kind: "invalid", message: String(e) };

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
  if (init.ssh_host) await connectSsh();
  else if (init.dashboard_url && init.api_url) await connect();
  else showPairing();
}

async function connect() {
  app.replaceChildren(h("p", { className: "loading", textContent: "Connecting to the gateway…" }));
  try {
    const status = await invoke<Status>("status");
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
      supported = (await reauth("key", "The API server did not accept the saved API key."))!;
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

/** Blocks until the rejected credential is replaced. For a new key, resolves with whether chat is supported. */
function reauth(kind: "sign-in" | "key", reason: string): Promise<boolean | undefined> {
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
        textContent: "Use a different gateway",
        onclick: () => {
          dialog.close();
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
let overSsh = false; // Hermes over SSH (hermes acp): Approvals can be answered, Sessions can't be deleted
let sessionId: string | null = null;
let active: Turn | null = null;
let pending: Attachment[] = [];
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
    input: h("textarea", { placeholder: runs ? "Message Hermes. Drop, paste, or attach files." : unavailable, rows: 3 }),
    send: h("button", { textContent: "Send" }),
    drop: h("div", { className: "drop", hidden: true, textContent: runs ? "Drop files to attach" : unavailable }),
    composerError: h("p", { className: "error" }),
    toolbar: h("div", { className: "toolbar" }),
    spacer: h("span", { className: "spacer" }),
  };
  const { input: box, send: button } = ui;
  const picker = h("input", { type: "file", multiple: true, hidden: true });
  picker.onchange = async () => {
    await addFiles([...(picker.files ?? [])]);
    picker.value = "";
  };
  const paperclip = h("button", { type: "button", className: "icon-btn", title: "Attach files", onclick: () => picker.click() }, icon("paperclip"));
  paperclip.disabled = !runs;
  ui.toolbar.append(paperclip, picker, ui.spacer, button);
  box.onpaste = (e) => void pasteImages(e);
  box.onkeydown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (!active) send();
    }
  };
  button.onclick = () => (active ? stop() : send());
  app.replaceChildren(
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
  newChat();
  updateComposer();
  refreshSessions();
}

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
      ui!.composerError.textContent = `Not attached: ${err instanceof Error ? err.message : String(err)}`;
    }
  }
  renderPending();
}

function updateComposer() {
  ui!.send.textContent = active ? "Stop" : "Send";
  ui!.send.disabled = !runs || (active !== null && !active.runId);
  ui!.input.disabled = !runs;
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
    run = await invoke<RunStarted>("start_run", { sessionId, text: turn.text, files: turn.files });
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
  updateComposer();
  try {
    await openStream(turn);
  } catch (e) {
    setStatus(turn, "failed", `Connection lost: ${asError(e).message}`);
  }
}

function setRuns(next: boolean | undefined) {
  if (next !== undefined) runs = next;
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
  invoke("stop_run", { runId: turn.runId });
}

/** Leaving a Session stops its streaming Turn so nothing keeps writing into a closed view. */
function leave() {
  if (active?.runId) stop();
  else if (active) setStatus(active, "stopped", "Stopped");
}

function markCurrent() {
  for (const li of ui!.sessions.children) li.classList.toggle("current", (li as HTMLElement).dataset.id === sessionId);
}

function newChat() {
  leave();
  sessionId = null;
  ui!.turns.replaceChildren(h("p", { className: "empty", textContent: "New chat. Type a message, or drop files to attach them." }));
  markCurrent();
}

async function openSession(id: string) {
  leave();
  sessionId = id;
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

boot();
