# Hermes Desktop

A thin desktop client for a self-hosted Hermes Agent gateway. The gateway owns all conversation state; the client only displays and drives it.

## Language

### Gateway

**Gateway**:
The `hermes serve` backend the client is paired with, made of two surfaces: the Dashboard and the API server.
_Avoid_: server (ambiguous), backend

**Dashboard**:
The gateway surface that answers status and Sign-in requests.
_Avoid_: web UI, auth server

**API server**:
The gateway surface that executes Runs and stores Sessions, gated by the API key.
_Avoid_: OpenAI server, chat server

**API key**:
The bearer secret (`API_SERVER_KEY`) that authorizes every API server request.
_Avoid_: token, password

### Access

**Pairing**:
The first-run flow that records the gateway URLs, completes Sign-in, and stores the API key.
_Avoid_: setup, onboarding, login

**Sign-in**:
Username/password login against the Dashboard, producing the sign-in cookie.
_Avoid_: session, login session

**Sign-in cookie**:
The Dashboard-issued cookie proving a completed Sign-in.
_Avoid_: session cookie, session

**Re-auth prompt**:
The screen shown when a stored credential is rejected, asking only for the rejected credential.
_Avoid_: login error, auth error

### Conversation

**Profile**:
A named Hermes agent identity on the host, with its own Sessions, API key, default model, Skills, and settings; the active Profile is the one the client works with, one at a time, and the app never changes the Gateway's own default Profile.
_Avoid_: account, persona, agent, user

**Session**:
A conversation transcript stored on the API server under one Profile; never the Sign-in cookie.
_Avoid_: chat, thread, conversation

**Pin**:
Hermes's own keep flag on a Session, shared by every client of that Profile; a Pinned Session sits above the rest of the Session list.
_Avoid_: favorite, star, bookmark

**Turn**:
One user message plus the reply shown for it in the chat view.
_Avoid_: message, exchange

**Run**:
One server-side execution that produces a reply; a Turn is backed by one Run, or more after Retry.
_Avoid_: job, request, completion

**Model choice**:
The provider and model that a Session's next Turns run on; "Profile default" until the user picks one, and changeable mid-Session.
_Avoid_: engine, LLM

**Reasoning level**:
How much the model reasons before replying, chosen per Session; "Default" follows the Profile.
_Avoid_: effort, thinking mode

**Attachment**:
A file (dropped, picked with the paperclip, or a pasted image) carried inside the Run request of the Turn it was added to.
_Avoid_: upload, file

**Dictation**:
Speaking into the microphone to fill the message box with text the user can edit before sending.
_Avoid_: voice mode, TTS

**Retry**:
Recovering a failed, stopped, or not-sent Turn: reattaching to its Run's event stream where it left off, or starting a new Run with the same input in the same Session when that Run is gone.
_Avoid_: resend, regenerate

**Stop**:
Closing a Turn's event stream at once and asking the API server to stop its Run.
_Avoid_: cancel, abort

**Approval request**:
A Run pausing until a human allows or denies a risky command; this client can answer it only over SSH.
_Avoid_: permission prompt, confirmation

### Appearance

**Appearance settings**:
This machine's look for the app, the same whichever Gateway it connects to.
_Avoid_: preferences, config

**Theme**:
The light or dark base colors (or follow the system).
_Avoid_: mode, color scheme

**Skin**:
An accent palette layered on top of either Theme.
_Avoid_: theme, color scheme

### Work

**Board**:
The Hermes Kanban board of Tasks; the client works with the default Board only.
_Avoid_: project, kanban (as a noun)

**Task**:
A unit of work on the Board with a title, body, status, priority, and optional Assignee; unrelated to a Run or a Turn.
_Avoid_: ticket, card, job

**Assignee**:
The Profile a Task is assigned to.
_Avoid_: owner, worker

**Comment**:
A note appended to a Task's discussion.
_Avoid_: message, reply

**Dispatch**:
One pass of the Hermes dispatcher that hands ready Tasks to their Assignees; a Dispatch preview shows what a pass would do without doing it.
_Avoid_: run (reserved for Run), nudge

**Skill**:
An instruction file (SKILL.md) installed for a Profile, grouped by category, which can be enabled or disabled for that Profile.
_Avoid_: plugin, tool, prompt

### Diagnostics

**Client log**:
This client's own record of what it did while talking to Hermes (requests with method, path, and status; connection events; warnings). Never contains message content or secrets. Distinct from Hermes's server logs.
_Avoid_: logs (unqualified), server log
