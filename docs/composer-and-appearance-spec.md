# Hermes Desktop: composer controls and Appearance settings

Status: agreed design, 2026-09-29. Vocabulary is in [CONTEXT.md](../CONTEXT.md).

## Goal

Pick the Profile, Model choice, and Reasoning level from the composer. Attach files with a paperclip or by pasting images. Dictate a message. Change the app's look in a Settings view.

## Composer bar

The bar sits under the message box. From left to right: paperclip, mic, Profile selector, model selector (CPU icon), and Reasoning level (brain icon). Send/Stop stays on the right.

**Paperclip:** opens a file picker. Picked files follow the same Attachment rules as dropped files: text is inlined, `png`/`jpg`/`gif`/`webp` images are sent as images, other types are refused, and so is anything over 2 MB.

**Pasting images:**
- Pasting an image (a screenshot, or an image copied from a browser) adds it as an Attachment.
- It appears as a chip with a thumbnail and a remove (×) button.
- Pasted images over 2 MB, or in a format that can't be sent, are re-encoded as JPEG. The image is scaled down until it fits, and refused only if it still doesn't.
- Paste doesn't handle copied files; use the paperclip or drag-and-drop for those.

**Mic (Dictation):**
- Click to start recording and click again to stop. Recording stops on its own after 2 minutes.
- The transcript is inserted at the cursor.
- While recording, the mic turns red and shows a timer.
- If transcription fails, an inline error appears and the message box is left unchanged.
- It uses the Dashboard's speech-to-text (`POST /api/audio/transcribe`), so the mic is hidden over SSH.

**Profile selector:**
- Switching Profile switches the whole view. Any streaming Turn stops, a new chat starts, and the Sessions panel shows only that Profile's Sessions.
- At launch it selects the Profile last used in this app on this connection. If there isn't one, it uses the Gateway's default Profile (`hermes profile use`), then `default`.
- The app never changes the Gateway's default Profile.
- Over HTTP, a named Profile is reached at `/p/<name>/…` and has its own API key. The first switch asks for that key with the Re-auth prompt, which offers "Back to <previous Profile>". The key is then kept in the keyring alongside the others.
- Over SSH, switching restarts Hermes as `hermes -p <name> acp`. The Profile list comes from `hermes profile list`.

**Model selector:**
- Lists only providers that are set up (authenticated) and have models, grouped by provider, with a search box.
- The first entry is "Profile default".
- Over HTTP, the list comes from `GET /api/model/options`. Over SSH, it comes from the ACP `session/new` answer.

**Reasoning level:**
- Choices are Default, none, minimal, low, medium, high, xhigh, and max. Default means the field is omitted and the Profile decides.
- It's disabled over SSH, with a tooltip saying it's set in the Profile's config there.

## How choices behave

- The Model choice and Reasoning level belong to one Session and can change partway through it.
- A change made while a reply is streaming takes effect from the next Turn. The selectors stay usable the whole time.
- The first reply after a change shows a small muted label, such as "glm-5.3-flash · high".
- Reopening a Session restores its Model choice and Reasoning level.
- Over HTTP, both are sent with every Run (`provider`, `model`, `model_options.reasoning_effort`).
- Over SSH, the model is set with `session/set_model` before a Turn whenever it differs from the Session's current model.

## Settings

- A gear at the bottom-left of the Sessions panel opens a full Settings view, with a tab list on the left and a way back to the chat.
- **Appearance** is the only tab. It has three settings:
  - Theme: System (the default), Light, or Dark
  - Skin: hermes-webui's accent palettes (default, ares, mono, slate, poseidon, sisyphus, charizard, sienna, catppuccin, nous, geist-contrast, zeus), each working with either Theme
  - Font size: Small 13px, Default 14px, or Large 16px, scaling the whole app
- These settings belong to this machine, not to a Gateway.

## Out of scope

Saved prompts, the working-directory picker, hands-free voice mode, toolset overrides, read-aloud, and Skills.
