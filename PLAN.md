# Implementation Plan: CLIController (Rust AI CLI Gateway & UI)

This project builds a **gateway in Rust** at `f:\AntiGravity\CLIController`. The service accepts commands from the LAN through an **OpenAI-compatible API**, secured with a **Bearer Token**, calls the local AI CLIs (**Claude Code**, **Antigravity/`agy`**, **OpenAI Codex**) to process them, and returns the results, together with a visual **chat test UI**.

> All the technical details below are **derived from real, working CLI behavior** captured during the spike phase — not guesswork. Each solution documents the underlying mechanism for reference.

---

## 1. Technical Architecture & Data Flow

```
                      LAN Clients (cURL / Python SDK / Web UI)
                                        │
                                        ▼ [Port 8080]  Bearer Token
                            ┌───────────────────────┐
                            │      CLIController     │
                            │  (Axum Rust Server)    │
                            │  registry + fail-fast  │
                            └───────────┬───────────┘
                                        │  (resolve .cmd + stdin pipe)
             ┌──────────────────────────┼──────────────────────────┐
             ▼                          ▼                          ▼
   ┌──────────────────┐       ┌──────────────────┐       ┌──────────────────┐
   │    claude CLI    │       │      agy CLI     │       │    codex CLI     │
   │  stdin + JSON    │       │  temp-file +     │       │  dual-mode:      │
   │  envelope        │       │  semaphore(1)    │       │  CLI / REST API  │
   └──────────────────┘       └──────────────────┘       └──────────────────┘
```

### Core principles
1. **Resolve `.cmd` on Windows**: CLIs installed via npm are `.cmd` shim files. Try `PATH` first (`which`), then fall back to `%APPDATA%\npm\<name>.cmd`, `%USERPROFILE%\AppData\Roaming\npm\<name>.cmd`, `~\AppData\Roaming\npm\<name>.cmd`. Cache the result.
2. **Always pass the prompt over stdin**, never via argv: `cmd.exe` re-parses argv for `.cmd` files, corrupting special characters (`"`, `&`, `|`, newlines). Stdin bypasses the argv parser entirely.
3. **CREATE_NO_WINDOW** (`creation_flags(0x08000000)`): avoids a black console window flashing every time a `.cmd` is invoked.
4. **Fail-fast gating**: check `is_available()` (cached `--version`) BEFORE dispatching, so we don't swallow the full 90s timeout.
5. **Provider is a singleton**: cache the `--version` probe and `codex --help` result for the lifetime of the process.

### CLI flag reference table

| | **Claude** | **agy (Antigravity)** | **Codex** |
|---|---|---|---|
| Binary | `claude` (.cmd) | `agy` (.cmd) | `codex` (.cmd) |
| Command | `claude -p --output-format json` | `agy --print "<instruction>" --dangerously-skip-permissions` | `codex exec --output-format json -p -` |
| Prompt | via **stdin** | via **arg `--print`** (the content tells agy to read the input file and write the output file) | via **stdin** (sentinel `-`) |
| System prompt | `--append-system-prompt <text>` | embed `[System: ...]` in the payload | `--system <text>` |
| Attachment | `@<abspath>` token embedded in the prompt | absolute paths listed in the payload | image flag probed from `--help` (`--image`/`--attach`/`--file`/`--input`) |
| Permissions | `--add-dir <parent>` (one per unique parent) + `--permission-mode bypassPermissions` (when attachments present) | `--dangerously-skip-permissions` | (none) |
| Output | parse JSON `{"result":"...","is_error":...}` | read `out_{uuid}.txt`, fall back to stdout, strip ANSI, extract JSON | parse JSON, try `result`→`output_text`→`text` in order |
| Special | read all of stdout (avoid deadlock) | **Semaphore(1)** serialization; detect quota keywords | **dual-mode**: missing CLI / text-only+vision → REST API fallback |
| Vision | yes | yes | depends on CLI version; otherwise REST `gpt-4o` base64 |

---

## 2. Engineering lessons & how to port them to Rust

### 2.1 Stdin piping — AVOID DEADLOCK
The reference implementation uses synchronous `subprocess.run(input=...)`. In Rust/Tokio you must write stdin, **then drop stdin**, and read the output with `wait_with_output()` to avoid a deadlock when the stdout buffer fills:
```rust
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

let mut cmd = Command::new(&resolved_bin);
cmd.args(["-p", "--output-format", "json"])
   .stdin(Stdio::piped())
   .stdout(Stdio::piped())
   .stderr(Stdio::piped());

#[cfg(windows)]
{
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

let mut child = cmd.spawn()?;
{
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(full_prompt.as_bytes()).await?;
    // stdin is dropped at the end of this block -> closes the stream, the CLI knows input is done
}
let output = tokio::time::timeout(
    Duration::from_secs(90),
    child.wait_with_output(),
).await??; // on timeout -> kill the child in the Err branch
```

### 2.2 Claude — getting past the permission prompt
When attachments are present: `--add-dir <parent_dir>` for each unique parent dir + `--permission-mode bypassPermissions`. Without these, the CLI asks "y/n" and a non-interactive `-p` command receives a refusal as text instead of the result.

### 2.3 agy — exchanging via temp files (the real mechanism)
1. Create `agy_tmp/in_{uuid}.txt` containing: the payload (system + prompt + attachment list) **plus an instruction telling agy to write the result to `out_{uuid}.txt`**.
2. Call `agy --print "Read the instruction from <in.txt> ... write the result to the output file" --dangerously-skip-permissions`, with `cwd = agy_tmp`.
3. Wait for completion → read `out_{uuid}.txt`; if empty, fall back to stdout.
4. `strip_ansi()` (regex `\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])`) → `extract_json_fragment()` (find a parseable `{}`/`[]` pair).
5. Delete both temp files.
6. Wrap the whole thing in a **`tokio::sync::Semaphore(1)`** — agy runs only one call at a time.
7. Detect quota: scan for the keywords `usage limit` / `rate limit` / `credits` / `upgrade to pro` → return a clear error.

### 2.4 Codex — dual-mode + image flag probe
- Probe `codex --version` (CLI present?) and `codex --help` (which image flag?) once, and cache.
- Routing table: CLI has an image flag → CLI; CLI text-only + no attachment → CLI; CLI text-only + has attachment → **REST API fallback** (`gpt-4o`, base64 images ≤5MB); no CLI → REST API (requires API key).

### 2.5 Shared infrastructure
- Validation: prompt ≤ **100KB**, attachments ≤ **10 files**, files must exist and be readable, **sanitize filenames** against path traversal.
- Timeouts: probe **5s**, dispatch default **90s**.

---

## 3. Phased implementation plan

### Phase 0 — CLI validation spike (DO THIS FIRST, ~half a day)
Run manually on the actual host machine to lock down real behavior before coding:
```cmd
claude --version & agy --version & codex --version
echo Say pong | claude -p --output-format json
codex --help
```
- **DoD**: 3 sample output logs (claude envelope, agy file output, codex envelope + real image flag) to write parsers against reality.

### Phase 1 — Foundation, Config, Auth & `cli_utils`
- `cargo init --bin`; `Cargo.toml`: `axum`, `tokio` (full), `serde`, `serde_json`, `tower-http` (fs, cors), `uuid`, `regex`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`, `reqwest` (for the Codex API fallback).
- `src/config.rs`: read ENV `CLI_CONTROLLER_TOKEN`/`PORT` (priority) > `config.json`.
- `src/auth.rs`: Bearer middleware, **constant-time comparison**, 401 on mismatch.
- `src/cli/resolve.rs`: resolve `.cmd` on Windows + cache + CREATE_NO_WINDOW helper.
- `src/cli/validate.rs`: prompt 100KB, ≤10 attachments, sanitize filenames.
- **DoD**: `cargo check` passes; unit test for resolve finds `claude.cmd`; 401 on a bad token.

### Phase 2 — CLI Subprocess Adapters (`src/cli/`)
- `run_claude`: stdin pipe + `--append-system-prompt` + `--add-dir`/bypass + parse the `result` envelope.
- `run_agy`: temp-file exchange + `Semaphore(1)` + strip ANSI + extract JSON + detect quota.
- `run_codex`: dual-mode (CLI first, REST fallback) + cache the image flag from `--help`.
- Every adapter: `tokio::time::timeout` + **kill the child** on timeout; map errors to a unified enum (`thiserror`).
- **DoD**: 3 integration tests that make real calls and return clean text; a timeout test that kills the process.

### Phase 3 — Axum Server & Registry (`src/main.rs`, `src/registry.rs`)
- `registry.rs`: provider singleton + `is_available()` cache + **fail-fast gate** before dispatch.
- Endpoints:
  - `POST /v1/chat/completions`: accept OpenAI JSON, **flatten `messages[]`** (merge system → `--append-system-prompt`/`--system`, concatenate user/assistant), decode/load attachments, route by `model` → wrap in an OpenAI JSON response. Decide firmly: support `stream:true` (SSE) or force `stream:false`.
  - `POST /api/upload`: accept `multipart/form-data`, save to `temp_uploads/<uuid>_<safe_filename>`.
  - `GET /api/providers`: list available CLIs (via the registry).
- Static routes: `/static` (frontend) + `/outputs`.
- **DoD**: cURL without a token → 401; with a token → 200 OpenAI JSON; `/api/providers` reports the correct status of all 3 CLIs.

### Phase 4 — Chat UI (`static/`)
- `index.html`: left column for configuration (CLI model selector, system prompt, token); right column for the chat view.
- `style.css`: dark background (Slate), glassmorphism, chat bubble animations.
- `app.js`: send messages, preview attachments, call `/v1/chat/completions` with the Bearer Token.
- **DoD**: send/receive messages end-to-end through the UI with all 3 models.

### Phase 5 — Hardening & Polish
- Clean up `temp_uploads/` and `outputs/` on a TTL.
- CORS (`tower-http`), map errors → **OpenAI error JSON format**, logging with `tracing`.
- (Optional) Codex output capture: if kept, isolate per **UUID subdir** rather than diffing a snapshot of the whole directory (avoids a race condition with concurrent requests).
- **DoD**: a Python SDK pointed at `base_url=http://host:8080/v1` runs smoothly; `cargo test` + `cargo clippy` clean.

---

## 4. Verification Plan

### 4.1 CLI presence
```cmd
claude --version
agy --version
codex --version
```

### 4.2 API via cURL
```bash
# No token  → expect 401
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{\"messages\":[{\"role\":\"user\",\"content\":\"hello\"}]}"

# With token → expect 200 + OpenAI JSON
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Authorization: Bearer my-secret-lan-token" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"claude\",\"messages\":[{\"role\":\"user\",\"content\":\"Say pong\"}]}"
```

### 4.3 Risk checklist to verify
- [ ] A prompt containing `"`, `&`, `|`, newlines → the CLI receives it intact (thanks to stdin).
- [ ] No cmd window flashing (CREATE_NO_WINDOW).
- [ ] Two concurrent agy requests → no temp-file overwrite (Semaphore).
- [ ] CLI hangs > 90s → the process is killed and a timeout error is returned.
- [ ] Filename `..\..\evil.txt` → gets sanitized.
- [ ] CLI quota exhausted → a clear message is returned, not a confusing parse error.
