# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root: the glossary (Gateway, Profile, Session, Turn, Run, …).
- **`docs/adr/`**: read the ADRs that touch the area you're about to work in (`0001` networking in Rust, `0002` SSH Kanban and Skills via the hermes CLI).

This is a single-context repo; there is no `CONTEXT-MAP.md` and no per-directory `CONTEXT.md`.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to the synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0001 (networking in Rust), but worth reopening because…_
