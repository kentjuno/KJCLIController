#!/usr/bin/env python3
"""
consult.py — Dispatch a prompt to the local CLIController gateway and print the reply.

Why this exists: calling the gateway by hand with curl is error-prone on Windows
(escaped quotes get mangled by the shell) and the Codex/"openai" provider rejects
the `--system` flag, so a system message must be folded into the user message.
This script handles both pitfalls and has zero third-party dependencies (urllib only).

Usage:
  python consult.py --model claude --prompt "Explain Rust lifetimes"
  python consult.py --all --prompt "Design a rate limiter" --system "Be concise"
  python consult.py --model openai --prompt "Refactor this" --timeout 240
  python consult.py --providers          # list available CLIs and exit

  # Orchestration mode: run the worker inside a shared workspace and have it
  # read/append the AGENT_LOG.md ledger automatically (see ORCHESTRATION.md):
  python consult.py --model codex --workspace . --prompt "Implement the parser module"

  # Ask a chosen subset (the two strongest, quota-efficient workers):
  python consult.py --models claude,openai --prompt "Review this approach"

Env / flags:
  --token        Bearer token (default: $CLI_CONTROLLER_TOKEN or "my-secret-lan-token")
  --base-url     gateway base (default: $CLI_CONTROLLER_URL or http://localhost:8080)
  --workspace    run the worker in this dir (sets cwd) and auto-apply the ledger protocol
  --json         print the raw JSON envelope instead of just the text
"""
import argparse
import concurrent.futures
import json
import os
import sys
import urllib.request
import urllib.error

DEFAULT_BASE = os.environ.get("CLI_CONTROLLER_URL", "http://localhost:8080")
DEFAULT_TOKEN = os.environ.get("CLI_CONTROLLER_TOKEN", "my-secret-lan-token")
ALL_MODELS = ["claude", "gemini", "openai"]
LEDGER_FILE = "AGENT_LOG.md"


def wrap_with_ledger(prompt):
    """Wrap a task prompt with the shared-ledger orchestration protocol.

    The worker is told to read AGENT_LOG.md first (context from previous steps),
    do the task, then append a dated summary and end with a parseable footer.
    """
    return (
        "You are collaborating with other AI agents in a shared workspace "
        "(your current working directory).\n"
        f"FIRST, read {LEDGER_FILE} here (if it exists) for context from previous steps.\n\n"
        f"YOUR TASK:\n{prompt}\n\n"
        f"WHEN DONE: append a dated entry to {LEDGER_FILE} (create it if missing) summarizing "
        "what you did, key decisions, and files changed. Use paths relative to the workspace. "
        "End your reply with this footer:\n"
        "STATUS: done | blocked | needs-info\n"
        "FILES_CHANGED: <relative paths, or none>\n"
        "NEXT: <suggested next step>"
    )


def _post(base_url, token, payload, timeout_http):
    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout_http) as resp:
        return json.loads(resp.read().decode("utf-8"))


def consult(model, prompt, system=None, timeout=120, base_url=DEFAULT_BASE,
            token=DEFAULT_TOKEN, workspace=None):
    """Return (model, text, error). error is None on success.

    If `workspace` is given, the worker runs inside that directory (via the gateway's
    `cwd` field) and the prompt is wrapped with the AGENT_LOG.md ledger protocol.
    """
    task = wrap_with_ledger(prompt) if workspace else prompt

    messages = []
    # The "openai"/Codex CLI rejects a system role (no --system flag on this build),
    # so fold any system instruction into the user message for that provider.
    if system and model != "openai":
        messages.append({"role": "system", "content": system})
        user_text = task
    elif system:
        user_text = f"[System instruction: {system}]\n\n{task}"
    else:
        user_text = task
    messages.append({"role": "user", "content": user_text})

    payload = {"model": model, "messages": messages, "timeout": timeout}
    if workspace:
        # Absolute path so the gateway can resolve and chdir into it reliably.
        payload["cwd"] = os.path.abspath(workspace)
    try:
        # Give the HTTP read a little more headroom than the CLI's own timeout.
        data = _post(base_url, token, payload, timeout + 30)
        text = data["choices"][0]["message"]["content"]
        return (model, text, None)
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        return (model, None, f"HTTP {e.code}: {body}")
    except Exception as e:  # noqa: BLE001 - surface any transport/parse error verbatim
        return (model, None, f"{type(e).__name__}: {e}")


def list_providers(base_url, token):
    req = urllib.request.Request(
        f"{base_url}/api/providers",
        headers={"Authorization": f"Bearer {token}"},
    )
    with urllib.request.urlopen(req, timeout=15) as resp:
        return json.loads(resp.read().decode("utf-8"))


def main():
    ap = argparse.ArgumentParser(description="Consult local AI CLIs via the CLIController gateway.")
    ap.add_argument("--model", choices=ALL_MODELS, help="single provider to consult")
    ap.add_argument("--all", action="store_true", help="consult claude, gemini and openai in parallel")
    ap.add_argument("--models", default=None,
                    help="comma-separated subset, e.g. 'claude,openai' (overrides --model/--all)")
    ap.add_argument("--prompt", help="the question / task to send")
    ap.add_argument("--system", default=None, help="optional system instruction")
    ap.add_argument("--timeout", type=int, default=120, help="CLI execution timeout in seconds")
    ap.add_argument("--token", default=DEFAULT_TOKEN)
    ap.add_argument("--base-url", default=DEFAULT_BASE)
    ap.add_argument("--workspace", default=None,
                    help="run the worker in this dir (sets cwd) + apply the AGENT_LOG.md ledger protocol")
    ap.add_argument("--providers", action="store_true", help="list available CLIs and exit")
    ap.add_argument("--json", action="store_true", help="print raw JSON results")
    args = ap.parse_args()

    if args.providers:
        try:
            print(json.dumps(list_providers(args.base_url, args.token), indent=2))
        except Exception as e:  # noqa: BLE001
            print(f"Failed to reach gateway at {args.base_url}: {e}", file=sys.stderr)
            sys.exit(1)
        return

    if not args.prompt:
        ap.error("--prompt is required unless using --providers")

    if args.models:
        targets = [m.strip() for m in args.models.split(",") if m.strip()]
        unknown = [m for m in targets if m not in ALL_MODELS]
        if unknown:
            ap.error(f"unknown model(s): {', '.join(unknown)} (valid: {', '.join(ALL_MODELS)})")
    elif args.all:
        targets = ALL_MODELS
    else:
        targets = [args.model or "claude"]

    results = []
    if len(targets) == 1:
        results.append(consult(targets[0], args.prompt, args.system, args.timeout,
                               args.base_url, args.token, args.workspace))
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(targets)) as ex:
            futs = {
                ex.submit(consult, m, args.prompt, args.system, args.timeout,
                          args.base_url, args.token, args.workspace): m
                for m in targets
            }
            # Preserve a stable order (claude, gemini, openai) regardless of finish time.
            done = {futs[f]: f.result() for f in concurrent.futures.as_completed(futs)}
            results = [done[m] for m in targets]

    if args.json:
        print(json.dumps(
            [{"model": m, "text": t, "error": e} for (m, t, e) in results],
            indent=2, ensure_ascii=False,
        ))
        return

    exit_code = 0
    for (model, text, error) in results:
        header = f"===== {model.upper()} ====="
        print(header)
        if error:
            print(f"[ERROR] {error}")
            exit_code = 2
        else:
            print(text)
        print()
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
