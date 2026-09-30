//! The Board, Skills, and MCP servers in one shape, whichever way Hermes is reached (Dashboard or SSH).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Skills Hermes never lets anyone disable (`agent/skill_utils.py` ESSENTIAL_SKILLS).
const ESSENTIAL: [&str; 1] = ["hermes-agent"];

/// Separates `hermes config get skills.disabled --json` from the SKILL.md frontmatter that
/// follows it in the SSH listing; each file's frontmatter starts with `@@@ <relative path>`.
pub const SKILLS_MARK: &str = "@@@skills";
pub const FILE_MARK: &str = "@@@ ";

/// The Task fields the client shows; the Dashboard and `hermes kanban … --json` name them alike.
#[derive(Serialize, Deserialize, Debug)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    pub status: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize)]
pub struct Detail {
    pub task: Task,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

fn tasks(list: &Value) -> Vec<Task> {
    let list = list.as_array().map(Vec::as_slice).unwrap_or_default();
    let tasks: Vec<Task> = list.iter().filter_map(|t| serde_json::from_value(t.clone()).ok()).collect();
    if tasks.len() < list.len() {
        crate::log::write("kanban", format!("skipped {} unreadable tasks", list.len() - tasks.len()));
    }
    tasks
}

/// `GET /api/plugins/kanban/board`: Tasks grouped by column.
pub fn board_from_dashboard(body: Value) -> Vec<Task> {
    let columns = body["columns"].as_array().map(Vec::as_slice).unwrap_or_default();
    columns.iter().flat_map(|c| tasks(&c["tasks"])).collect()
}

/// `hermes kanban list --json`: already a flat list.
pub fn board_from_cli(body: Value) -> Vec<Task> {
    tasks(&body)
}

/// `GET /tasks/{id}` or `hermes kanban show --json`.
pub fn detail_from(body: Value) -> Result<Detail, serde_json::Error> {
    serde_json::from_value(body)
}

/// `hermes kanban assignees --json` is a list; the Dashboard wraps it in `assignees`.
pub fn assignee_names(body: Value) -> Vec<String> {
    let list = if body.is_array() { &body } else { &body["assignees"] };
    let list = list.as_array().map(Vec::as_slice).unwrap_or_default();
    list.iter().filter_map(|a| a["name"].as_str().or(a.as_str()).map(Into::into)).collect()
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub enabled: bool,
    pub essential: bool,
}

pub fn is_essential(name: &str) -> bool {
    ESSENTIAL.contains(&name)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

/// `GET /api/skills`.
pub fn skills_from_dashboard(body: Value) -> Vec<Skill> {
    let list = body.as_array().cloned().unwrap_or_default();
    list.iter()
        .map(|s| Skill {
            name: text(&s["name"]),
            description: text(&s["description"]),
            category: s["category"].as_str().map(Into::into),
            enabled: s["enabled"].as_bool().unwrap_or(true),
            essential: is_essential(s["name"].as_str().unwrap_or_default()),
        })
        .collect()
}

/// The SSH listing script's output: the Skills, and each one's SKILL.md path under the
/// Profile's skills directory (for reading it later).
pub fn skills_from_ssh(output: &str) -> (Vec<Skill>, HashMap<String, String>) {
    let (disabled, files) = output.split_once(SKILLS_MARK).unwrap_or(("", output));
    // Hermes may print chatter before the value; the JSON is the last line.
    let disabled: HashSet<String> =
        disabled.trim().lines().last().and_then(|l| serde_json::from_str(l).ok()).unwrap_or_default();
    let mut skills = Vec::new();
    let mut paths = HashMap::new();
    for block in files.split(&format!("\n{FILE_MARK}")).skip(1) {
        let (path, frontmatter) = block.split_once('\n').unwrap_or((block, ""));
        let path = path.trim_start_matches("./");
        let segments: Vec<&str> = path.split('/').collect();
        let fields = frontmatter_fields(frontmatter);
        let name = fields.get("name").cloned().unwrap_or_else(|| segments[segments.len().saturating_sub(2)].to_owned());
        // Hermes: a category only when the path is at least `category/skill/SKILL.md`.
        let category = (segments.len() >= 3).then(|| segments[0].to_owned());
        // Hermes keeps the first Skill of a name; so does this.
        if paths.contains_key(&name) {
            continue;
        }
        paths.insert(name.clone(), path.to_owned());
        skills.push(Skill {
            enabled: !disabled.contains(&name) || is_essential(&name),
            essential: is_essential(&name),
            description: fields.get("description").cloned().unwrap_or_default(),
            category,
            name,
        });
    }
    (skills, paths)
}

/// Top-level `key: value` pairs of YAML frontmatter: plain, quoted, or `>`/`|` blocks
/// (joined into one line). Enough for name and description; not a YAML parser.
fn frontmatter_fields(frontmatter: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let mut lines = frontmatter.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((key, value)) = line.split_once(':').filter(|_| !line.starts_with([' ', '\t', '#'])) else { continue };
        let value = value.trim();
        let value = if value.starts_with(['>', '|']) {
            let mut block = Vec::new();
            while let Some(next) = lines.next_if(|l| l.starts_with([' ', '\t']) || l.is_empty()) {
                block.push(next.trim());
            }
            block.join(" ").trim().to_owned()
        } else {
            value.trim_matches(|c| c == '"' || c == '\'').to_owned()
        };
        fields.insert(key.trim().to_owned(), value);
    }
    fields
}

/// An MCP server as the MCP servers view shows it. Env values never leave Rust: only their names.
#[derive(Serialize, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub auth: Option<String>,
    pub enabled: bool,
    /// Which of its tools Hermes registers, in words.
    pub tools: String,
    /// `config` (the Profile's config.yaml) or `plugin`.
    pub source: String,
    pub plugin: Option<String>,
}

/// A list's entries as text, the way Hermes `str()`s them; nulls and nested values are dropped.
fn strings(v: &Value) -> Vec<String> {
    let text = |s: &Value| match s {
        Value::String(s) => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(s.to_string()),
        _ => None,
    };
    v.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(text).collect()
}

fn env_names(v: &Value) -> Vec<String> {
    let mut names: Vec<String> = v.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    names.sort();
    names
}

/// `tools.include` wins over `tools.exclude`; either may be one name or a list, anything else is
/// ignored (tools/mcp_tool_registration.py `_make_tool_filter`).
fn tool_selection(tools: &Value) -> String {
    let names = |v: &Value| match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(_) => Some(strings(v).join(", ")),
        _ => None,
    };
    match (names(&tools["include"]), names(&tools["exclude"])) {
        (Some(include), _) if include.is_empty() => "No tools".into(),
        (Some(include), _) => format!("Only {include}"),
        (None, Some(exclude)) if !exclude.is_empty() => format!("All tools except {exclude}"),
        (None, _) => "All tools".into(),
    }
}

/// `GET /api/mcp/servers` (hermes_cli/web_server_mcp.py `_mcp_server_summary`).
pub fn mcp_from_dashboard(body: Value) -> Vec<McpServer> {
    let list = body["servers"].as_array().cloned().unwrap_or_default();
    list.iter()
        .map(|s| McpServer {
            name: text(&s["name"]),
            transport: s["transport"].as_str().unwrap_or("unknown").into(),
            url: s["url"].as_str().map(Into::into),
            command: s["command"].as_str().map(Into::into),
            args: strings(&s["args"]),
            env: env_names(&s["env"]),
            auth: s["auth"].as_str().map(Into::into),
            enabled: s["enabled"].as_bool().unwrap_or(true),
            tools: tool_selection(&s["tools"]),
            source: s["source"].as_str().unwrap_or("config").into(),
            plugin: s["plugin"].as_str().map(Into::into),
        })
        .collect()
}

/// `hermes config get mcp_servers --json`: the raw config map, summarized the way the Dashboard
/// does it. Servers that plugins provide aren't in it.
pub fn mcp_from_config(body: Value) -> Vec<McpServer> {
    let map = body.as_object().cloned().unwrap_or_default();
    let mut servers: Vec<McpServer> = map
        .into_iter()
        .filter(|(_, cfg)| cfg.is_object())
        .map(|(name, cfg)| {
            let url = cfg["url"].as_str().filter(|u| !u.is_empty()).map(String::from);
            let command = cfg["command"].as_str().filter(|c| !c.is_empty()).map(String::from);
            let authorization = cfg["headers"].as_object().is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("authorization")));
            McpServer {
                transport: if url.is_some() { "http" } else if command.is_some() { "stdio" } else { "unknown" }.into(),
                args: strings(&cfg["args"]),
                env: env_names(&cfg["env"]),
                auth: cfg["auth"].as_str().map(Into::into).or(authorization.then(|| "header".into())),
                enabled: enabled(&cfg["enabled"]),
                tools: tool_selection(&cfg["tools"]),
                source: "config".into(),
                plugin: None,
                url,
                command,
                name,
            }
        })
        .collect();
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    servers
}

/// Hermes's `mcp_server_enabled`: absent, null, or unreadable means on.
fn enabled(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !["false", "0", "no", "off"].contains(&s.trim().to_lowercase().as_str()),
        _ => true,
    }
}

/// `POST /api/mcp/servers/{name}/test`: a live probe's tools, or why it failed. Over SSH there
/// are no prompt or resource counts.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct McpTest {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tools: Vec<McpTool>,
    #[serde(default)]
    pub prompts: Option<i64>,
    #[serde(default)]
    pub resources: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// `hermes mcp test`'s stdout. A failure is its one `✗ Connection failed …` line (the lines
/// before it can show part of an auth header). A success is the tool table under
/// "Tools discovered: N", one `    name   description` line each (hermes_cli/mcp_config.py `_print_tools`).
pub fn mcp_test_from_cli(out: &str) -> McpTest {
    if let Some(why) = out.lines().find_map(|l| l.trim().strip_prefix("✗ ")) {
        return McpTest { ok: false, error: Some(why.into()), tools: Vec::new(), prompts: None, resources: None };
    }
    if !out.contains("Tools discovered:") {
        // Not a connection failure but not a result either: hermes exited early (config error,
        // broken launcher). Its reason went to stderr; the last stdout line is the best hint.
        let last = out.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("hermes mcp test printed no result");
        return McpTest { ok: false, error: Some(last.into()), tools: Vec::new(), prompts: None, resources: None };
    }
    let tools = out
        .lines()
        .skip_while(|l| !l.contains("Tools discovered:"))
        .skip(1)
        .filter_map(|l| {
            let (name, description) = l.trim().split_once(char::is_whitespace).unwrap_or((l.trim(), ""));
            let description = description.trim();
            (!name.is_empty()).then(|| McpTool { name: name.into(), description: (!description.is_empty()).then(|| description.into()) })
        })
        .collect();
    McpTest { ok: true, error: None, tools, prompts: None, resources: None }
}

/// What the New task form sends. Serializes straight into the Dashboard's `POST /tasks` body.
#[derive(Deserialize, Serialize)]
pub struct NewTask {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn skill(name: &str, description: &str, category: Option<&str>, enabled: bool, essential: bool) -> Skill {
        Skill { name: name.into(), description: description.into(), category: category.map(Into::into), enabled, essential }
    }

    fn task(id: &str, status: &str, assignee: Option<&str>) -> Value {
        json!({ "id": id, "title": format!("Task {id}"), "body": "Body", "assignee": assignee, "status": status,
                "priority": 5, "tenant": null, "created_at": 1784649416, "workspace_kind": "scratch", "skills": [] })
    }

    fn ids(board: &[Task]) -> Vec<(&str, &str)> {
        board.iter().map(|t| (t.id.as_str(), t.status.as_str())).collect()
    }

    #[test]
    fn dashboard_board_columns_flatten_into_tasks() {
        let body = json!({
            "columns": [
                { "name": "todo", "tasks": [task("t_1", "todo", Some("daedalus"))] },
                { "name": "running", "tasks": [] },
                { "name": "archived", "tasks": [task("t_2", "archived", None)] },
            ],
            "tenants": ["acme"], "assignees": ["daedalus"], "latest_event_id": 9,
        });
        let board = board_from_dashboard(body);
        assert_eq!(ids(&board), [("t_1", "todo"), ("t_2", "archived")]);
        assert_eq!(board[0].assignee.as_deref(), Some("daedalus"));
        assert_eq!(board[0].priority, 5);
    }

    #[test]
    fn cli_task_list_is_the_board() {
        let board = board_from_cli(json!([task("t_9", "blocked", None), task("t_8", "done", Some("forge"))]));
        assert_eq!(ids(&board), [("t_9", "blocked"), ("t_8", "done")]);
        assert_eq!(board[1].title, "Task t_8");
    }

    #[test]
    fn task_detail_keeps_comments_from_either_transport() {
        let dashboard = json!({ "task": task("t_1", "todo", None), "events": [], "runs": [],
            "comments": [{ "id": 1, "task_id": "t_1", "author": "dashboard", "body": "Looks good", "created_at": 10 }] });
        let cli = json!({ "task": task("t_1", "todo", None), "events": [], "runs": [],
            "comments": [{ "author": "dashboard", "body": "Looks good", "created_at": 10 }] });
        for body in [dashboard, cli] {
            let detail = detail_from(body).unwrap();
            assert_eq!(detail.task.id, "t_1");
            assert_eq!(detail.comments, [Comment { author: "dashboard".into(), body: "Looks good".into(), created_at: 10 }]);
        }
    }

    #[test]
    fn assignee_names_from_either_transport() {
        let cli = json!([{ "name": "aegis", "on_disk": true, "counts": {} }, { "name": "forge", "on_disk": false }]);
        assert_eq!(assignee_names(cli), ["aegis", "forge"]);
        assert_eq!(assignee_names(json!({ "assignees": [{ "name": "aegis" }, "forge"] })), ["aegis", "forge"]);
    }

    #[test]
    fn dashboard_skills_become_skills() {
        let body = json!([
            { "name": "unslop", "description": "Cut AI tells", "category": null, "enabled": true, "usage": 3, "provenance": "agent" },
            { "name": "codex", "description": "Delegate coding", "category": "autonomous-ai-agents", "enabled": false },
            { "name": "hermes-agent", "description": "Use Hermes", "category": "autonomous-ai-agents", "enabled": true },
        ]);
        assert_eq!(
            skills_from_dashboard(body),
            [
                skill("unslop", "Cut AI tells", None, true, false),
                skill("codex", "Delegate coding", Some("autonomous-ai-agents"), false, false),
                skill("hermes-agent", "Use Hermes", Some("autonomous-ai-agents"), true, true),
            ]
        );
    }

    #[test]
    fn ssh_listing_reads_frontmatter_category_and_disabled_state() {
        let output = "hermes: chatter\n[\"codex\", \"hermes-agent\"]\n@@@skills\n\
            @@@ ./unslop/SKILL.md\nname: unslop\ndescription: Cut AI tells: all of them\n\
            @@@ ./autonomous-ai-agents/codex/SKILL.md\nname: \"codex\"\ndescription: 'Delegate coding'\nversion: 1\n\
            @@@ ./devops/ad-cs/SKILL.md\ndescription: >-\n  Request AD\n  certificates\ntags: [x]\n\
            @@@ ./autonomous-ai-agents/hermes-agent/SKILL.md\nname: hermes-agent\ndescription: |\n  Use Hermes\n";
        let (skills, paths) = skills_from_ssh(output);
        assert_eq!(
            skills,
            [
                skill("unslop", "Cut AI tells: all of them", None, true, false),
                skill("codex", "Delegate coding", Some("autonomous-ai-agents"), false, false),
                skill("ad-cs", "Request AD certificates", Some("devops"), true, false),
                skill("hermes-agent", "Use Hermes", Some("autonomous-ai-agents"), true, true),
            ]
        );
        assert_eq!(paths["codex"], "autonomous-ai-agents/codex/SKILL.md");
    }

    #[test]
    fn ssh_listing_with_nothing_disabled_or_installed() {
        assert!(skills_from_ssh("null\n@@@skills\n").0.is_empty());
        let (skills, _) = skills_from_ssh("@@@skills\n@@@ ./a/SKILL.md\nname: a\n");
        assert_eq!(skills, [skill("a", "", None, true, false)]);
    }

    fn mcp(name: &str, transport: &str, enabled: bool, tools: &str, source: &str) -> (String, String, bool, String, String) {
        (name.into(), transport.into(), enabled, tools.into(), source.into())
    }

    fn mcp_rows(servers: &[McpServer]) -> Vec<(String, String, bool, String, String)> {
        servers.iter().map(|s| (s.name.clone(), s.transport.clone(), s.enabled, s.tools.clone(), s.source.clone())).collect()
    }

    #[test]
    fn dashboard_mcp_servers_keep_env_names_only() {
        let body = json!({ "servers": [
            { "name": "github", "transport": "stdio", "url": null, "command": "npx", "args": ["-y", "@mcp/github"],
              "env": { "GITHUB_TOKEN": "ghp_...abcd", "A": "" }, "auth": null, "enabled": true,
              "tools": { "exclude": ["delete_repo"] }, "source": "config", "plugin": null },
            { "name": "linear", "transport": "http", "url": "https://mcp.linear.app/sse", "command": null, "args": [],
              "env": {}, "auth": "oauth", "enabled": false, "tools": null, "source": "plugin", "plugin": "linear-kit" },
        ] });
        let servers = mcp_from_dashboard(body);
        assert_eq!(mcp_rows(&servers), [
            mcp("github", "stdio", true, "All tools except delete_repo", "config"),
            mcp("linear", "http", false, "All tools", "plugin"),
        ]);
        assert_eq!(servers[0].env, ["A", "GITHUB_TOKEN"]);
        assert_eq!(servers[0].args, ["-y", "@mcp/github"]);
        assert_eq!(servers[1].url.as_deref(), Some("https://mcp.linear.app/sse"));
        assert_eq!(servers[1].plugin.as_deref(), Some("linear-kit"));
        assert!(!format!("{servers:?}").contains("ghp_"));
    }

    #[test]
    fn config_mcp_servers_are_summarized_like_the_dashboard() {
        let body = json!({
            "zed": { "url": "https://z/mcp", "headers": { "Authorization": "Bear...1234" }, "tools": { "include": ["a", "b"], "exclude": ["c"] } },
            "fs": { "command": "mcp-fs", "args": ["/srv"], "env": { "SECRET": "plain-value" }, "enabled": "off", "tools": { "include": [] } },
            "odd": { "enabled": 0, "tools": { "include": "one" } },
            "junk": "not a server",
        });
        let servers = mcp_from_config(body);
        assert_eq!(mcp_rows(&servers), [
            mcp("fs", "stdio", false, "No tools", "config"),
            mcp("odd", "unknown", false, "Only one", "config"),
            mcp("zed", "http", true, "Only a, b", "config"),
        ]);
        assert_eq!(servers[2].auth.as_deref(), Some("header"));
        assert_eq!(servers[0].env, ["SECRET"]);
        assert!(!format!("{servers:?}").contains("plain-value"));
        assert!(mcp_from_config(Value::Null).is_empty());
    }

    #[test]
    fn empty_or_unusable_tool_filters_select_every_tool() {
        // Hermes's _make_tool_filter: an empty exclude removes nothing; an include that isn't a name or list is ignored.
        let body = json!({
            "a": { "command": "x", "tools": { "exclude": [] } },
            "b": { "command": "x", "tools": { "include": 3, "exclude": "c" } },
        });
        let tools: Vec<String> = mcp_from_config(body).into_iter().map(|s| s.tools).collect();
        assert_eq!(tools, ["All tools", "All tools except c"]);
    }

    #[test]
    fn mcp_probe_results_parse() {
        let ok: McpTest = serde_json::from_value(json!({ "ok": true, "prompts": 2, "resources": 0,
            "tools": [{ "name": "search", "description": "Search issues", "schema_chars": 412 }, { "name": "bare", "description": null }] })).unwrap();
        assert!(ok.ok && ok.error.is_none() && ok.prompts == Some(2));
        assert_eq!(ok.tools, [
            McpTool { name: "search".into(), description: Some("Search issues".into()) },
            McpTool { name: "bare".into(), description: None },
        ]);
        let failed: McpTest = serde_json::from_value(json!({ "ok": false, "error": "Connection refused", "tools": [] })).unwrap();
        assert_eq!((failed.ok, failed.error.as_deref(), failed.tools.len()), (false, Some("Connection refused"), 0));
    }

    #[test]
    fn cli_test_output_lists_the_tools_after_the_count() {
        // `hermes mcp test` on a pipe: no colors, descriptions cut at 55 characters.
        let out = "\n  Testing 'learn'...\n  Transport: HTTP → https://l/mcp\n    Authorization: Bear...1234\n  ✓ Connected (1725ms)\n  ✓ Tools discovered: 2\n\n    microsoft_docs_search                Search official Microsoft/Azure documentation to find t...\n    bare\n\n";
        let test = mcp_test_from_cli(out);
        assert!(test.ok && test.error.is_none() && test.prompts.is_none());
        assert_eq!(test.tools, [
            McpTool { name: "microsoft_docs_search".into(), description: Some("Search official Microsoft/Azure documentation to find t...".into()) },
            McpTool { name: "bare".into(), description: None },
        ]);
        assert!(mcp_test_from_cli("  ✓ Connected (3ms)\n  ✓ Tools discovered: 0\n\n").tools.is_empty());
        // No result at all (hermes died before testing) is a failure, never "Connected, 0 tools".
        assert!(!mcp_test_from_cli("").ok);
        assert_eq!(mcp_test_from_cli("  Testing 'x'...\n").error.as_deref(), Some("Testing 'x'..."));
    }

    #[test]
    fn a_failed_cli_test_reports_only_the_failure_line() {
        // The auth line carries part of a header value; it must not reach the reason.
        let out = "\n  Testing 'learn'...\n  Transport: HTTP → https://l/mcp\n    Authorization: Bear...1234\n  ✗ Connection failed (0.4s): Connection refused\n  Check the server is running and the URL/command in its config, then run: hermes mcp test learn\n\n";
        let test = mcp_test_from_cli(out);
        assert_eq!((test.ok, test.error.as_deref()), (false, Some("Connection failed (0.4s): Connection refused")));
        assert!(test.tools.is_empty());
    }

    #[test]
    fn non_string_args_and_filter_entries_are_kept() {
        // Hermes str()s both, so a YAML `- 8080` is an argument and `include: [123]` names a tool.
        let servers = mcp_from_config(json!({ "a": { "command": "srv", "args": ["--port", 8080], "tools": { "include": [123] } } }));
        assert_eq!(servers[0].args, ["--port", "8080"]);
        assert_eq!(servers[0].tools, "Only 123");
    }
}
