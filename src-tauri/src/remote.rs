//! One-shot `ssh <host> hermes …` commands for the Board, Skills, and MCP servers (ADR 0002), separate from
//! the long-lived `hermes acp` connection. Every argument reaches the remote POSIX shell, so
//! each one is single-quoted; Task bodies go on stdin, never on the command line.

use crate::gateway::Error;
use crate::work::NewTask;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// A remote command: `summary` is all the Client log ever sees of it.
pub struct Cmd {
    pub summary: &'static str,
    script: String,
    pub stdin: Option<String>,
}

impl Cmd {
    /// What ssh runs remotely. The installer puts `hermes` in ~/.local/bin, which
    /// non-interactive SSH shells often leave off PATH.
    pub fn remote_line(&self) -> String {
        format!("PATH=\"$HOME/.local/bin:$PATH\"; {}", self.script)
    }
}

/// Runs one command on `host` and returns its stdout. The Client log gets its summary and
/// exit code only.
pub async fn run(host: &str, cmd: Cmd) -> Result<String, Error> {
    // Tests point this at a shim that runs the remote command locally (see mock/fake-ssh).
    let ssh = std::env::var_os("HERMES_DESKTOP_SSH").unwrap_or_else(|| "ssh".into());
    let mut child = Command::new(ssh)
        .args(["-T", "-o", "BatchMode=yes", "--", host, &cmd.remote_line()])
        .stdin(if cmd.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            crate::log::write("ssh", format!("{} -> could not run ssh: {e}", cmd.summary));
            Error::Unreachable(format!("Could not run ssh: {e}"))
        })?;
    if let (Some(text), Some(mut stdin)) = (cmd.stdin, child.stdin.take()) {
        // Dropping stdin afterwards is the EOF `--body-file -` waits for.
        let _ = stdin.write_all(text.as_bytes()).await;
    }
    let output = match tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            crate::log::write("ssh", format!("{} -> ssh failed: {e}", cmd.summary));
            return Err(Error::Unreachable(format!("ssh failed: {e}")));
        }
        Err(_) => {
            crate::log::write("ssh", format!("{} -> no answer within 60 s", cmd.summary));
            return Err(Error::Unreachable(format!("No answer from {host} within 60 s")));
        }
    };
    let code = output.status.code().map_or("killed".to_owned(), |c| c.to_string());
    crate::log::write("ssh", format!("{} -> exit {code}", cmd.summary));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let why = last_lines(if stderr.trim().is_empty() { &stdout } else { &stderr });
    // ssh itself exits 255 when it can't connect or authenticate.
    Err(match output.status.code() {
        Some(255) => Error::Unreachable(format!("ssh: {why}")),
        _ => Error::Http(format!("{} failed: {why}", cmd.summary)),
    })
}

fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(3)..].join("\n")
}

/// The JSON a `--json` command printed, skipping any chatter Hermes or the login shell printed
/// first: the first line from which the rest parses. `null` is what an unset config key prints.
pub fn json(stdout: &str) -> Result<Value, Error> {
    let starts = std::iter::once(0).chain(stdout.match_indices('\n').map(|(i, _)| i + 1));
    starts
        .filter(|&i| stdout[i..].starts_with(['{', '[']) || stdout[i..].trim() == "null")
        .find_map(|i| serde_json::from_str(&stdout[i..]).ok())
        .ok_or_else(|| Error::Http("hermes printed no readable JSON".into()))
}

/// POSIX single-quoting: the shell passes the result through as exactly one literal argument.
pub fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// `hermes <words> <args>`, every arg quoted. User-supplied positionals go after `--`, and
/// option values are joined as `--flag=value`, so nothing typed can be read as an option.
fn hermes(summary: &'static str, args: &[String]) -> Cmd {
    let words = summary.strip_prefix("hermes ").expect("summary names a hermes subcommand");
    let args: Vec<String> = args.iter().map(|a| quote(a)).collect();
    Cmd { summary, script: format!("exec hermes {words} {}", args.join(" ")), stdin: None }
}

pub fn kanban_list() -> Cmd {
    hermes("hermes kanban list", &["--archived".into(), "--json".into()])
}

pub fn kanban_assignees() -> Cmd {
    hermes("hermes kanban assignees", &["--json".into()])
}

pub fn kanban_show(id: &str) -> Cmd {
    hermes("hermes kanban show", &["--json".into(), "--".into(), id.into()])
}

pub fn kanban_create(new: &NewTask) -> Cmd {
    let mut args = vec!["--json".to_owned()];
    let body = new.body.clone().filter(|b| !b.is_empty());
    if body.is_some() {
        args.extend(["--body-file".into(), "-".into()]);
    }
    let set = |flag: &str, value: &Option<String>| value.as_ref().filter(|v| !v.is_empty()).map(|v| format!("--{flag}={v}"));
    args.extend(set("assignee", &new.assignee));
    args.extend(new.priority.map(|p| format!("--priority={p}")));
    args.extend(set("tenant", &new.tenant));
    args.extend(["--".into(), new.title.clone()]);
    Cmd { stdin: body, ..hermes("hermes kanban create", &args) }
}

pub fn kanban_comment(id: &str, text: &str) -> Cmd {
    hermes("hermes kanban comment", &["--".into(), id.into(), text.into()])
}

/// Spawns per Dispatch: Hermes's own default, so one click can't flood the host.
pub const DISPATCH_MAX: &str = "8";

pub fn kanban_dispatch(dry_run: bool) -> Cmd {
    let mut args = vec!["--max".to_owned(), DISPATCH_MAX.into(), "--json".into()];
    if dry_run {
        args.push("--dry-run".into());
    }
    hermes("hermes kanban dispatch", &args)
}

/// The active Profile's home: `hermes config path` names its config.yaml (last line, past any chatter).
const PROFILE_HOME: &str = "home=$(hermes config path | tail -n 1) && home=${home%/*} || exit 1";

/// The disabled list, then every SKILL.md's frontmatter under the Profile's skills directory,
/// skipping hidden directories as Hermes does (`.archive`, `.hub`, …). Parsed by
/// `work::skills_from_ssh`.
// ponytail: only the Profile's own skills dir; add `skills.external_dirs` when someone uses them.
pub fn skills_list() -> Cmd {
    let script = format!(
        "{PROFILE_HOME}; hermes config get skills.disabled --json 2>/dev/null; printf '\\n{}\\n'; \
         cd \"$home/skills\" 2>/dev/null || exit 0; \
         exec find -L . -name SKILL.md ! -path '*/.*/*' -exec awk \
         'FNR==1 {{ print \"{}\" FILENAME; if ($0 !~ /^---/) nextfile; next }} /^---/ {{ nextfile }} {{ print }}' {{}} +",
        crate::work::SKILLS_MARK,
        crate::work::FILE_MARK,
    );
    Cmd { summary: "hermes skills (list)", script, stdin: None }
}

/// One SKILL.md, by the path `skills_list` reported for it.
pub fn skill_content(path: &str) -> Cmd {
    let script = format!("{PROFILE_HOME}; exec cat -- \"$home/skills/\"{}", quote(path));
    Cmd { summary: "hermes skills (read)", script, stdin: None }
}

pub fn skills_disabled() -> Cmd {
    hermes("hermes config get", &["skills.disabled".into(), "--json".into()])
}

pub fn skills_set_disabled(names: &[String]) -> Cmd {
    hermes("hermes config set", &["skills.disabled".into(), serde_json::to_string(names).expect("serializable")])
}

/// `-p <profile> ` for a `hermes` command line, or nothing for the Gateway's default Profile.
fn profile_flag(profile: Option<&str>) -> String {
    profile.map(|p| format!("-p {} ", quote(p))).unwrap_or_default()
}

/// The Profile's `mcp_servers` config. Hermes masks secret-looking values under secret-looking
/// keys only, so env values are dropped in `work`.
pub fn mcp_servers(profile: Option<&str>) -> Cmd {
    let script = format!("exec hermes {}config get mcp_servers --json", profile_flag(profile));
    Cmd { summary: "hermes config get mcp_servers", script, stdin: None }
}

/// Connects to one MCP server and prints its tools. Hermes exits 1 when it can't connect; that
/// is a test result, so the script exits 0 and `work` reads the reason from stdout. Any other
/// failure (3 is "no such server") stays an error.
pub fn mcp_test(profile: Option<&str>, name: &str) -> Cmd {
    let script = format!("hermes {}mcp test -- {}; code=$?; [ $code -eq 1 ] && exit 0; exit $code", profile_flag(profile), quote(name));
    Cmd { summary: "hermes mcp test", script, stdin: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASTY: [&str; 7] = ["it's", "$(touch /tmp/pwned)", "`id`", "a\nb", "-rf", "\"q\" \\ ;|&", ""];

    /// Runs a command's remote line in a local `sh` whose `hermes` prints its argv NUL-separated.
    /// Runs `cmd` locally with `hermes` replaced by a shell script whose body is `fake`.
    fn run_with_fake_hermes(cmd: &Cmd, fake: &str) -> std::process::Output {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!("hd-remote-{}-{n}", std::process::id()));
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("hermes"), format!("#!/bin/sh\n{fake}\n")).unwrap();
        std::process::Command::new("chmod").arg("+x").arg(bin.join("hermes")).status().unwrap();
        std::process::Command::new("sh").arg("-c").arg(cmd.remote_line()).env("HOME", &home).output().unwrap()
    }

    fn argv_seen_by_hermes(cmd: &Cmd) -> Vec<String> {
        let text = String::from_utf8(run_with_fake_hermes(cmd, "printf '%s\\0' \"$@\"").stdout).unwrap();
        text.split_terminator('\0').map(str::to_owned).collect()
    }

    #[test]
    fn new_task_title_is_one_positional_and_the_body_goes_on_stdin() {
        for title in NASTY {
            let new = NewTask { title: title.into(), body: Some("line 1\n\n- line 3".into()), assignee: Some("-x".into()), priority: Some(5), tenant: None };
            let cmd = kanban_create(&new);
            assert_eq!(
                argv_seen_by_hermes(&cmd),
                ["kanban", "create", "--json", "--body-file", "-", "--assignee=-x", "--priority=5", "--", title]
            );
            assert_eq!(cmd.stdin.as_deref(), Some("line 1\n\n- line 3"));
            assert_eq!(cmd.summary, "hermes kanban create");
        }
    }

    #[test]
    fn comment_text_and_task_id_follow_the_end_of_options_marker() {
        let cmd = kanban_comment("-t_1", "$(rm -rf ~) it's");
        assert_eq!(argv_seen_by_hermes(&cmd), ["kanban", "comment", "--", "-t_1", "$(rm -rf ~) it's"]);
        assert_eq!(cmd.summary, "hermes kanban comment");
    }

    #[test]
    fn disabling_skills_writes_them_as_one_json_list() {
        let cmd = skills_set_disabled(&["a'b".to_owned(), "c d".to_owned()]);
        assert_eq!(argv_seen_by_hermes(&cmd), ["config", "set", "skills.disabled", r#"["a'b","c d"]"#]);
    }

    #[test]
    fn skills_listing_finds_every_skill_md_but_hidden_ones() {
        let home = std::env::temp_dir().join(format!("hd-skills-{}", std::process::id()));
        let profile = home.join("profile");
        for (dir, text) in [
            ("unslop", "---\nname: unslop\ndescription: Cut AI tells\n---\n# Body\n---\n"),
            ("devops/ad-cs", "---\ndescription: Request certs\n---\nbody"),
            (".archive/old", "---\nname: old\n---\n"),
            ("no-frontmatter", "# Just a body\n"),
        ] {
            std::fs::create_dir_all(profile.join("skills").join(dir)).unwrap();
            std::fs::write(profile.join("skills").join(dir).join("SKILL.md"), text).unwrap();
        }
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = format!(
            "#!/bin/sh\ncase \"$*\" in\n  'config path') echo {}/config.yaml ;;\n  'config get skills.disabled --json') echo '[\"unslop\"]' ;;\nesac\n",
            profile.display()
        );
        std::fs::write(bin.join("hermes"), fake).unwrap();
        std::process::Command::new("chmod").arg("+x").arg(bin.join("hermes")).status().unwrap();
        let out = std::process::Command::new("sh").arg("-c").arg(skills_list().remote_line()).env("HOME", &home).output().unwrap();

        let (skills, paths) = crate::work::skills_from_ssh(&String::from_utf8(out.stdout).unwrap());
        let mut names: Vec<_> = skills.iter().map(|s| (s.name.as_str(), s.enabled, s.description.as_str())).collect();
        names.sort();
        assert_eq!(names, [("ad-cs", true, "Request certs"), ("no-frontmatter", true, ""), ("unslop", false, "Cut AI tells")]);
        assert_eq!(paths["ad-cs"], "devops/ad-cs/SKILL.md");

        let out = std::process::Command::new("sh").arg("-c").arg(skill_content(&paths["unslop"]).remote_line()).env("HOME", &home).output().unwrap();
        assert!(String::from_utf8(out.stdout).unwrap().ends_with("# Body\n---\n"));
    }

    /// Read-only. Needs a working local `hermes`: `HERMES_DESKTOP_SSH=$PWD/../mock/fake-ssh cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn reads_board_and_skills_from_real_hermes() {
        let tasks = crate::work::board_from_cli(json(&run("localhost", kanban_list()).await.unwrap()).unwrap());
        let first = tasks.first().expect("at least one task");
        let detail = crate::work::detail_from(json(&run("localhost", kanban_show(&first.id)).await.unwrap()).unwrap()).unwrap();
        assert_eq!(detail.task.id, first.id);
        assert!(!crate::work::assignee_names(json(&run("localhost", kanban_assignees()).await.unwrap()).unwrap()).is_empty());
        let (skills, paths) = crate::work::skills_from_ssh(&run("localhost", skills_list()).await.unwrap());
        let skill = skills.iter().find(|s| !s.description.is_empty()).expect("a skill with a description");
        assert!(run("localhost", skill_content(&paths[&skill.name])).await.unwrap().contains(&skill.name));
    }

    #[test]
    fn json_output_is_found_past_chatter() {
        let pretty = "hermes: completing source-update dependencies...\n[warn] slow disk\n[\n  {\n    \"id\": \"t_1\"\n  }\n]\n";
        assert_eq!(json(pretty).unwrap()[0]["id"], "t_1");
        assert!(json("null\n").unwrap().is_null());
        assert!(json("Traceback (most recent call last):\n").is_err());
    }

    #[test]
    fn quoted_arguments_reach_the_shell_unchanged() {
        for arg in NASTY {
            let out = std::process::Command::new("sh").arg("-c").arg(format!("printf %s {}", quote(arg))).output().unwrap();
            assert_eq!(String::from_utf8(out.stdout).unwrap(), arg);
        }
    }

    #[test]
    fn mcp_servers_are_read_for_the_profile() {
        assert_eq!(argv_seen_by_hermes(&mcp_servers(Some("coder"))), ["-p", "coder", "config", "get", "mcp_servers", "--json"]);
        assert_eq!(argv_seen_by_hermes(&mcp_servers(None)), ["config", "get", "mcp_servers", "--json"]);
        for name in NASTY {
            assert_eq!(argv_seen_by_hermes(&mcp_test(Some("coder"), name)), ["-p", "coder", "mcp", "test", "--", name]);
        }
        assert_eq!(argv_seen_by_hermes(&mcp_test(None, "gh")), ["mcp", "test", "--", "gh"]);
    }

    #[test]
    fn only_a_failed_connection_counts_as_a_test_result() {
        // 1 is "connection failed" (output to parse); 3 (no such server) and 127 (no hermes) stay errors.
        for (exit, status) in [(0, 0), (1, 0), (3, 3), (127, 127)] {
            let out = run_with_fake_hermes(&mcp_test(None, "gh"), &format!("echo out; echo err >&2; exit {exit}"));
            assert_eq!(out.status.code(), Some(status), "hermes exit {exit}");
            assert_eq!(String::from_utf8(out.stdout).unwrap(), "out\n", "stderr stays out of the parsed output");
        }
    }

    /// Read-only, like `reads_board_and_skills_from_real_hermes`.
    #[tokio::test]
    #[ignore]
    async fn reads_and_tests_mcp_servers_from_real_hermes() {
        let servers = crate::work::mcp_from_config(json(&run("localhost", mcp_servers(None)).await.unwrap()).unwrap());
        for server in servers.iter().filter(|s| s.enabled) {
            if let Ok(out) = run("localhost", mcp_test(None, &server.name)).await {
                return assert!(!crate::work::mcp_test_from_cli(&out).tools.is_empty(), "{} lists no tools", server.name);
            }
        }
        panic!("no enabled MCP server answered");
    }
}
