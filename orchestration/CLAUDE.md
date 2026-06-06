# Agent Project Instructions

This project can orchestrate **multiple local AI agents** — Claude Code, Gemini/Antigravity,
and OpenAI Codex — through the **KJCLIController** gateway.

When a task would benefit from delegating subtasks to, or getting a second opinion from, the
other CLIs, read **[ORCHESTRATION.md](ORCHESTRATION.md)** in this directory and follow that
protocol.

In short: delegate via the OpenAI-compatible gateway at `http://localhost:8080`, keep shared
cross-step context in `AGENT_LOG.md`, and use paths relative to this workspace.

> Copy this file into the root of whatever project you orchestrate from. It is the pointer
> that makes Claude Code aware of the multi-agent setup.
