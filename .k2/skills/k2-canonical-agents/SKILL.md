---
k2_skill: k2-canonical-agents
skill_version: 2
skill_checksum: 2d082bab9469ec7f
name: k2-canonical-agents
description: Set up or refresh the canonical AGENTS.md from existing harness files (run with an AI assistant)
---

<!-- K2:MANAGED:BEGIN -->
# K2 Canonical Agents

Operator for the canonical workspace-context shape. Run this WITH an AI
assistant so it merges existing content with judgment rather than overwriting.

## The shape
- `.k2/agent/ROLE.md` — authored role.
- `.k2/PROJECT.md` — project context (authored).
- `.k2/AGENTS.md` — canonical, GENERATED from the two above.
- cwd `AGENTS.md` — generate (default ON for **new** agents): a real-file copy
  of `.k2/AGENTS.md` so tools that read the workspace root see the same bytes.
- `CLAUDE.md`, `GEMINI.md`, root `AGENT.md`, `.goosehints`, `.cursor/rules/k2so.mdc`
  — leftover name-compat, planted only when **fan-out** is on (default OFF).
- Skills (`.k2/skills/<name>/SKILL.md`) are loadable capabilities, not the entrypoint.

## Set up an EXISTING project (merge, don't clobber)
1. Read any existing `CLAUDE.md` / `AGENTS.md` / `GEMINI.md` in the workspace.
2. Fold persona-ish guidance into `.k2/agent/ROLE.md` and project facts into
   `.k2/PROJECT.md`, preserving the user's wording.
3. Regenerate: the daemon composes `.k2/AGENTS.md` and plants cwd `AGENTS.md`
   when generate is on. Leftover names only if fan-out is on.
4. A user-authored cwd `AGENTS.md` is never overwritten by generate.

## A NEW project
Generate (default ON) writes cwd `AGENTS.md`. Fan-out is a separate opt-in
for leftover harness names — it is not how you get `AGENTS.md`.
<!-- K2:MANAGED:END -->

<!-- Content below this line is yours — K2 will never modify it. -->
<!-- Content below this line is yours — K2 will never modify it. -->
