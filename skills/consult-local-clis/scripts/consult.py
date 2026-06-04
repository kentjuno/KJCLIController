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

Env / flags:
  --token        Bearer token (default: $CLI_CONTROLLER_TOKEN or "my-secret-lan-token")
  --base-url     gateway base (default: $CLI_CONTROLLER_URL or http://localhost:8080)
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


def consult(model, prompt, system=None, timeout=120, base_url=DEFAULT_BASE, token=DEFAULT_TOKEN):
    """Return (model, text, error). error is None on success."""
    messages = []
    # The "openai"/Codex CLI rejects a system role (no --system flag on this build),
    # so fold any system instruction into the user message for that provider.
    if system and model != "openai":
        messages.append({"role": "system", "content": system})
        user_text = prompt
    elif system:
        user_text = f"[System instruction: {system}]\n\n{prompt}"
    else:
        user_text = prompt
    messages.append({"role": "user", "content": user_text})

    payload = {"model": model, "messages": messages, "timeout": timeout}
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
    ap.add_argument("--prompt", help="the question / task to send")
    ap.add_argument("--system", default=None, help="optional system instruction")
    ap.add_argument("--timeout", type=int, default=120, help="CLI execution timeout in seconds")
    ap.add_argument("--token", default=DEFAULT_TOKEN)
    ap.add_argument("--base-url", default=DEFAULT_BASE)
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

    targets = ALL_MODELS if args.all else [args.model or "claude"]

    results = []
    if len(targets) == 1:
        results.append(consult(targets[0], args.prompt, args.system, args.timeout, args.base_url, args.token))
    else:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(targets)) as ex:
            futs = {
                ex.submit(consult, m, args.prompt, args.system, args.timeout, args.base_url, args.token): m
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
