//! The Board and Skills in one shape, whichever way Hermes is reached (Dashboard or SSH).

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
}
