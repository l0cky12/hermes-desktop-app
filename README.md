# Hermes Desktop

A small Tauri client for a self-hosted Hermes Agent gateway: pairing and sign-in, streaming chat with tool progress and Stop, a Sessions panel, Profile, model, and Reasoning level pickers, Dictation, drag-and-drop, picked, and pasted attachments, Appearance settings, and, from the icon rail, the Hermes Kanban Board, Skills with on/off toggles, and the Client log. Vocabulary is in [CONTEXT.md](CONTEXT.md).

## Install on Arch Linux

Either install a prebuilt package or build one on the PC. Both install through pacman, and you remove them with `sudo pacman -R hermes-desktop`.

- **Prebuilt:** copy `packaging/arch/hermes-desktop-*.pkg.tar.zst` to the PC and run `sudo pacman -U hermes-desktop-*.pkg.tar.zst`. pacman pulls in the runtime dependencies. The file isn't committed to git; rebuild it with the command below.
- **From source:** get the repo onto the PC, for example with `git clone` over SSH, then:

  ```sh
  sudo pacman -S --needed base-devel git
  cd hermes-desktop/packaging/arch && makepkg -si   # builds the committed checkout (~5 min)
  ```

  To update later, run `git pull` and `makepkg -si` again.

This adds **Hermes Desktop** to the app menu (`/usr/bin/hermes-desktop`).
- **Credentials:** saving them needs a Secret Service keyring. GNOME and KDE (KWallet) provide one. On other desktops, install `gnome-keyring`. Without one, the app still works, but asks you to sign in on every launch.
- **Blank window with NVIDIA drivers:** a known WebKitGTK workaround is to start it as `WEBKIT_DISABLE_DMABUF_RENDERER=1 hermes-desktop`.

## Build and run (development)

System packages on Debian or Ubuntu (on Arch, `makepkg -s` installs them):

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential pkg-config curl wget file libxdo-dev libssl-dev librsvg2-dev
```

Then, with Node 20+ and a stable Rust toolchain:

```sh
npm install
npm run tauri dev                    # development window
npm run tauri build -- --no-bundle   # release binary: src-tauri/target/release/hermes-desktop
(cd src-tauri && cargo test)         # SSE parser, URL validation, cookie jar, attachments, pickers, SSH quoting, Board/Skills parsing, Client log
npm test                             # webview preferences (Node 22.18+)
```

## Pairing

On first launch, enter the Dashboard URL (usually `http://<host>:9119`) and the API server URL (usually `http://<host>:8642`). Then **Check gateway**, sign in if the gateway requires it, and enter the `API_SERVER_KEY`. URLs must be plain `http(s)://host[:port]`. HTTPS uses the system trust store; there is no custom certificate handling.

### Over SSH instead

Under **Or run Hermes over SSH**, enter a host (`host`, `user@host`, or an `~/.ssh/config` alias). The app runs `ssh <host> hermes acp` and speaks the Agent Client Protocol over that connection, so there's no Dashboard, sign-in, or API key; your SSH keys or ssh-agent do the authentication. SSH runs in batch mode, so password and host-key prompts can't be answered: connect once from a terminal first to accept the host key. `hermes` must be on the remote `PATH` (`~/.local/bin` is added automatically).

Over SSH, the app can answer Approval requests. Sessions can't be deleted, and a dropped connection can't be reattached, so Retry starts a new Run in the same Session.

Kanban and Skills also work over SSH: each action runs its own `ssh <host> hermes kanban …` or `hermes config …` command (see [ADR 0002](docs/adr/0002-ssh-kanban-and-skills-via-hermes-cli.md)), and the open Board refreshes every 30 s. Every command pays for a new SSH connection, so turning on connection sharing for the host in `~/.ssh/config` makes them much faster:

```
Host hermes-box
  ControlMaster auto
  ControlPath ~/.ssh/cm-%r@%h:%p
  ControlPersist 10m
```

## Profiles, models, and reasoning

The bar under the message box picks the **Profile** (person icon), model (CPU icon), and Reasoning level (brain icon).

- Switching Profile shows that Profile's Sessions and starts a new chat.
- The app opens on the Profile you last used, or on the Gateway's default Profile (`hermes profile use`). It never changes that default itself.
- Over HTTP, a named Profile is reached at `/p/<name>/…` and needs its own `API_SERVER_KEY` (from that Profile's `.env`). The app asks for it the first time and keeps it in the keyring.
- Over SSH, switching restarts Hermes as `hermes -p <name> acp`.
- The model list shows only providers you've set up.
- The model and Reasoning level belong to each Session, can change partway through it, and take effect from the next Turn. A small label marks the first reply after a change.
- Over SSH, the Reasoning level comes from the Profile's config and can't be changed here.

## Dictation

The microphone records until you click it again, or for up to 2 minutes. The Dashboard's speech-to-text (`POST /api/audio/transcribe`) then turns the recording into text, which is inserted at the cursor for you to edit. This uses the Profile's own voice settings. It isn't available over SSH.

## Kanban, Skills, and Logs

- **Kanban** shows the default Board. Filter by text, Assignee, tenant, or status (click a stats chip). Create a Task from the "New task" box (Enter), or use **More…** for body, Assignee, priority, and tenant. Click a Task for its body and Comments. **Preview dispatcher** shows what one Dispatch would do; **Run dispatcher** runs one now (at most 8 spawns). Tasks are read-only apart from Comments.
- **Skills** lists the active Profile's Skills by category. Re-opening it shows the last list at once (until the app quits) and refreshes it in the background. Click one to read its SKILL.md; the switch enables or disables it on every platform. Essential Skills (`hermes-agent`) can't be switched off. Over SSH, only the Profile's own `skills/` directory is listed, not `skills.external_dirs`.
- **Logs** is the Client log (see below).

The dev mock (`node mock/server.mjs`) serves a small Board and Skills list too; `MOCK_KANBAN=off` answers like a Hermes with the Kanban plugin disabled, and `MOCK_SKILLS_MS=2000` makes the Skills list as slow as over SSH.

## Where things are stored

- `~/.config/local.hermes-desktop/settings.json` holds the two gateway URLs, or the SSH host. It never holds secrets.
- The API key and sign-in cookie are stored as one entry in the OS keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. That entry is bound to the gateway URLs it was issued for.
- Named Profiles' API keys are stored in that same keyring entry.
- The webview's own storage holds Appearance settings (Theme, Skin, Font size), the last Profile used on each connection, and each Session's model and Reasoning level. It never holds credentials.
- **Keyring fallback:** if the keyring is missing, locked, or doesn't answer within 10 s (headless machines, CI), the app says so in a banner. Credentials then stay in memory for that run only, and it asks for them again on the next launch. It never falls back to a plaintext file.
- The Client log (the **Logs** view, also written to stderr) holds request method, path, and status; SSH subcommand names and exit codes; ACP method names; and keyring or settings warnings. Never message content, task text, or secrets. It keeps the last 2,000 lines in memory and starts empty each launch.

Every request goes directly to the configured gateway: system proxies are ignored, and redirects are never followed.

## Attachments

The API server has no upload endpoint, so Attachments ride inside the Run request. Drop files on the window, pick them with the paperclip, or paste an image.
- Text files are inlined as fenced text.
- Images (`png`, `jpg`, `gif`, `webp`) are sent as `image_url` data-URL parts.
- Other file types, and files over 2 MB, are refused with an inline "Not sent" error.
- Pasted images over 2 MB, or in a format that can't be sent, are re-encoded as JPEG and scaled down until they fit.

## Testing against the mock gateway

`mock/` is test infrastructure and is not shipped. `mock/server.mjs` is a dependency-free Node imitation of the Dashboard and the API server, using Hermes' wire formats.

```sh
node mock/server.mjs   # Dashboard :19119, API server :18642
                       # user tester / correct-horse-battery, key hd-test-key-4f9c2a7e1b
```

Profiles: `default` (the key above), `orchestrator` (`hd-orch-key-7d3e9a1c5b`), and `coder` (no key, so switching to it is refused). Replies start with `[profile · model · reasoning]` so you can see what a Run was sent with. `MOCK_NO_MODELS=1` makes the model list fail; `MOCK_TRANSCRIBE_MS` delays speech-to-text.

Prompt keywords script the reply:
- `tool`: tool events
- `idle`: 12 s of silence, so a keepalive fires mid-stream
- `slow`: a long reply, for kill and Stop tests
- `approval`: an approval request
- `fail`: a server-side failure

Headless run with a throwaway keyring:

```sh
Xvfb :121 -screen 0 1400x900x24 & export DISPLAY=:121
export DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address=1)
echo -n test | gnome-keyring-daemon --unlock --components=secrets --daemonize
src-tauri/target/release/hermes-desktop
```

`python3 mock/drag.py FILE...` opens a GTK drag source. You can drag from it with xdotool (`mousedown 1`, a few `mousemove` steps into the app window, then `mouseup 1`).

`mock/fake-ssh` stands in for `ssh` and runs the remote command locally, so SSH mode can be tried against a local `hermes`: start the app with `HERMES_DESKTOP_SSH=$PWD/mock/fake-ssh npm run tauri dev` and enter any host. `(cd src-tauri && HERMES_DESKTOP_SSH=$PWD/../mock/fake-ssh cargo test -- --ignored)` lists and loads real Sessions, and reads the real Board and Skills (read-only), that way.
