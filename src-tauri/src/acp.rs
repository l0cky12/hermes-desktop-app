//! Hermes over SSH: `ssh <host> hermes acp`, speaking the Agent Client Protocol (JSON-RPC 2.0,
//! one message per line) on the process's stdio. SSH does the authentication, so nothing here
//! is a secret, and the remote Hermes owns all Session state.

use crate::gateway::Error;
use crate::picker::{model_switch, parse_profile_list, ModelChoice, Profiles};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const HOME_MARK: &str = "hermes-desktop-home ";
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

/// Accepts `host`, `user@host`, or an ssh_config alias. Anything that ssh could read as an
/// option, or that splits into several arguments, is refused.
pub fn valid_host(input: &str) -> Result<String, Error> {
    let host = input.trim();
    let bad = |why: &str| Error::Invalid(format!("{host:?}: {why}"));
    if host.is_empty() {
        return Err(bad("enter a host, user@host, or ssh_config alias"));
    }
    if host.starts_with('-') {
        return Err(bad("must not start with -"));
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad("must not contain spaces"));
    }
    Ok(host.to_owned())
}

pub struct Conn {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, Error>>>>,
    /// Where `session/update` notifications (and permission requests) for a Session go.
    listeners: Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>,
    /// Unanswered `session/request_permission` ids, per Session.
    permissions: Mutex<HashMap<String, Vec<Value>>>,
    alive: AtomicBool,
    closed: Mutex<String>,
    /// Remote `$HOME`: the cwd for new Sessions.
    pub home: String,
    /// Each listed Session's own cwd, so loading it doesn't move it.
    pub cwds: Mutex<HashMap<String, String>>,
    /// `models` from the last `session/new` answer: the list and the Profile's default.
    models: Mutex<Option<Value>>,
    /// The model id each Session is on, as last reported by `session/new`/`session/load` or set here.
    current: Mutex<HashMap<String, String>>,
    /// The last completed `session/prompt` usage per Session: Hermes reports running totals, so a
    /// Turn's own usage is the difference from this.
    usage: Mutex<HashMap<String, Value>>,
    _child: Child,
}

/// `ssh host <command>`. `host` has passed `valid_host`.
fn ssh(host: &str, command: &str) -> Command {
    // Tests point this at a shim that runs the remote command locally (see mock/fake-ssh).
    let mut ssh = Command::new(std::env::var_os("HERMES_DESKTOP_SSH").unwrap_or_else(|| "ssh".into()));
    // BatchMode: no password or host-key prompt nobody could answer; keys or ssh-agent only.
    ssh.args(["-T", "-o", "BatchMode=yes", "--", host, command]).kill_on_drop(true);
    ssh
}

/// Starts `ssh host hermes acp` and completes the ACP handshake.
pub async fn connect(host: &str, profile: Option<&str>) -> Result<Arc<Conn>, Error> {
    let child = ssh(host, &remote(profile))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Unreachable(format!("Could not run ssh: {e}")))?;
    connect_child(host, child).await
}

async fn connect_child(host: &str, mut child: Child) -> Result<Arc<Conn>, Error> {
    let stderr = tokio::spawn(tail(BufReader::new(child.stderr.take().expect("piped")).lines()));
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    let home = loop {
        match tokio::time::timeout(Duration::from_secs(30), lines.next_line()).await {
            Ok(Ok(Some(line))) => match line.strip_prefix(HOME_MARK) {
                Some(home) => break home.to_owned(),
                None => continue, // login-shell chatter
            },
            Ok(_) => return Err(Error::Unreachable(exited(stderr.await.unwrap_or_default()))),
            Err(_) => return Err(Error::Unreachable(format!("No answer from {host} within 30 s"))),
        }
    };
    let conn = Arc::new(Conn {
        stdin: Arc::new(tokio::sync::Mutex::new(child.stdin.take().expect("piped"))),
        next_id: AtomicI64::new(1),
        pending: Mutex::default(),
        listeners: Mutex::default(),
        permissions: Mutex::default(),
        alive: AtomicBool::new(true),
        closed: Mutex::default(),
        home,
        cwds: Mutex::default(),
        models: Mutex::default(),
        current: Mutex::default(),
        usage: Mutex::default(),
        _child: child,
    });
    tokio::spawn(read(Arc::downgrade(&conn), lines, stderr));
    let client = json!({ "name": "hermes-desktop", "version": env!("CARGO_PKG_VERSION") });
    let init = json!({ "protocolVersion": 1, "clientCapabilities": {}, "clientInfo": client });
    tokio::time::timeout(Duration::from_secs(30), conn.request("initialize", init)).await
        .map_err(|_| Error::Unreachable(format!("Hermes on {host} did not initialize within 30 s")))??;
    crate::log::write("acp", format!("connected to {host}"));
    Ok(conn)
}

/// `hermes profile list` on the host (a second, short SSH call), parsed.
pub async fn list_profiles(host: &str) -> Result<Profiles, Error> {
    let mut command = ssh(host, &format!("{HERMES} profile list"));
    command.stdin(Stdio::null());
    let out = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| Error::Unreachable(format!("No answer from {host} within 30 s")))?
        .map_err(|e| Error::Unreachable(format!("Could not run ssh: {e}")))?;
    if !out.status.success() {
        return Err(Error::Unreachable(exited(String::from_utf8_lossy(&out.stderr).into_owned())));
    }
    Ok(parse_profile_list(&String::from_utf8_lossy(&out.stdout)))
}

/// The last few stderr lines: ssh's own errors, or why Hermes stopped.
async fn tail(mut lines: Lines<BufReader<tokio::process::ChildStderr>>) -> String {
    let mut last = std::collections::VecDeque::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if last.len() == 3 {
            last.pop_front();
        }
        last.push_back(line);
    }
    Vec::from(last).join("\n")
}

fn exited(stderr: String) -> String {
    match stderr.trim() {
        "" => "ssh exited without starting Hermes".to_owned(),
        why => format!("ssh exited: {why}"),
    }
}

async fn read(conn: Weak<Conn>, mut lines: Lines<BufReader<ChildStdout>>, stderr: JoinHandle<String>) {
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(msg) = serde_json::from_str::<Value>(&line) {
            let Some(conn) = conn.upgrade() else { return };
            let response = conn.dispatch(msg);
            let stdin = conn.stdin.clone();
            drop(conn); // A blocked automatic reply must not keep the SSH child alive.
            if let Some(response) = response {
                let _ = write_message(&stdin, response).await;
            }
        }
    }
    let why = exited(stderr.await.unwrap_or_default());
    let Some(conn) = conn.upgrade() else { return };
    // Only the fact: the stderr tail in `why` could echo anything Hermes printed.
    crate::log::write("acp", "connection closed");
    *conn.closed.lock().unwrap() = why.clone();
    conn.alive.store(false, Ordering::SeqCst);
    for (_, tx) in conn.pending.lock().unwrap().drain() {
        let _ = tx.send(Err(Error::Unreachable(why.clone())));
    }
    conn.listeners.lock().unwrap().clear();
}

async fn write_message(stdin: &tokio::sync::Mutex<ChildStdin>, msg: Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&msg).expect("serializable");
    line.push(b'\n');
    let mut stdin = stdin.lock().await;
    stdin.write_all(&line).await?;
    stdin.flush().await
}

impl Conn {
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub fn closed_reason(&self) -> String {
        self.closed.lock().unwrap().clone()
    }

    fn dispatch(&self, msg: Value) -> Option<Value> {
        let session = msg["params"]["sessionId"].as_str().unwrap_or_default().to_owned();
        match (msg["method"].as_str(), msg.get("id").cloned()) {
            (Some("session/update"), None) => {
                if let Some(tx) = self.listeners.lock().unwrap().get(&session) {
                    let _ = tx.send(msg["params"]["update"].clone());
                }
            }
            (Some("session/request_permission"), Some(id)) => {
                let request = json!({ "sessionUpdate": "permission_request", "requestId": id, "request": msg["params"] });
                let delivered = self.listeners.lock().unwrap().get(&session).is_some_and(|tx| tx.send(request).is_ok());
                if delivered {
                    self.permissions.lock().unwrap().entry(session).or_default().push(id);
                } else {
                    return Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": { "outcome": "cancelled" } } }));
                }
            }
            (Some(_), Some(id)) => {
                // We advertise no client capabilities (fs, terminal), so nothing else should arrive.
                let error = json!({ "code": -32601, "message": "Method not found" });
                return Some(json!({ "jsonrpc": "2.0", "id": id, "error": error }));
            }
            (Some(_), None) => {}
            (None, Some(id)) => {
                let Some(tx) = id.as_i64().and_then(|id| self.pending.lock().unwrap().remove(&id)) else { return None };
                let result = match msg.get("error") {
                    Some(e) => Err(Error::Http(e["message"].as_str().unwrap_or("Hermes returned an error").to_owned())),
                    None => Ok(msg["result"].clone()),
                };
                let _ = tx.send(result);
            }
            (None, None) => {}
        }
        None
    }

    async fn write(&self, msg: Value) -> Result<(), Error> {
        write_message(&self.stdin, msg).await.map_err(|_| Error::Unreachable(self.closed_reason()))
    }

    async fn respond(&self, id: Value, result: Value) {
        let _ = self.write(json!({ "jsonrpc": "2.0", "id": id, "result": result })).await;
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        crate::log::write("acp", method.to_owned());
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if !self.alive() {
            self.pending.lock().unwrap().remove(&id);
            return Err(Error::Unreachable(self.closed_reason()));
        }
        self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await?;
        let result = rx.await.unwrap_or_else(|_| Err(Error::Unreachable(self.closed_reason())));
        if method == "session/new" {
            if let Ok(created) = &result {
                *self.models.lock().unwrap() = Some(created["models"].clone());
            }
        }
        result
    }

    pub fn models(&self) -> Option<Value> {
        self.models.lock().unwrap().clone()
    }

    /// Records the model a `session/new` or `session/load` answer says the Session is on.
    pub fn note_model(&self, session: &str, answer: &Value) {
        if let Some(id) = answer["models"]["currentModelId"].as_str() {
            self.current.lock().unwrap().insert(session.to_owned(), id.to_owned());
        }
    }

    /// Switches the Session's model between Turns, only when it's on another one than chosen.
    pub async fn use_model(&self, session: &str, model: Option<&ModelChoice>) -> Result<(), Error> {
        let current = self.current.lock().unwrap().get(session).cloned();
        let default = self.models().and_then(|m| m["currentModelId"].as_str().map(str::to_owned));
        let Some(wanted) = model_switch(model, current.as_deref(), default.as_deref()) else { return Ok(()) };
        self.request("session/set_model", json!({ "sessionId": session, "modelId": wanted })).await?;
        self.current.lock().unwrap().insert(session.to_owned(), wanted);
        // The switch rebuilds the Session's AIAgent in Hermes, which starts its usage totals from zero.
        self.usage.lock().unwrap().remove(session);
        Ok(())
    }

    /// This Turn's usage from a completed `session/prompt` result, remembering its totals for the
    /// next Turn.
    // ponytail: a Stopped Turn's answer is never read, so its tokens count toward the next Turn.
    pub fn turn_usage(&self, session: &str, result: &Value) -> Option<Value> {
        let totals = result.get("usage").filter(|u| u.is_object())?.clone();
        let before = self.usage.lock().unwrap().insert(session.to_owned(), totals.clone());
        Some(usage_delta(&totals, before.as_ref()))
    }

    /// Routes a Session's updates to the returned receiver until `unsubscribe` or disconnect.
    pub fn subscribe(&self, session: &str) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.listeners.lock().unwrap().insert(session.to_owned(), tx);
        rx
    }

    pub fn unsubscribe(&self, session: &str) {
        self.listeners.lock().unwrap().remove(session);
    }

    /// Sends a request and also returns the updates it streamed before answering
    /// (`session/load` replays the transcript that way).
    pub async fn collect(&self, session: &str, method: &str, params: Value) -> Result<(Value, Vec<Value>), Error> {
        let mut rx = self.subscribe(session);
        let result = self.request(method, params).await;
        self.unsubscribe(session);
        let mut updates = Vec::new();
        while let Ok(update) = rx.try_recv() {
            updates.push(update);
        }
        Ok((result?, updates))
    }

    /// Answers one permission request; `None` means cancelled. Unknown ids are ignored.
    pub async fn answer(&self, session: &str, id: Value, option: Option<String>) {
        let known = self.permissions.lock().unwrap().get_mut(session).is_some_and(|ids| {
            let before = ids.len();
            ids.retain(|pending| *pending != id);
            ids.len() < before
        });
        if known {
            let outcome = match option {
                Some(option) => json!({ "outcome": "selected", "optionId": option }),
                None => json!({ "outcome": "cancelled" }),
            };
            self.respond(id, json!({ "outcome": outcome })).await;
        }
    }

    /// Stops the Session's prompt; the spec requires open permission requests be cancelled too.
    pub async fn cancel(&self, session: &str) {
        let _ = self.write(json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": session } })).await;
        let open = self.permissions.lock().unwrap().remove(session).unwrap_or_default();
        for id in open {
            self.respond(id, json!({ "outcome": { "outcome": "cancelled" } })).await;
        }
    }
}

/// Turns a Run `input` (see attach.rs) into ACP prompt content blocks.
pub fn prompt_blocks(input: Value) -> Vec<Value> {
    let Value::Array(messages) = input else {
        return vec![json!({ "type": "text", "text": input.as_str().unwrap_or_default() })];
    };
    let parts = messages.first().and_then(|m| m["content"].as_array()).cloned().unwrap_or_default();
    parts
        .into_iter()
        .filter_map(|part| match part["type"].as_str()? {
            "text" => Some(part),
            "image_url" => {
                let (mime, data) = part["image_url"]["url"].as_str()?.strip_prefix("data:")?.split_once(";base64,")?;
                Some(json!({ "type": "image", "mimeType": mime, "data": data }))
            }
            _ => None,
        })
        .collect()
}

/// Maps one `session/update` onto the Run events the chat view already understands.
/// `tools` remembers tool titles by id, because completions carry only the id.
pub fn translate(update: &Value, tools: &mut HashMap<String, String>) -> Vec<Value> {
    let id = update["toolCallId"].as_str().unwrap_or_default().to_owned();
    let status = update["status"].as_str().unwrap_or_default();
    let finished = |title: String| json!({ "event": "tool.completed", "tool": title, "error": status == "failed" });
    match update["sessionUpdate"].as_str().unwrap_or_default() {
        "agent_message_chunk" => {
            vec![json!({ "event": "message.delta", "delta": update["content"]["text"].as_str().unwrap_or_default() })]
        }
        "tool_call" => {
            let title = update["title"].as_str().unwrap_or("tool").to_owned();
            let mut events = vec![json!({ "event": "tool.started", "tool": title, "preview": "" })];
            match status {
                "completed" | "failed" => events.push(finished(title)),
                _ => drop(tools.insert(id, title)),
            }
            events
        }
        "tool_call_update" if matches!(status, "completed" | "failed") => {
            tools.remove(&id).map(finished).into_iter().collect()
        }
        "permission_request" => {
            let request = &update["request"];
            let options: Vec<Value> = request["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| json!({ "id": o["optionId"], "name": o["name"] }))
                .collect();
            let description = request["toolCall"]["title"].as_str().unwrap_or("a command");
            vec![json!({ "event": "approval.request", "description": description, "request_id": update["requestId"], "options": options })]
        }
        _ => vec![],
    }
}

/// ACP usage fields, and the API server's `usage` field each maps to.
const USAGE_FIELDS: [(&str, &str); 3] =
    [("inputTokens", "input_tokens"), ("outputTokens", "output_tokens"), ("cachedReadTokens", "cache_read_tokens")];

/// Hermes's ACP usage is the running totals of the Session's AIAgent, not the Turn's own (the ACP spec
/// says per-Turn), so the Turn's usage is `totals - before`, in the API server's `run.completed` shape.
/// Totals below `before` mean Hermes rebuilt the AIAgent (e.g. `/new`) and count from zero.
/// Like the API server's, `inputTokens` includes cache reads and writes.
// ponytail: a rebuild whose first Turn outgrows every old total reads as a small Turn; Hermes
// sending per-Turn usage would fix it.
pub fn usage_delta(totals: &Value, before: Option<&Value>) -> Value {
    let count = |usage: &Value, key: &str| usage[key].as_u64().unwrap_or(0);
    let before = before.filter(|b| USAGE_FIELDS.iter().all(|(k, _)| count(totals, k) >= count(b, k)));
    let turn = USAGE_FIELDS.map(|(acp, http)| (http, count(totals, acp) - before.map_or(0, |b| count(b, acp))));
    json!(serde_json::Map::from_iter(turn.map(|(k, v)| (k.to_owned(), json!(v)))))
}

/// The terminal Run event for a `session/prompt` result. `usage` (`Conn::turn_usage`) is read only
/// for a completed Turn, so a Stopped or refused Turn's tokens count toward the next one.
pub fn finished(result: Result<Value, Error>, usage: impl FnOnce(&Value) -> Option<Value>) -> Value {
    match result {
        Ok(r) if r["stopReason"] == "cancelled" => json!({ "event": "run.cancelled" }),
        Ok(r) if r["stopReason"] == "refusal" => json!({ "event": "run.failed", "error": "Hermes refused the prompt" }),
        Ok(r) => match usage(&r) {
            Some(usage) => json!({ "event": "run.completed", "usage": usage }),
            None => json!({ "event": "run.completed" }),
        },
        Err(Error::Unreachable(m) | Error::Unauthorized(m) | Error::NotFound(m) | Error::Http(m) | Error::Invalid(m)) => {
            json!({ "event": "run.failed", "error": m })
        }
    }
}

/// Rebuilds API-server-shaped messages from a `session/load` replay, for the chat view.
pub fn history(updates: &[Value]) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for update in updates {
        let role = match update["sessionUpdate"].as_str().unwrap_or_default() {
            "user_message_chunk" => "user",
            "agent_message_chunk" => "assistant",
            "tool_call" => {
                let name = update["title"].as_str().unwrap_or("tool");
                messages.push(json!({ "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": name } }] }));
                continue;
            }
            _ => continue,
        };
        let text = update["content"]["text"].as_str().unwrap_or_default();
        match messages.last_mut() {
            // Chunks stream in pieces; a Turn's reply is one message until a tool call splits it.
            Some(last) if last["role"] == role && last.get("tool_calls").is_none() => {
                let joined = format!("{}{text}", last["content"].as_str().unwrap_or_default());
                last["content"] = json!(joined);
            }
            _ => messages.push(json!({ "role": role, "content": text })),
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn the_reader_does_not_keep_a_replaced_connection_alive() {
        let child = Command::new("sh").args(["-c", r#"
            printf 'hermes-desktop-home /tmp\n'
            read -r request
            printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
            read -r request
            printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"unsupported"}'
            while read -r request; do :; done
        "#]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .kill_on_drop(true).spawn().unwrap();
        let conn = tokio::time::timeout(Duration::from_secs(5), connect_child("test", child)).await.unwrap().unwrap();
        let weak = Arc::downgrade(&conn);
        let stdin = conn.stdin.clone();
        let mut locked = stdin.lock().await;
        locked.write_all(b"trigger\n").await.unwrap();
        // Hold the writer while the reader handles the automatic method-not-found reply.
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(conn);
        assert!(weak.upgrade().is_none(), "the reader must not own the SSH child after a Profile switch");
    }

    /// Needs a working local `hermes`: `HERMES_DESKTOP_SSH=$PWD/../mock/fake-ssh cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn lists_and_loads_sessions_from_real_hermes() {
        let conn = connect("localhost", None).await.unwrap();
        assert!(conn.home.starts_with('/'));
        let page = conn.request("session/list", json!({})).await.unwrap();
        let first = &page["sessions"][0];
        let id = first["sessionId"].as_str().expect("at least one session");
        let params = json!({ "sessionId": id, "cwd": first["cwd"], "mcpServers": [] });
        let (loaded, replay) = conn.collect(id, "session/load", params).await.unwrap();
        assert!(!loaded.is_null());
        assert!(history(&replay).iter().any(|m| m["role"] == "user"), "{replay:?}");
    }

    #[test]
    fn hosts_that_ssh_could_misread_are_refused() {
        assert_eq!(valid_host(" liam@hermes.lan ").unwrap(), "liam@hermes.lan");
        assert!(valid_host("hermes-box").is_ok());
        for bad in ["", "-oProxyCommand=x", "a b", "a\nb"] {
            assert!(valid_host(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn run_input_becomes_prompt_blocks() {
        assert_eq!(prompt_blocks(json!("hi")), vec![json!({ "type": "text", "text": "hi" })]);
        let input = json!([{ "role": "user", "content": [
            { "type": "text", "text": "look" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,iVBORw==" } },
        ] }]);
        assert_eq!(prompt_blocks(input)[1], json!({ "type": "image", "mimeType": "image/png", "data": "iVBORw==" }));
    }

    #[test]
    fn updates_become_run_events() {
        let mut tools = HashMap::new();
        let chunk = json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hi" } });
        assert_eq!(translate(&chunk, &mut tools), vec![json!({ "event": "message.delta", "delta": "Hi" })]);
        let start = json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "terminal: ls", "status": "pending" });
        assert_eq!(translate(&start, &mut tools)[0]["event"], "tool.started");
        let done = json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "failed" });
        assert_eq!(translate(&done, &mut tools), vec![json!({ "event": "tool.completed", "tool": "terminal: ls", "error": true })]);
        // The permission bubble's own close has an id we never started.
        let perm = json!({ "sessionUpdate": "tool_call_update", "toolCallId": "perm-check-1", "status": "completed" });
        assert!(translate(&perm, &mut tools).is_empty());
        assert_eq!(finished(Ok(json!({ "stopReason": "cancelled" })), |_| None)["event"], "run.cancelled");
        assert_eq!(finished(Ok(json!({ "stopReason": "end_turn" })), |_| None), json!({ "event": "run.completed" }));
    }

    #[test]
    fn turn_usage_is_the_change_in_session_totals() {
        let totals = |i: u64, o: u64, c: u64| json!({ "inputTokens": i, "outputTokens": o, "totalTokens": i + o, "cachedReadTokens": c });
        let turn = |i: u64, o: u64, c: u64| json!({ "input_tokens": i, "output_tokens": o, "cache_read_tokens": c });
        assert_eq!(usage_delta(&totals(1000, 50, 0), None), turn(1000, 50, 0));
        assert_eq!(usage_delta(&totals(2500, 80, 900), Some(&totals(1000, 50, 0))), turn(1500, 30, 900));
        // A rebuilt AIAgent (model switch, /new) restarts its totals.
        assert_eq!(usage_delta(&totals(700, 20, 0), Some(&totals(2500, 80, 900))), turn(700, 20, 0));
        // Optional cache field missing (provider without cache accounting).
        assert_eq!(usage_delta(&json!({ "inputTokens": 10, "outputTokens": 2, "totalTokens": 12 }), None), turn(10, 2, 0));
        let done = finished(Ok(json!({ "stopReason": "end_turn" })), |_| Some(turn(1, 2, 0)));
        assert_eq!(done, json!({ "event": "run.completed", "usage": turn(1, 2, 0) }));
        // A Stopped or refused Turn's totals are left for the next Turn's difference, not dropped.
        for reason in ["cancelled", "refusal"] {
            finished(Ok(json!({ "stopReason": reason })), |_| panic!("read the usage of a {reason} Turn"));
        }
    }

    #[test]
    fn replay_rebuilds_messages() {
        let text = |kind: &str, t: &str| json!({ "sessionUpdate": kind, "content": { "type": "text", "text": t } });
        let replay = [
            text("user_message_chunk", "list files"),
            text("agent_message_chunk", "Sure"),
            json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "ls", "status": "completed" }),
            text("agent_message_chunk", "Done, "),
            text("agent_message_chunk", "two files."),
        ];
        assert_eq!(
            history(&replay),
            vec![
                json!({ "role": "user", "content": "list files" }),
                json!({ "role": "assistant", "content": "Sure" }),
                json!({ "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "ls" } }] }),
                json!({ "role": "assistant", "content": "Done, two files." }),
            ]
        );
    }
}
