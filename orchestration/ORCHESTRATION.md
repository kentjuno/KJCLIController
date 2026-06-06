# Multi-Agent Orchestration Protocol (KJCLIController)

This file tells **any** AI agent how to act as a **conductor** that delegates subtasks to
the other local AI CLIs — **Claude Code**, **Gemini/Antigravity**, **OpenAI Codex** —
through the KJCLIController gateway, while keeping a shared understanding across steps.

> If you are an AI agent reading this: you may delegate to and consult the other local
> CLIs. Follow this protocol. It is tool-agnostic — it works whether you are Claude,
> Gemini/Antigravity, or Codex.

## 1. The gateway

OpenAI-compatible endpoint: `POST http://localhost:8080/v1/chat/completions`
- Header: `Authorization: Bearer <token>` (default `my-secret-lan-token`; configurable in `config.json`)
- Body: `{"model": "<claude|gemini|openai>", "messages": [...], "timeout": 120, "cwd": "<workspace>"}`
- Model names: `claude` = Claude Code, `gemini` = Gemini/Antigravity, `openai` = OpenAI Codex.
- Check availability first: `GET http://localhost:8080/api/providers`.

How to call (use whatever your environment supports — all equivalent):
- Convenience CLI: `python consult.py --model claude --prompt "..."` (or `--all`)
- Raw HTTP: `curl` the endpoint.
- Any OpenAI SDK with `base_url = http://localhost:8080/v1`.

## 2. Shared context = a ledger file in the workspace

The gateway is **stateless** and each worker is a **one-shot process**, so shared memory
must live **on disk**, not in the server.

- Choose a **workspace** = the working directory for the task. Pass it as the `cwd` field so
  workers run *inside* it (they get real read/write access there).
- Keep **`AGENT_LOG.md`** at the workspace root as the canonical cross-step record.
- Rule for **every** delegated subtask — the worker prompt MUST tell it to:
  1. **Read `AGENT_LOG.md` first** for context.
  2. Do the task.
  3. **Append a dated entry**: what it did, key decisions, and files changed.
- The conductor **reads `AGENT_LOG.md` before planning each next step**.

This makes context durable (survives gateway restarts), portable (just a file), and
quota-efficient (workers read a compact ledger instead of the whole history).

## 3. Context envelope per delegation

Make each delegation prompt self-contained:
- **Goal** — the overall objective.
- **Current step** — what THIS subtask must achieve.
- **Relevant prior results** — or pointers to files (use **relative** paths).
- **Workspace path**.

Ask each worker to end its answer with this footer so you can parse the outcome:
```
STATUS: done | blocked | needs-info
FILES_CHANGED: <relative paths, or none>
NEXT: <suggested next step>
```

## 4. Roles (tune to your own quota)

All three are first-class workers:

- **claude (Claude Code)** — senior technical advisor; hard design, architecture, and review calls.
- **openai (Codex)** — broad context and overview; implementation, plan coherence, cross-checks.
- **gemini (Antigravity / agy)** — fully capable for code and reasoning, but the **slowest**
  backend; give it a generous `--timeout` (300+) and use it for work you can wait on — it
  reports back when finished.

If you are already chatting in Antigravity directly, you often don't need to route to `gemini`
through the gateway — but it stays fully available for delegation whenever you want it. For a
quick two-worker delegation, `consult.py --models claude,openai` simply skips the slow backend.

## 5. Portability rules (critical when this runs on other machines)

1. **Use paths RELATIVE to the workspace.** Never hardcode absolute paths in prompts or in
   `AGENT_LOG.md` — they break the moment the project is cloned elsewhere.
2. **Do not assume CLI install locations or that every CLI exists.** The gateway resolves
   binaries (PATH + platform fallbacks). Always check `/api/providers` and **degrade
   gracefully** if a provider is unavailable (skip it, tell the user, continue with the rest).
3. **Be quota-aware.** Workers read the compact ledger, not full history. Don't over-delegate;
   reserve Claude for high-value decisions.
