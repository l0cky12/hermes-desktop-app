---
status: done
---

# Icon rail with Chat, Kanban, Skills, and Logs

## Problem Statement

Hermes Desktop has one screen: the Sessions panel and chat. To see or manage the Hermes Board, check which Skills a Profile has enabled, or find out why the client failed to talk to Hermes, the user has to leave the app: open the Dashboard in a browser, SSH in and run `hermes kanban …` or the interactive `hermes skills config`, or read stderr from a terminal. When the client is connected over SSH there is no Dashboard at all, so none of it is reachable from the app.

## Solution

A narrow icon rail down the left edge switches between four views, and the whole app moves to a dark theme matching the Hermes Dashboard look:

- **Chat**: exactly today's screen (Sessions panel and chat).
- **Kanban**: the default Board as columns of Task cards, with a left panel of search, filters, status stats, New task, and dispatcher controls; clicking a card opens a Task drawer with its body and Comments, where a Comment can be added.
- **Skills**: the active Profile's Skills grouped by category, each with an on/off toggle; clicking one shows its raw SKILL.md with the toggle in the header.
- **Logs**: the Client log, a live, filterable record of what this client did while talking to Hermes.

Kanban and Skills work the same whether the client is paired with a Gateway or connected over SSH.

## User Stories

### Rail and theme

1. As a user, I want an icon rail on the left edge with Chat, Kanban, Skills, and Logs, so that I can switch views in one click.
2. As a user, I want the rail's current view highlighted, so that I know where I am.
3. As a user, I want each rail icon to have a tooltip and accessible name, so that I can tell the icons apart and use them with a screen reader or keyboard.
4. As a user, I want Chat to be the view I land on at launch, so that the app still opens straight into conversation.
5. As a user, I want the rail to appear only after Pairing (or SSH connect) is done, so that the pairing screens stay uncluttered.
6. As a user, I want switching away from Chat and back to keep my current Session, draft, and any streaming Turn intact, so that looking at the Board never interrupts a Run.
7. As a user, I want the whole app in one dark theme with the existing purple accent, so that it matches the Hermes Dashboard and is easy on the eyes.
8. As a user, I want the pairing, Re-auth prompt, banner, and chat screens to be readable in the dark theme, so that nothing is left light-on-light or dark-on-dark.

### Kanban: board

9. As a user, I want the Board shown as columns for Triage, Todo, Scheduled, Ready, Running, Blocked, Review, and Done, so that I see Tasks by status the way Hermes tracks them.
10. As a user, I want the Scheduled and Review columns hidden when they're empty, so that the board doesn't waste width on statuses I rarely use.
11. As a user, I want each column header to show its Task count, so that I can see load at a glance.
12. As a user, I want an empty column to show "Empty", so that I can tell it loaded rather than failed.
13. As a user, I want each card to show the Task id, priority, title, the start of its body, and its Assignee, so that I can identify the Task without opening it.
14. As a user, I want the Board header to show the Board's name ("Default"), so that I know which Board I'm looking at.
15. As a user, I want the columns to scroll horizontally when they don't fit, so that I can reach Done on a narrow window.

### Kanban: filters and stats

16. As a user, I want to search Tasks by title and body text, so that I can find one Task among dozens.
17. As a user, I want to filter by Assignee, so that I can see one Profile's work.
18. As a user, I want to filter by tenant, so that I can separate work for different tenants.
19. As a user, I want archived Tasks hidden unless I tick "Include archived", so that finished history doesn't clutter the Board.
20. As a user, I want stats chips with counts per status (and a total), so that I can see the Board's shape without scrolling.
21. As a user, I want clicking a status chip to filter the Board to that status, and clicking it again to clear it, so that I can focus on, say, Blocked Tasks.
22. As a user, I want a "N visible tasks" count that reflects my filters, so that I know how much the filters are hiding.

### Kanban: New task

23. As a user, I want an inline "New task" box where typing a title and pressing Enter creates a Task, so that capturing work is instant.
24. As a user, I want a "More…" link that opens a small form with title, body, Assignee (picked from the Board's Profiles), priority, and tenant, so that I can create a fully described Task when I need to.
25. As a user, I want everything else about a new Task (workspace, skills, model, triage) left at Hermes defaults, so that the form stays short.
26. As a user, I want a multi-line body with blank lines and lines starting with `-` to arrive intact, so that Markdown-ish bodies aren't mangled.
27. As a user, I want the Board to reload after I create a Task and show it in its column, so that I can see it worked.
28. As a user, I want a clear error message if creation fails, with my typed title kept, so that I don't lose what I wrote.

### Kanban: Task drawer

29. As a user, I want clicking a card to open a drawer with the Task's title, id, status, Assignee, priority, tenant, and full body, so that I can read the whole Task.
30. As a user, I want the drawer to show the Task's Comments with author and time, oldest first, so that I can follow the discussion.
31. As a user, I want an Add comment box in the drawer, so that I can leave a note for the Assignee.
32. As a user, I want the drawer's Comments to refresh after I add one, so that I can see mine landed.
33. As a user, I want to close the drawer with a close button or Escape, so that I can get back to the Board quickly.
34. As a user, I want the drawer to be view-only apart from Comments, so that I can't change a Task's status or Assignee by accident.

### Kanban: Dispatch

35. As a user, I want a "Preview dispatcher" button that shows what a Dispatch would do (Tasks it would start, reclaim, or promote) without doing it, so that I can check before acting.
36. As a user, I want a "Dispatch now" button in that preview, so that I can act on what I just reviewed.
37. As a user, I want a "Run dispatcher" button that runs one Dispatch immediately, so that I don't have to wait for the gateway's own 60 s loop.
38. As a user, I want to see what a Dispatch actually did, and the Board reloaded afterwards, so that I know whether anything was started.
39. As a user, I want both dispatcher actions capped at Hermes's default of 8 spawns, so that one click can't flood the host.

### Kanban: freshness

40. As a user, I want the Board loaded when I open the Kanban view, so that it's current when I look at it.
41. As a user, I want a refresh button, so that I can reload on demand.
42. As a user, I want the Board to refresh every 30 s while the Kanban view is open (and not while it's hidden), so that Running and Done stay roughly current without hammering Hermes.
43. As a user, I want a refresh that fails to leave the last Board on screen with an error line, so that a blip doesn't blank the view.

### Skills

44. As a user, I want the active Profile's Skills listed and grouped by category with a count per category, so that I can scan a long list.
45. As a user, I want Skills without a category grouped under "General", so that none go missing.
46. As a user, I want categories collapsible, so that I can fold the ones I don't care about.
47. As a user, I want each row to show a toggle, the Skill's name, and its description truncated to one line, so that I can see what it's for and whether it's on.
48. As a user, I want disabled Skills dimmed, so that on and off are distinguishable at a glance.
49. As a user, I want a search box that filters Skills by name and description, so that I can find one quickly.
50. As a user, I want clicking a Skill to show its raw SKILL.md in a monospace, read-only pane, so that I can read exactly what the agent reads.
51. As a user, I want the same toggle in the detail pane's header, so that I can switch it off right after reading it.
52. As a user, I want toggling a Skill to take effect for the active Profile on all platforms, so that "off" means off.
53. As a user, I want essential Skills (e.g. `hermes-agent`) shown with a locked toggle and a tooltip saying they can't be disabled, so that I'm not surprised when they stay on.
54. As a user, I want a failed toggle to snap back and show an error, so that the switch never lies about the Skill's state.

### Logs (Client log)

55. As a user, I want a Logs view showing the Client log live, newest at the bottom, so that I can watch the client talk to Hermes.
56. As a user, I want each line to show a time, a source (gateway, ssh, acp, kanban, keyring, settings), and the text, so that I can tell what happened where.
57. As a user, I want gateway lines to show method, path, and status (or the error), so that I can see which request failed.
58. As a user, I want SSH lines to show which `hermes …` subcommand ran and its exit code, but never its arguments, so that Task titles and bodies never land in the log.
59. As a user, I want ACP lines to show method names and connection open/close only, never message content, so that my conversations stay out of the log.
60. As a user, I want to filter by source and search the text, so that I can isolate one kind of problem.
61. As a user, I want auto-scroll that pauses when I scroll up, so that I can read an older line without it jumping.
62. As a user, I want "Copy all" and "Clear" buttons, so that I can paste the log into a bug report or start fresh.
63. As a user, I want the Client log to hold the last 2,000 lines in memory and start empty each launch, so that it never grows without bound or persists anything to disk.
64. As a user, I want the same lines still written to stderr, so that running from a terminal still works as before.

### Both connection modes

65. As a user paired with a Gateway, I want Kanban and Skills to work through the Dashboard with my existing Sign-in, so that I don't sign in again.
66. As a user connected over SSH, I want Kanban (board, New task, drawer, Comments, Dispatch) and Skills (list, SKILL.md, toggle) to work too, so that SSH isn't a second-class mode.
67. As a user whose Sign-in cookie is rejected while using Kanban or Skills, I want the usual Re-auth prompt, so that recovery works the same as in chat.
68. As a user whose Hermes has the Kanban plugin disabled, I want the Kanban view to say so plainly, so that I know it isn't a client bug.
69. As a user whose SSH command fails (host unreachable, `hermes` not on PATH, old Hermes without a subcommand), I want the view to show the error and the Client log to record it, so that I can fix the host.

## Implementation Decisions

- **All Hermes traffic stays in Rust** (ADR 0001). The webview only calls new Tauri commands; it never fetches.
- **One transport-agnostic Hermes work module** in the Tauri shell owns the Board and Skills. Its interface is the set of new Tauri commands, each returning the same normalized shape regardless of connection mode:
  - `kanban_board() -> tasks[]` and `kanban_assignees() -> names[]`
  - `kanban_task(id) -> { task, comments[] }`
  - `kanban_create({ title, body?, assignee?, priority?, tenant? }) -> task`
  - `kanban_comment(id, text)`
  - `kanban_dispatch(dry_run) -> { spawned[], reclaimed[], promoted[], … }`, always capped at 8.
  - `skills_list() -> [{ name, description, category, enabled, essential }]`
  - `skill_content(name) -> string`
  - `skill_toggle(name, enabled)`
  - `client_log() -> lines[]` plus a live channel of new lines.
- Inside that module, each command picks a backend by connection mode:
  - **Gateway**: Dashboard routes with the Sign-in cookie, through the existing Dashboard request path (so credential host-pinning, no proxies, no redirects all still apply). Board: `/api/plugins/kanban/board`, `/tasks`, `/tasks/{id}`, `/tasks/{id}/comments`, `/dispatch?dry_run=&max=8`, `/assignees`, `/stats`. Skills: `GET /api/skills`, `GET /api/skills/content?name=`, `PUT /api/skills/toggle`. A 404 on the kanban plugin routes maps to a "Kanban plugin is disabled on this Hermes" error.
  - **SSH** (ADR 0002): a one-shot `ssh <host> …` per action, separate from the long-lived `hermes acp` connection, reusing the existing host validation, batch mode, the `~/.local/bin` PATH prefix, and the test shim override. Board: `hermes kanban list --json` (plus `--archived`), `stats --json`, `assignees --json`, `show <id> --json`, `create <title> --body-file - --json` with the body on stdin, `comment <id> <text>`, `dispatch [--dry-run] --max 8 --json`. Skills: a short read-only script finds the active Profile's home via `hermes config path`, prints `hermes config get skills.disabled --json`, then each SKILL.md's frontmatter under that Profile's `skills/` directory (hidden directories skipped); one SKILL.md is read with `cat` by the path that listing reported, never a path from the webview. Toggling reads the current disabled list and writes it back with `hermes config set skills.disabled '[…]'`.
  - Every user-supplied value on an SSH command line (title, comment text, skill names, task id) is single-quote escaped for the remote POSIX shell; bodies go on stdin, never argv.
  - The whole Board (archived included) is loaded once per refresh; search, Assignee, tenant, archived, and status-chip filtering all happen client-side.
- **Normalizers**: small pure functions turn each backend's raw JSON into the shared shapes, so the webview never sees transport differences. Status names are Hermes's (`triage, todo, scheduled, ready, running, blocked, review, done, archived`); priority is Hermes's plain integer shown as `P<n>`.
- **Skill toggle semantics**: global for the active Profile (`skills.disabled`), not per platform. Essential Skills are reported as `essential: true` and the toggle is refused client-side; Hermes also drops them from the disabled list itself.
- **Client log module**: a bounded in-memory ring (2,000 lines) of `{ time, source, text }`, fed by one logging helper that replaces today's direct stderr writes and still echoes to stderr. Redaction is structural, not filtering: callers only ever pass method, path, status, subcommand name, exit code, ACP method name, or a warning, so there is nothing secret to strip. New lines are pushed to the webview over a Tauri channel.
- **Webview**: the existing single frontend module gains the rail and three views, built with the same small DOM helper and no `innerHTML` (SKILL.md and all Task text render as text). No new frontend dependencies. The Chat view's DOM is kept alive while hidden, so a streaming Turn keeps streaming.
- **Theme**: the existing CSS variables are flipped to a dark palette in place; the accent stays `#5b4bd6` (lightened only where contrast needs it).
- **Polling**: the Kanban view refreshes on open, after its own writes, on the refresh button, and every 30 s while visible; the timer stops when another view is shown.
- **Dev mock**: the mock gateway gains the kanban and skills Dashboard routes it needs, so `npm run tauri dev` against it exercises the Gateway path.

## Testing Decisions

- A good test drives the public interface (a normalizer, the SSH argument builder, the Client log ring, or a Tauri command against a mock) and asserts on what comes out, never on internal state or call order.
- **Unit tests (Rust, `cargo test`)**, at these seams:
  - Normalizers: real JSON captured from `hermes kanban list/show/stats/dispatch --json` and from the Dashboard kanban and skills routes goes in; the shared shapes come out. Include an archived Task, an uncategorized Skill, and an essential Skill.
  - SSH argument building: titles/comments containing quotes, `$()`, backticks, newlines, and leading `-` come out as a single safe remote argument; the body is never in argv; a Client log line for the command contains the subcommand and exit code but none of the arguments.
  - Client log ring: keeps only the last 2,000 lines in order; each line has time, source, text.
- **Integration tests (ignored by default, like the existing ACP one)**: through the fake-ssh shim against a real local `hermes`, list the Board, show a Task, list Skills, and read one SKILL.md. Writes (create, comment, toggle, dispatch) are not run against a real Hermes in tests.
- **Prior art**: the pure-function tests in the ACP module (update translation, history replay, host validation), the URL/cookie tests in the gateway module, and the ignored real-Hermes ACP test with `HERMES_DESKTOP_SSH=mock/fake-ssh`.
- **Frontend**: typechecked (`tsc`), then checked by hand in `npm run tauri dev` against the mock gateway and over fake-ssh; no frontend test framework is added.

## Out of Scope

- Rail items beyond Chat, Kanban, Skills, and Logs (no placeholders).
- Boards other than Default, and the board switcher.
- Bulk actions, drag and drop between columns, and editing a Task's status, Assignee, priority, or body.
- "Only mine" filter.
- The duplicate task list under the Kanban filters.
- Live Board updates over the Dashboard WebSocket.
- The Task event timeline and worker attempts (task runs).
- Profile picker for Skills; per-platform Skill toggles.
- Editing, installing, or uninstalling Skills; rendered Markdown.
- Hermes server logs (`agent.log`, `gateway.log`, …); persisting the Client log to disk.

## Further Notes

- Vocabulary follows CONTEXT.md: Board, Task, Assignee, Comment, Dispatch, Skill, Profile, Client log. Hermes's own "task runs" are deliberately not surfaced so they never compete with Run.
- To verify first: a Dashboard that doesn't require Sign-in (loopback bind) authenticates its API routes with an `X-Hermes-Session-Token` header rather than the Sign-in cookie. Pairing today allows a Dashboard with no Sign-in, so check whether such a Dashboard serves the kanban and skills routes to the client; if not, show "Sign-in is required on the Dashboard for Kanban and Skills" rather than adding token handling.
- Over SSH, users with many actions per minute should enable ssh ControlMaster; the README should mention it alongside the existing SSH notes.
