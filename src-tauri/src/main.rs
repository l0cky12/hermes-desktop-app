mod acp;
mod attach;
mod gateway;
mod log;
mod picker;
mod remote;
mod sse;
mod title;
mod work;

use gateway::{absorb_cookies, cookie_header, describe, endpoint, parse_origin, send, Error};
use reqwest::{header, Method, RequestBuilder, Response, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{async_runtime, ipc::Channel, DragDropEvent, Manager, State, WindowEvent};

const KEYRING_SERVICE: &str = "local.hermes-desktop";

/// The only thing written to disk: where the gateway is, or the SSH host running Hermes. Never secrets.
#[derive(Serialize, Deserialize, Default)]
struct Settings {
    #[serde(default)]
    dashboard_url: String,
    #[serde(default)]
    api_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ssh_host: Option<String>,
}

/// Lives only in the OS keyring (or memory). Bound to the origins it was issued for, so an
/// edited settings file can't redirect stored credentials to another host.
#[derive(Serialize, Deserialize, Default, Clone)]
struct Creds {
    dashboard_url: String,
    api_url: String,
    api_key: Option<String>,
    cookies: BTreeMap<String, String>,
    /// Named Profiles' own API keys; `api_key` is the default Profile's.
    #[serde(default)]
    profile_keys: BTreeMap<String, String>,
}

#[derive(PartialEq)]
struct Gateway {
    dashboard: Url,
    api: Url,
}

#[derive(Default)]
struct Inner {
    gateway: Option<Gateway>,
    /// Set instead of `gateway` when Hermes is reached with `ssh host hermes acp`.
    ssh_host: Option<String>,
    creds: Creds,
    keyring: bool,
    run_stop: bool,
    /// The Profile this app is showing (already `picker::valid_profile`); `None` until the chat
    /// view picks one. Over SSH, `None` runs plain `hermes acp`, meaning the Gateway's default.
    profile: Option<String>,
}

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

struct AppState {
    client: reqwest::Client,
    slow_client: reqwest::Client,
    settings_path: PathBuf,
    inner: Mutex<Inner>,
    streams: Mutex<HashMap<String, async_runtime::JoinHandle<()>>>,
    /// Paths the user actually dropped on the window; the only files a Run may read.
    dropped: Mutex<HashSet<PathBuf>>,
    hermes: tokio::sync::Mutex<Option<Arc<acp::Conn>>>,
    /// SSH-mode Runs by run id: their Session, and the prompt until `stream_run` sends it.
    acp_runs: Mutex<HashMap<String, AcpRun>>,
    /// Over SSH: each listed Skill's SKILL.md path, so reading one never takes a path from the webview.
    skill_paths: Mutex<HashMap<String, String>>,
}

struct AcpRun {
    session: String,
    prompt: Option<Vec<Value>>,
}

impl AppState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Dashboard request; the sign-in cookie is only ever attached here.
    fn dashboard(&self, method: Method, path: &[&str], query: &[(&str, &str)]) -> Result<RequestBuilder, Error> {
        let inner = self.lock();
        let gateway = inner.gateway.as_ref().ok_or_else(not_configured)?;
        let request = self.client.request(method, endpoint(&gateway.dashboard, path, query));
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
        let url = endpoint(&gateway.api, &picker::api_segments(inner.profile.as_deref(), path), query);
        Ok(self.client.request(method, url).bearer_auth(key))
    }

    /// Writes current credentials to the keyring. `false` means they are held in memory only.
    async fn persist(&self) -> bool {
        let (creds, keyring) = {
            let inner = self.lock();
            (inner.creds.clone(), inner.keyring)
        };
        if !keyring {
            return false;
        }
        let saved = async_runtime::spawn_blocking(move || save_creds(creds)).await.unwrap_or(false);
        if !saved {
            self.lock().keyring = false;
        }
        saved
    }

    fn ssh(&self) -> bool {
        self.lock().ssh_host.is_some()
    }

    fn ssh_host(&self) -> Option<String> {
        self.lock().ssh_host.clone()
    }

    /// A Dashboard request's JSON answer, keeping any cookies it rotated.
    async fn dashboard_json(&self, request: RequestBuilder) -> Result<Value, Error> {
        let response = send(&self.client, request).await?;
        self.absorb(&response).await;
        json_body(response).await
    }

    fn kanban(&self, method: Method, path: &[&str], query: &[(&str, &str)]) -> Result<RequestBuilder, Error> {
        self.dashboard(method, &[&["api", "plugins", "kanban"], path].concat(), query)
    }

    /// The SSH connection to Hermes, (re)started on demand.
    async fn hermes(&self) -> Result<Arc<acp::Conn>, Error> {
        let mut slot = self.hermes.lock().await;
        if let Some(conn) = slot.as_ref().filter(|c| c.alive()) {
            return Ok(conn.clone());
        }
        let (host, profile) = {
            let inner = self.lock();
            (inner.ssh_host.clone().ok_or_else(not_configured)?, inner.profile.clone())
        };
        let conn = acp::connect(&host, profile.as_deref()).await?;
        *slot = Some(conn.clone());
        Ok(conn)
    }

    fn write_settings(&self, settings: &Settings) -> Result<(), Error> {
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(self.settings_path.parent().expect("settings file has a parent"))?;
            std::fs::write(&self.settings_path, serde_json::to_vec_pretty(settings).expect("serializable"))
        };
        write().map_err(|e| Error::Invalid(format!("Could not save settings: {e}")))
    }

    async fn absorb(&self, response: &Response) {
        let changed = absorb_cookies(&mut self.lock().creds.cookies, response.headers());
        if changed {
            self.persist().await;
        }
    }
}

fn not_configured() -> Error {
    Error::Invalid("No gateway configured".into())
}

async fn json_body(response: Response) -> Result<Value, Error> {
    response.json().await.map_err(|e| Error::Http(format!("Unreadable response: {}", describe(&e))))
}

fn keyring_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, "credentials")
}

/// A Secret Service that is activatable but never answers, or an unlock prompt nobody sees,
/// blocks keyring calls forever. Bound them; no answer counts as "keyring unavailable".
// ponytail: fixed 10 s bound; a manual unlock slower than that runs memory-only until next launch.
fn bounded<T: Send + 'static>(call: impl FnOnce() -> keyring::Result<T> + Send + 'static) -> keyring::Result<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(call()));
    rx.recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Err(keyring::Error::PlatformFailure("no answer from the OS keyring within 10 s".into())))
}

/// `(stored credentials, keyring usable)`. An unusable keyring is reported, never worked around.
fn load_creds() -> (Option<Creds>, bool) {
    match bounded(|| keyring_entry()?.get_password()) {
        Ok(json) => (serde_json::from_str(&json).ok(), true),
        Err(keyring::Error::NoEntry) => (None, true),
        Err(e) => {
            log::write("keyring", format!("unavailable, credentials will not be saved: {e}"));
            (None, false)
        }
    }
}

fn save_creds(creds: Creds) -> bool {
    let result = bounded(move || {
        let entry = keyring_entry()?;
        if creds.api_key.is_none() && creds.cookies.is_empty() && creds.profile_keys.is_empty() {
            match entry.delete_credential() {
                Err(keyring::Error::NoEntry) => Ok(()),
                other => other,
            }
        } else {
            entry.set_password(&serde_json::to_string(&creds).expect("serializable"))
        }
    });
    if let Err(e) = &result {
        log::write("keyring", format!("save failed, credentials kept in memory only: {e}"));
    }
    result.is_ok()
}

fn load_settings(path: &Path) -> Option<Settings> {
    serde_json::from_slice(&std::fs::read(path).ok()?)
        .inspect_err(|e| log::write("settings", format!("ignoring unreadable {}: {e}", path.display())))
        .ok()
}

fn load_ssh_host(settings: &Settings, path: &Path) -> Option<String> {
    let host = acp::valid_host(settings.ssh_host.as_deref()?)
        .inspect_err(|_| log::write("settings", format!("ignoring invalid SSH host in {}", path.display())));
    host.ok()
}

fn load_gateway(settings: &Settings, path: &Path) -> Option<Gateway> {
    match (parse_origin(&settings.dashboard_url), parse_origin(&settings.api_url)) {
        (Ok(dashboard), Ok(api)) => Some(Gateway { dashboard, api }),
        _ => {
            log::write("settings", format!("ignoring invalid gateway URLs in {}", path.display()));
            None
        }
    }
}

#[derive(Serialize)]
struct Init {
    dashboard_url: Option<String>,
    api_url: Option<String>,
    ssh_host: Option<String>,
    keyring: bool,
}

/// Loads stored credentials. Runs off the main thread so a slow keyring never blocks the window.
#[tauri::command]
async fn init(state: State<'_, AppState>) -> Result<Init, Error> {
    let (stored, keyring) = async_runtime::spawn_blocking(load_creds).await.unwrap_or((None, false));
    let mut inner = state.lock();
    inner.keyring = keyring;
    inner.creds = match (stored, &inner.gateway) {
        (Some(c), Some(g)) if c.dashboard_url == g.dashboard.as_str() && c.api_url == g.api.as_str() => c,
        (Some(_), _) => {
            log::write("keyring", "stored credentials belong to a different gateway; ignoring them");
            Creds::default()
        }
        (None, _) => Creds::default(),
    };
    Ok(Init {
        dashboard_url: inner.gateway.as_ref().map(|g| g.dashboard.to_string()),
        api_url: inner.gateway.as_ref().map(|g| g.api.to_string()),
        ssh_host: inner.ssh_host.clone(),
        keyring,
    })
}

/// Points the client at a gateway. Changing gateways drops every credential for the old one.
#[tauri::command]
async fn configure(state: State<'_, AppState>, dashboard_url: String, api_url: String) -> Result<(), Error> {
    let gateway = Gateway { dashboard: parse_origin(&dashboard_url)?, api: parse_origin(&api_url)? };
    let changed = {
        let mut inner = state.lock();
        let changed = inner.gateway.as_ref() != Some(&gateway);
        inner.ssh_host = None;
        if changed {
            inner.profile = None;
            inner.creds = Creds {
                dashboard_url: gateway.dashboard.to_string(),
                api_url: gateway.api.to_string(),
                ..Creds::default()
            };
            inner.gateway = Some(gateway);
        }
        changed
    };
    if changed {
        state.persist().await;
    }
    Ok(())
}

#[derive(Serialize)]
struct Status {
    auth_required: bool,
    auth_providers: Vec<String>,
}

#[tauri::command]
async fn status(state: State<'_, AppState>) -> Result<Status, Error> {
    let body = json_body(send(&state.client, state.dashboard(Method::GET, &["api", "status"], &[])?).await?).await?;
    let auth_required = body["auth_required"]
        .as_bool()
        .ok_or_else(|| Error::Invalid("Not a Hermes dashboard: /api/status has no auth_required".into()))?;
    let auth_providers = body["auth_providers"]
        .as_array()
        .map(|list| list.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    Ok(Status { auth_required, auth_providers })
}

/// Basic-provider sign-in. Returns whether the sign-in cookie was saved to the keyring.
#[tauri::command]
async fn sign_in(state: State<'_, AppState>, username: String, password: String) -> Result<bool, Error> {
    let body = json!({ "provider": "basic", "username": username, "password": password });
    let request = state.dashboard(Method::POST, &["auth", "password-login"], &[])?.json(&body);
    let response = send(&state.client, request).await?;
    absorb_cookies(&mut state.lock().creds.cookies, response.headers());
    Ok(state.persist().await)
}

/// Confirms the sign-in cookie via `/api/auth/me`, keeping any rotated cookies it hands back.
#[tauri::command]
async fn check_sign_in(state: State<'_, AppState>) -> Result<String, Error> {
    match send(&state.client, state.dashboard(Method::GET, &["api", "auth", "me"], &[])?).await {
        Ok(response) => {
            state.absorb(&response).await;
            Ok(json_body(response).await?["display_name"].as_str().unwrap_or_default().to_owned())
        }
        Err(Error::Unauthorized(message)) => {
            state.lock().creds.cookies.clear();
            state.persist().await;
            Err(Error::Unauthorized(message))
        }
        Err(e) => Err(e),
    }
}

/// Whether chat is possible: the API server must accept Runs and stream their events.
#[tauri::command]
async fn capabilities(state: State<'_, AppState>) -> Result<bool, Error> {
    let request = state.api(Method::GET, &["v1", "capabilities"], &[])?;
    let body = json_body(send(&state.client, request).await?).await?;
    let on = |feature: &str| body["features"][feature].as_bool().unwrap_or(false);
    state.lock().run_stop = on("run_stop");
    Ok(on("run_submission") && on("run_events_sse"))
}

#[derive(Serialize)]
struct KeyAccepted {
    runs: bool,
    saved: bool,
}

/// Keeps the key only if `/v1/capabilities` accepts it.
#[tauri::command]
async fn set_api_key(state: State<'_, AppState>, key: String) -> Result<KeyAccepted, Error> {
    let previous = state.lock().replace_key(Some(key.trim().to_owned()));
    match capabilities(state.clone()).await {
        Ok(runs) => Ok(KeyAccepted { runs, saved: state.persist().await }),
        Err(e) => {
            state.lock().replace_key(previous);
            Err(e)
        }
    }
}

#[tauri::command]
fn finish_pairing(state: State<'_, AppState>) -> Result<(), Error> {
    let settings = {
        let inner = state.lock();
        let gateway = inner.gateway.as_ref().ok_or_else(not_configured)?;
        Settings { dashboard_url: gateway.dashboard.to_string(), api_url: gateway.api.to_string(), ssh_host: None }
    };
    state.write_settings(&settings)
}

/// Switches to Hermes over SSH, saving the host once `hermes acp` answers. Drops every gateway
/// credential: SSH keys do the authentication.
#[tauri::command]
async fn configure_ssh(state: State<'_, AppState>, host: String) -> Result<(), Error> {
    let host = acp::valid_host(&host)?;
    {
        let mut inner = state.lock();
        inner.gateway = None;
        inner.creds = Creds::default();
        inner.ssh_host = Some(host.clone());
        inner.profile = None;
    }
    state.hermes.lock().await.take();
    state.persist().await;
    state.hermes().await?;
    state.write_settings(&Settings { ssh_host: Some(host), ..Settings::default() })
}

#[tauri::command]
async fn connect_ssh(state: State<'_, AppState>) -> Result<(), Error> {
    state.hermes().await.map(drop)
}

/// Profiles to offer, and the Gateway's default Profile, which this app never changes.
#[tauri::command]
async fn list_profiles(state: State<'_, AppState>) -> Result<picker::Profiles, Error> {
    if state.ssh() {
        let host = state.lock().ssh_host.clone().ok_or_else(not_configured)?;
        let profiles = acp::list_profiles(&host).await?;
        // Plain `hermes acp` (no Profile picked yet) runs the Gateway's default, so picking that
        // one next doesn't restart Hermes.
        state.lock().profile.get_or_insert_with(|| profiles.gateway_default.clone().unwrap_or_else(|| "default".into()));
        return Ok(profiles);
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

/// Dictation: the Dashboard transcribes with the current Profile's speech-to-text settings.
#[tauri::command]
async fn transcribe(state: State<'_, AppState>, data_url: String) -> Result<String, Error> {
    let profile = state.lock().profile.clone();
    let query: Vec<(&str, &str)> = profile.as_deref().map(|p| vec![("profile", p)]).unwrap_or_default();
    let request = state.dashboard(Method::POST, &["api", "audio", "transcribe"], &query)?.json(&json!({ "data_url": data_url }));
    let body = json_body(send(&state.slow_client, request).await?).await?;
    Ok(body["transcript"].as_str().unwrap_or_default().to_owned())
}

/// Answers an Approval request from Hermes over SSH; `None` denies by cancelling.
#[tauri::command]
async fn answer_permission(
    state: State<'_, AppState>,
    run_id: String,
    request_id: Value,
    option_id: Option<String>,
) -> Result<(), Error> {
    let session = state.acp_runs.lock().unwrap().get(&run_id).map(|r| r.session.clone());
    if let Some(session) = session {
        state.hermes().await?.answer(&session, request_id, option_id).await;
    }
    Ok(())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Value, Error> {
    if state.ssh() {
        let hermes = state.hermes().await?;
        // ponytail: first page only, like the API server's newest 100; follow nextCursor when it matters.
        let mut page = hermes.request("session/list", json!({})).await?;
        let mut cwds = hermes.cwds.lock().unwrap();
        let sessions = page["sessions"].as_array_mut().map(std::mem::take).unwrap_or_default();
        return Ok(sessions
            .into_iter()
            .map(|s| {
                let id = s["sessionId"].as_str().unwrap_or_default().to_owned();
                if let Some(cwd) = s["cwd"].as_str() {
                    cwds.insert(id.clone(), cwd.to_owned());
                }
                json!({ "id": id, "title": s["title"] })
            })
            .collect());
    }
    // ponytail: newest 100 only, no paging; add offset paging when 100 stops being enough.
    let request = state.api(Method::GET, &["api", "sessions"], &[("limit", "100")])?;
    Ok(json_body(send(&state.client, request).await?).await?["data"].take())
}

#[tauri::command]
async fn session_messages(state: State<'_, AppState>, id: String) -> Result<Value, Error> {
    if state.ssh() {
        let hermes = state.hermes().await?;
        let cwd = hermes.cwds.lock().unwrap().get(&id).cloned().unwrap_or_else(|| hermes.home.clone());
        let params = json!({ "sessionId": id, "cwd": cwd, "mcpServers": [] });
        let (loaded, replay) = hermes.collect(&id, "session/load", params).await?;
        if loaded.is_null() {
            return Err(Error::NotFound("Hermes has no such session".into()));
        }
        hermes.note_model(&id, &loaded);
        return Ok(json!(acp::history(&replay)));
    }
    let request = state.api(Method::GET, &["api", "sessions", &id, "messages"], &[])?;
    Ok(json_body(send(&state.client, request).await?).await?["data"].take())
}

/// Retitle: asks the model for a title from the transcript, saves it, and returns it. Hermes
/// has no endpoint for this, so the model is asked like any client would: a chat completion
/// over HTTP, `hermes chat` over SSH. Either way Hermes stores the question as a Session of its
/// own, which is deleted before the title is saved so it can't be holding that title.
#[tauri::command]
async fn retitle_session(state: State<'_, AppState>, id: String) -> Result<String, Error> {
    if state.acp_runs.lock().unwrap().values().any(|r| r.session == id) {
        // Loading the transcript over ACP would take the streaming reply's updates.
        return Err(Error::Invalid("Over SSH, a session can be retitled once its reply finishes".into()));
    }
    let messages = session_messages(state.clone(), id.clone()).await?;
    let prompt = title::prompt(messages.as_array().map_or(&[][..], Vec::as_slice))
        .ok_or_else(|| Error::Invalid("This session has no messages to title yet".into()))?;
    let no_title = || Error::Http("The model didn't reply with a usable title".into());
    if let Some(host) = state.ssh_host() {
        let profile = state.lock().profile.clone();
        let result = remote::chat_result(&remote::run(&host, remote::ask(profile.as_deref(), &prompt)).await?)?;
        if let Some(scratch) = result["session_id"].as_str().filter(|s| !s.is_empty() && *s != id) {
            let _ = remote::run(&host, remote::sessions_delete(profile.as_deref(), scratch)).await;
        }
        let title = title::clean(result["text"].as_str().unwrap_or_default()).ok_or_else(no_title)?;
        remote::run(&host, remote::sessions_rename(profile.as_deref(), &id, &title)).await?;
        return Ok(title);
    }
    let body = json!({ "messages": [{ "role": "user", "content": prompt }] });
    let request = state.api(Method::POST, &["v1", "chat", "completions"], &[])?.json(&body);
    let response = send(&state.slow_client, request).await?;
    let scratch = response.headers().get("X-Hermes-Session-Id").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let reply = json_body(response).await?["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_owned();
    if let Some(scratch) = scratch.filter(|s| *s != id) {
        let _ = send(&state.client, state.api(Method::DELETE, &["api", "sessions", &scratch], &[])?).await;
    }
    let title = title::clean(&reply).ok_or_else(no_title)?;
    let request = state.api(Method::PATCH, &["api", "sessions", &id], &[])?.json(&json!({ "title": title }));
    send(&state.client, request).await?;
    Ok(title)
}

#[tauri::command]
async fn delete_session(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    if state.ssh() {
        return Err(Error::Invalid("Hermes over SSH can't delete sessions".into()));
    }
    send(&state.client, state.api(Method::DELETE, &["api", "sessions", &id], &[])?).await?;
    Ok(())
}

#[derive(Serialize)]
struct RunStarted {
    run_id: String,
    session_id: String,
}

/// Creates a Run. Without a `session_id` the API server starts a new Session keyed by the run id.
#[tauri::command]
async fn start_run(
    state: State<'_, AppState>,
    session_id: Option<String>,
    text: String,
    files: Vec<attach::Source>,
    model: Option<picker::ModelChoice>,
    reasoning: Option<String>,
) -> Result<RunStarted, Error> {
    let reasoning = reasoning.map(|r| picker::valid_reasoning(&r)).transpose()?;
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
    let input = attach::build_input(&text, &files).map_err(Error::Invalid)?;
    if state.ssh() {
        let hermes = state.hermes().await?;
        let session = match session_id {
            Some(id) => id,
            None => {
                let created = hermes.request("session/new", json!({ "cwd": hermes.home, "mcpServers": [] })).await?;
                let id = created["sessionId"].as_str().ok_or_else(|| Error::Http("Session created without an id".into()))?;
                hermes.note_model(id, &created);
                id.to_owned()
            }
        };
        hermes.use_model(&session, model.as_ref()).await?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let run_id = format!("acp-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let run = AcpRun { session: session.clone(), prompt: Some(acp::prompt_blocks(input)) };
        state.acp_runs.lock().unwrap().insert(run_id.clone(), run);
        return Ok(RunStarted { run_id, session_id: session });
    }
    let body = picker::run_body(input, session_id.as_deref(), model.as_ref(), reasoning.as_deref());
    let request = state.api(Method::POST, &["v1", "runs"], &[])?.json(&body);
    let response = json_body(send(&state.client, request).await?).await?;
    let run_id = response["run_id"].as_str().ok_or_else(|| Error::Http("Run created without a run_id".into()))?;
    Ok(RunStarted { session_id: session_id.unwrap_or_else(|| run_id.to_owned()), run_id: run_id.to_owned() })
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamMsg {
    Event { data: Value },
    Dropped { message: String },
}

const TERMINAL_EVENTS: [&str; 4] = ["run.completed", "run.failed", "run.cancelled", "run.interrupted"];

/// Opens a Run's event stream after `last_seq` (-1 = from the start). A gone Run is `NotFound`,
/// which the UI uses to fall back to a fresh Run.
#[tauri::command]
async fn stream_run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    last_seq: i64,
    on_event: Channel<StreamMsg>,
) -> Result<(), Error> {
    if state.ssh() {
        return stream_acp(app, state, run_id, on_event).await;
    }
    let request = state
        .api(Method::GET, &["v1", "runs", &run_id, "events"], &[])?
        .header(header::ACCEPT, "text/event-stream")
        .header("Last-Event-ID", last_seq.to_string());
    let response = send(&state.client, request).await?;
    let mut streams = state.streams.lock().unwrap();
    let id = run_id.clone();
    let task = async_runtime::spawn(async move {
        pump(response, &on_event).await;
        app.state::<AppState>().streams.lock().unwrap().remove(&id);
    });
    if let Some(old) = streams.insert(run_id, task) {
        old.abort();
    }
    Ok(())
}

/// Forwards every event to the UI until a terminal one. Any other ending is a dropped stream.
async fn pump(mut response: Response, channel: &Channel<StreamMsg>) {
    let mut parser = sse::Parser::default();
    let message = loop {
        match response.chunk().await {
            Ok(Some(bytes)) => {
                for event in parser.push(&bytes) {
                    let Ok(mut data) = serde_json::from_str::<Value>(&event.data) else { continue };
                    // Runs frames carry the name inside the JSON; named `event:` frames are normalized to match.
                    if let (Some(name), Some(fields)) = (event.event, data.as_object_mut()) {
                        fields.entry("event").or_insert(name.into());
                    }
                    let terminal = data["event"].as_str().is_some_and(|e| TERMINAL_EVENTS.contains(&e));
                    if channel.send(StreamMsg::Event { data }).is_err() || terminal {
                        return;
                    }
                }
            }
            Ok(None) => break "The gateway closed the stream before the run finished".to_owned(),
            Err(e) if e.is_timeout() => {
                break format!("No data from the gateway for {} s", gateway::IDLE_TIMEOUT.as_secs())
            }
            Err(e) => break describe(&e),
        }
    };
    log::write("gateway", format!("run stream dropped: {message}"));
    let _ = channel.send(StreamMsg::Dropped { message });
}

/// Sends the Run's prompt and forwards its updates. There is no reattaching to an ACP prompt,
/// so a second call is `NotFound` and Retry starts a new Run in the same Session.
async fn stream_acp(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    run_id: String,
    channel: Channel<StreamMsg>,
) -> Result<(), Error> {
    let gone = || Error::NotFound("This run can't be resumed over SSH".into());
    let (session, prompt) = {
        let mut runs = state.acp_runs.lock().unwrap();
        let run = runs.get_mut(&run_id).ok_or_else(gone)?;
        (run.session.clone(), run.prompt.take().ok_or_else(gone)?)
    };
    let hermes = state.hermes().await?;
    let mut streams = state.streams.lock().unwrap();
    let id = run_id.clone();
    let task = async_runtime::spawn(async move {
        let mut updates = hermes.subscribe(&session);
        let request = hermes.request("session/prompt", json!({ "sessionId": session, "prompt": prompt }));
        tokio::pin!(request);
        let mut tools = HashMap::new();
        let forward = |update: Value, tools: &mut HashMap<String, String>| {
            acp::translate(&update, tools).into_iter().all(|data| channel.send(StreamMsg::Event { data }).is_ok())
        };
        let result = loop {
            tokio::select! {
                Some(update) = updates.recv() => if !forward(update, &mut tools) { break None },
                result = &mut request => break Some(result),
            }
        };
        if let Some(result) = result {
            // Updates sent before the answer may still be queued behind it.
            while let Ok(update) = updates.try_recv() {
                forward(update, &mut tools);
            }
            let _ = match result {
                Err(Error::Unreachable(message)) => channel.send(StreamMsg::Dropped { message }),
                result => channel.send(StreamMsg::Event { data: acp::finished(result) }),
            };
        }
        hermes.unsubscribe(&session);
        let state = app.state::<AppState>();
        state.streams.lock().unwrap().remove(&id);
        state.acp_runs.lock().unwrap().remove(&id);
    });
    if let Some(old) = streams.insert(run_id, task) {
        old.abort();
    }
    Ok(())
}

/// Stops the Turn at once by dropping its stream, then asks the server to stop the Run.
#[tauri::command]
async fn stop_run(state: State<'_, AppState>, run_id: String) -> Result<(), Error> {
    if let Some(task) = state.streams.lock().unwrap().remove(&run_id) {
        task.abort();
    }
    if state.ssh() {
        let run = state.acp_runs.lock().unwrap().remove(&run_id);
        if let Some(run) = run {
            let hermes = state.hermes().await?;
            hermes.unsubscribe(&run.session);
            hermes.cancel(&run.session).await;
        }
        return Ok(());
    }
    if state.lock().run_stop {
        let request = state.api(Method::POST, &["v1", "runs", &run_id, "stop"], &[])?;
        let client = state.client.clone();
        async_runtime::spawn(async move { send(&client, request).await });
    }
    Ok(())
}

// ---- Board and Skills: Dashboard routes, or `ssh host hermes …` (ADR 0002) ----

#[tauri::command]
async fn kanban_board(state: State<'_, AppState>) -> Result<Vec<work::Task>, Error> {
    if let Some(host) = state.ssh_host() {
        return Ok(work::board_from_cli(remote::json(&remote::run(&host, remote::kanban_list()).await?)?));
    }
    let request = state.kanban(Method::GET, &["board"], &[("include_archived", "true")])?;
    match state.dashboard_json(request).await {
        Err(Error::NotFound(_)) => Err(Error::NotFound("The Kanban plugin is disabled on this Hermes".into())),
        body => Ok(work::board_from_dashboard(body?)),
    }
}

#[tauri::command]
async fn kanban_assignees(state: State<'_, AppState>) -> Result<Vec<String>, Error> {
    let body = match state.ssh_host() {
        Some(host) => remote::json(&remote::run(&host, remote::kanban_assignees()).await?)?,
        None => state.dashboard_json(state.kanban(Method::GET, &["assignees"], &[])?).await?,
    };
    Ok(work::assignee_names(body))
}

#[tauri::command]
async fn kanban_task(state: State<'_, AppState>, id: String) -> Result<work::Detail, Error> {
    let body = match state.ssh_host() {
        Some(host) => remote::json(&remote::run(&host, remote::kanban_show(&id)).await?)?,
        None => state.dashboard_json(state.kanban(Method::GET, &["tasks", &id], &[])?).await?,
    };
    work::detail_from(body).map_err(|e| Error::Http(format!("Unreadable task: {e}")))
}

#[tauri::command]
async fn kanban_create(state: State<'_, AppState>, task: work::NewTask) -> Result<(), Error> {
    if task.title.trim().is_empty() {
        return Err(Error::Invalid("A task needs a title".into()));
    }
    match state.ssh_host() {
        Some(host) => remote::run(&host, remote::kanban_create(&task)).await.map(drop),
        None => state.dashboard_json(state.kanban(Method::POST, &["tasks"], &[])?.json(&task)).await.map(drop),
    }
}

#[tauri::command]
async fn kanban_comment(state: State<'_, AppState>, id: String, text: String) -> Result<(), Error> {
    if text.trim().is_empty() {
        return Err(Error::Invalid("A comment can't be empty".into()));
    }
    match state.ssh_host() {
        Some(host) => remote::run(&host, remote::kanban_comment(&id, &text)).await.map(drop),
        None => {
            let request = state.kanban(Method::POST, &["tasks", &id, "comments"], &[])?.json(&json!({ "body": text }));
            state.dashboard_json(request).await.map(drop)
        }
    }
}

/// One Dispatch pass (or a preview of one), capped at Hermes's default of 8 spawns.
#[tauri::command]
async fn kanban_dispatch(state: State<'_, AppState>, dry_run: bool) -> Result<Value, Error> {
    if let Some(host) = state.ssh_host() {
        return remote::json(&remote::run(&host, remote::kanban_dispatch(dry_run)).await?);
    }
    let query = [("dry_run", if dry_run { "true" } else { "false" }), ("max", remote::DISPATCH_MAX)];
    state.dashboard_json(state.kanban(Method::POST, &["dispatch"], &query)?).await
}

#[tauri::command]
async fn skills_list(state: State<'_, AppState>) -> Result<Vec<work::Skill>, Error> {
    if let Some(host) = state.ssh_host() {
        let (skills, paths) = work::skills_from_ssh(&remote::run(&host, remote::skills_list()).await?);
        *state.skill_paths.lock().unwrap() = paths;
        return Ok(skills);
    }
    Ok(work::skills_from_dashboard(state.dashboard_json(state.dashboard(Method::GET, &["api", "skills"], &[])?).await?))
}

/// The Skill's raw SKILL.md.
#[tauri::command]
async fn skill_content(state: State<'_, AppState>, name: String) -> Result<String, Error> {
    if let Some(host) = state.ssh_host() {
        let path = state.skill_paths.lock().unwrap().get(&name).cloned();
        let path = path.ok_or_else(|| Error::NotFound(format!("No skill named {name}")))?;
        return remote::run(&host, remote::skill_content(&path)).await;
    }
    let request = state.dashboard(Method::GET, &["api", "skills", "content"], &[("name", &name)])?;
    Ok(state.dashboard_json(request).await?["content"].as_str().unwrap_or_default().to_owned())
}

/// Enables or disables a Skill for the active Profile on every platform.
#[tauri::command]
async fn skill_toggle(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), Error> {
    if work::is_essential(&name) {
        return Err(Error::Invalid(format!("{name} is essential and can't be disabled")));
    }
    let Some(host) = state.ssh_host() else {
        let request = state.dashboard(Method::PUT, &["api", "skills", "toggle"], &[])?.json(&json!({ "name": name, "enabled": enabled }));
        return state.dashboard_json(request).await.map(drop);
    };
    // ponytail: read-modify-write; a toggle from another client in between is lost. Fine for one user.
    // An unreadable answer must stop here: writing back a guessed list would re-enable every other Skill.
    let current = remote::json(&remote::run(&host, remote::skills_disabled()).await?)?;
    let mut disabled: Vec<String> = match current {
        Value::Null => Vec::new(),
        list => serde_json::from_value(list).map_err(|e| Error::Http(format!("Unreadable skills.disabled: {e}")))?,
    };
    disabled.retain(|n| *n != name);
    if !enabled {
        disabled.push(name);
    }
    remote::run(&host, remote::skills_set_disabled(&disabled)).await.map(drop)
}

/// The Client log so far; new lines arrive on `on_line`.
#[tauri::command]
fn client_log(on_line: Channel<log::Line>) -> Vec<log::Line> {
    log::subscribe(on_line)
}

#[tauri::command]
fn clear_client_log() {
    log::clear();
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = load_settings(&settings_path);
            let ssh_host = settings.as_ref().and_then(|s| load_ssh_host(s, &settings_path));
            let gateway = settings.as_ref().filter(|_| ssh_host.is_none()).and_then(|s| load_gateway(s, &settings_path));
            app.manage(AppState {
                client: gateway::client(),
                slow_client: gateway::slow_client(),
                settings_path,
                inner: Mutex::new(Inner { gateway, ssh_host, ..Inner::default() }),
                streams: Mutex::default(),
                dropped: Mutex::default(),
                hermes: tokio::sync::Mutex::default(),
                acp_runs: Mutex::default(),
                skill_paths: Mutex::default(),
            });
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                window.with_webview(|webview| {
                    use webkit2gtk::{SettingsExt, WebViewExt};
                    if let Some(settings) = WebViewExt::settings(&webview.inner()) {
                        settings.set_enable_media_stream(true);
                    }
                })?;
            }
            Ok(())
        })
        // Dictation is the only thing that asks; everything else keeps the platform default.
        // ponytail: the mic is allowed for any origin, fine while the webview loads only bundled
        // pages; check the requesting origin if it ever shows remote content.
        .on_permission_request(|_, kind| match kind {
            PermissionKind::Microphone => PermissionResponse::Allow,
            _ => PermissionResponse::Default,
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                window.state::<AppState>().dropped.lock().unwrap().extend(paths.iter().cloned());
            }
        })
        .invoke_handler(tauri::generate_handler![
            init,
            configure,
            status,
            sign_in,
            check_sign_in,
            capabilities,
            set_api_key,
            finish_pairing,
            configure_ssh,
            connect_ssh,
            list_profiles,
            set_profile,
            list_models,
            transcribe,
            answer_permission,
            list_sessions,
            session_messages,
            delete_session,
            retitle_session,
            start_run,
            stream_run,
            stop_run,
            kanban_board,
            kanban_assignees,
            kanban_task,
            kanban_create,
            kanban_comment,
            kanban_dispatch,
            skills_list,
            skill_content,
            skill_toggle,
            client_log,
            clear_client_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hermes Desktop");
}

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
