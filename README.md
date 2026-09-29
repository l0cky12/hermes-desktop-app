# Hermes Desktop

A small Tauri client for a self-hosted Hermes Agent gateway: pairing and sign-in, streaming chat with tool progress and Stop, a Sessions panel, and drag-and-drop attachments. Vocabulary is in [CONTEXT.md](CONTEXT.md).

## Build and run

System packages (Tauri 2 on Linux):

```sh
# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev build-essential pkg-config curl wget file libxdo-dev libssl-dev librsvg2-dev
# Arch
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl librsvg xdotool
```

Then, with Node 20+ and a stable Rust toolchain:

```sh
npm install
npm run tauri dev                    # development window
npm run tauri build -- --no-bundle   # release binary: src-tauri/target/release/hermes-desktop
(cd src-tauri && cargo test)         # SSE parser, URL validation, cookie jar, attachments
```

## Pairing

On first launch, enter the Dashboard URL (usually `http://<host>:9119`) and the API server URL (usually `http://<host>:8642`). Then **Check gateway**, sign in if the gateway requires it, and enter the `API_SERVER_KEY`. URLs must be plain `http(s)://host[:port]`. HTTPS uses the system trust store; there is no custom certificate handling.

## Where things are stored

- `~/.config/local.hermes-desktop/settings.json` holds the two gateway URLs. It never holds secrets.
- The API key and sign-in cookie are stored as one entry in the OS keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows. That entry is bound to the gateway URLs it was issued for.
- **Keyring fallback:** if the keyring is missing, locked, or doesn't answer within 10 s (headless machines, CI), the app says so in a banner. Credentials then stay in memory for that run only, and it asks for them again on the next launch. It never falls back to a plaintext file.
- Logs go to stderr and contain request method, path, and status only.

Every request goes directly to the configured gateway: system proxies are ignored, and redirects are never followed.

## Attachments

The API server has no upload endpoint, so dropped files ride inside the Run request.
- Text files are inlined as fenced text.
- Images (`png`, `jpg`, `gif`, `webp`) are sent as `image_url` data-URL parts.
- Other file types, and files over 2 MB, are refused with an inline "Not sent" error.

## Testing against the mock gateway

`mock/` is test infrastructure and is not shipped. `mock/server.mjs` is a dependency-free Node imitation of the Dashboard and the API server, using Hermes' wire formats.

```sh
node mock/server.mjs   # Dashboard :19119, API server :18642
                       # user tester / correct-horse-battery, key hd-test-key-4f9c2a7e1b
```

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
