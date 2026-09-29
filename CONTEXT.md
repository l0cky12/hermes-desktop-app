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

**Session**:
A conversation transcript stored on the API server; never the Sign-in cookie.
_Avoid_: chat, thread, conversation

**Turn**:
One user message plus the reply shown for it in the chat view.
_Avoid_: message, exchange

**Run**:
One server-side execution that produces a reply; a Turn is backed by one Run, or more after Retry.
_Avoid_: job, request, completion

**Attachment**:
A dropped file carried inside the Run request of the Turn it was dropped onto.
_Avoid_: upload, file

**Retry**:
Recovering a failed, stopped, or not-sent Turn: reattaching to its Run's event stream where it left off, or starting a new Run with the same input in the same Session when that Run is gone.
_Avoid_: resend, regenerate

**Stop**:
Closing a Turn's event stream at once and asking the API server to stop its Run.
_Avoid_: cancel, abort

**Approval request**:
A Run pausing until a human allows or denies a risky command; this client can answer it only over SSH.
_Avoid_: permission prompt, confirmation

### Work

**Profile**:
A named Hermes agent identity on the host, with its own Skills and settings; the active Profile is the one the client works with.
_Avoid_: agent, user, account

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
