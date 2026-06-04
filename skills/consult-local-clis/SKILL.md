---
name: consult-local-clis
description: >-
  Consult local AI CLIs (Claude Code, Gemini/Antigravity, OpenAI Codex) through the
  KJ CLIController gateway and bring their answers back into the conversation. Use this
  skill WHENEVER the user wants a second opinion from another model or asks to "consult",
  "ask", or "check with" the other CLIs — including Vietnamese phrasings like "tham vấn
  mấy ông CLI", "hỏi mấy ông kia", "hỏi claude/gemini/codex xem", "cho mấy ông CLI coi
  thử", "tham khảo ý kiến", or English ones like "ask the other models", "get a second
  opinion from gemini/codex/claude", "what do the local CLIs think", "cross-check with
  the other AIs". Trigger even when the user names just one model (e.g. "hỏi codex thử")
  or all of them. This is the right tool any time the intent is to delegate a question to
  the local claude/gemini/openai CLIs and synthesize what comes back.
---

# Consult Local AI CLIs (via CLIController gateway)

This skill lets you forward a question or task to the local AI CLIs — **Claude Code**,
**Gemini/Antigravity (`agy`)**, and **OpenAI Codex** — running behind the local
**KJ CLIController** gateway (OpenAI-compatible API), then bring their replies back and
make sense of them for the user.

The user thinks of this as "tham vấn mấy ông CLI" — consulting the panel of CLIs.

## The one tool you need: `scripts/consult.py`

Always go through this bundled script. It is dependency-free (Python stdlib only) and it
already handles the two things that bite people who call the gateway by hand:
- On Windows, an inline `curl -d '{...}'` gets its quotes mangled by the shell. The script
  builds the JSON itself, so this never happens.
- The `openai` (Codex) provider on this host does **not** accept a system role / `--system`
  flag — sending one returns HTTP 500. The script folds any `--system` text into the user
  message for that provider automatically.

```bash
# Check which CLIs are up before dispatching (fast, do this if unsure)
python scripts/consult.py --providers

# Ask ONE model
python scripts/consult.py --model claude --prompt "<the question>"

# Ask ALL THREE in parallel, then synthesize
python scripts/consult.py --all --prompt "<the question>"

# With a system instruction and a longer timeout for heavy tasks
python scripts/consult.py --model openai --prompt "<task>" --system "Be concise" --timeout 240
```

The script prints each reply under a `===== MODEL =====` header. Pass `--json` if you'd
rather parse structured output. Exit code is non-zero if any provider errored.

Run it from the skill directory, or give the absolute path to `consult.py`. The gateway
defaults to `http://localhost:8080` with token `my-secret-lan-token`; override with
`--base-url`/`--token` or the env vars `CLI_CONTROLLER_URL` / `CLI_CONTROLLER_TOKEN` if the
user has changed them.

## How to choose who to consult

Read the user's phrasing and pick the narrowest interpretation that fits:

1. **User named specific model(s)** → consult exactly those. "hỏi codex" → `--model openai`.
   Note the name mapping: `claude` = Claude Code, `gemini` = Gemini/Antigravity, and
   **`openai` = OpenAI Codex** (so "codex"/"openai"/"GPT" all map to `--model openai`).

2. **User said "mấy ông CLI" / "cả ba" / "all the models" / "the panel"** → use `--all`
   and then synthesize (see below).

3. **Ambiguous "tham vấn" with no target** → this is the default: pick the single CLI best
   suited to the task rather than spamming all three. Rough guide:
   - **Codex (`openai`, GPT-5)** — pure code generation, algorithmic problems, tight
     reasoning, "write/refactor this function".
   - **Claude Code (`claude`)** — agentic/codebase reasoning, multi-file design, careful
     prose, nuanced judgement calls.
   - **Gemini (`gemini`)** — vision/multimodal, very large context, broad world knowledge.

   If the question is genuinely a "who's right / I want multiple perspectives" question
   (architecture decisions, tradeoffs, reviews), prefer `--all` even when unprompted — that
   is usually the spirit of "tham vấn".

When in real doubt about scope, ask the user once: one model or the whole panel?

## Synthesizing multiple answers

When you consulted more than one CLI, don't just dump the three blocks back. Add a short
synthesis on top so the consultation is actually useful:

- **Consensus** — what they agree on (this is the high-confidence part).
- **Divergence** — where they differ, and your read on which is more credible and why.
- **Recommendation** — your own bottom line, informed by all of them.

Keep the raw per-model answers available below the synthesis (or offer to show them) so the
user can audit. Attribute claims to the model that made them.

## Practical notes

- Default `--timeout` is 120s. Bump to 240–300 for large code generation or long analyses;
  the CLIs run real subprocesses and can be slow.
- If a provider returns an error, report it plainly and continue with the others — a partial
  consultation still has value. A quick `--providers` check explains most failures
  (a CLI not installed/authenticated shows `"available": false`).
- This is local-first: prompts go only to the user's own machine via the gateway, never to a
  cloud API directly. Reassure the user of that if relevant.
- Don't consult the CLIs for trivial questions you can answer yourself — the point is a
  genuine second opinion or specialized horsepower, not a reflex.
