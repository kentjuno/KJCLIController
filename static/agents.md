# Connect to KJ CLIController — Control Multiple Local AI Agents

You are an AI agent. This page tells you how to use the **KJ CLIController** gateway to
delegate work to the OTHER local AI CLIs — Claude Code, Gemini/Antigravity, OpenAI Codex —
and coordinate them step by step. Read it, then apply it to the current project.

## 1. The gateway (already running)

OpenAI-compatible endpoint: `POST http://localhost:8080/v1/chat/completions`
- Header: `Authorization: Bearer my-secret-lan-token` (default token; change if configured)
- Body: `{"model":"<claude|gemini|openai>","messages":[{"role":"user","content":"..."}],"timeout":120,"cwd":"<absolute path to this project>"}`
- Models: `claude` = Claude Code, `gemini` = Gemini/Antigravity, `openai` = OpenAI Codex.
- Check who is available first: `GET http://localhost:8080/api/providers`.

### How to call (use whatever you have)

Universal — curl:
```
curl -s http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-secret-lan-token" -H "Content-Type: application/json" \
  -d '{"model":"claude","messages":[{"role":"user","content":"..."}],"cwd":"<project abs path>"}'
```
Or any OpenAI SDK with `base_url = http://localhost:8080/v1`.

Optional helper (a small dependency-free Python CLI you can download from the gateway):
```
curl -O http://localhost:8080/consult.py
python consult.py --model claude --workspace . --prompt "..."   # one worker
python consult.py --all --prompt "..."                          # ask all three
```

## 2. Keep shared context across steps (the important part)

The gateway is **stateless** and each worker is a **one-shot process**, so shared memory must
live **on disk**:
- Use this project directory as the **workspace**; pass its absolute path as `cwd` so workers
  run inside it with real file access.
- Keep a ledger file **`AGENT_LOG.md`** at the workspace root.
- In **every** delegated prompt, instruct the worker to:
  1. **Read `AGENT_LOG.md` first** for context from previous steps.
  2. Do the task.
  3. **Append a dated entry** (what it did, key decisions, files changed) and end with:
     ```
     STATUS: done | blocked | needs-info
     FILES_CHANGED: <relative paths, or none>
     NEXT: <suggested next step>
     ```
- You (the conductor) **read `AGENT_LOG.md` before planning each next step**.

The `consult.py --workspace .` helper applies this wrapping automatically.

## 3. Roles (suggested)

- **gemini (Antigravity)** — main conductor / heavy lifting.
- **claude (Claude Code)** — senior technical advisor; consult for hard design / review calls. Use sparingly.
- **openai (Codex)** — broad context and overview; plan coherence and cross-checks.

Do not delegate to yourself — delegate to the OTHER two.

## 4. Portability rules (you may be on any machine)

- Use paths **relative** to the workspace in prompts and in `AGENT_LOG.md`. Only `cwd` is absolute.
- Do not assume a CLI exists — check `/api/providers` and skip/continue gracefully if one is missing.
- Be quota-aware: workers read the compact ledger, not the full history; reserve Claude for high-value calls.

## Apply now

Create or append `AGENT_LOG.md` in this project, then start delegating subtasks to the other
CLIs following the protocol above. Keep the human-readable version at
`http://localhost:8080/api/guide`.
