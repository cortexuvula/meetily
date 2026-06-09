# AGENTS.md

Hard-won context for agents working in this repo. `CLAUDE.md` is the long-form overview (architecture, commands, audio pipeline) — read it first. This file only contains the facts an agent is most likely to get wrong.

## Repo layout

Cargo workspace at the root with two members:
- `frontend/src-tauri` — Tauri 2.x app (Rust lib crate named `app_lib`, plus a thin `main.rs`). All real backend logic lives in `src/lib.rs` and `src/audio/`.
- `llama-helper` — separate Rust binary crate that uses `llama-cpp-2`; not used in the main Tauri build, kept in the workspace for LLM-helper experiments.

`frontend/` is a Next.js 14 (App Router) + React 18 + Tailwind project. `frontend/src-tauri` is a sibling that Tauri bundles. `package.json` is in `frontend/`, not the repo root.

`backend/` is a standalone FastAPI service (SQLite via `aiosqlite`, LLM providers: ollama/openai/anthropic/groq/openrouter). It is **not** part of the Cargo workspace.

`backend/whisper.cpp` is a **git submodule** (branch `develop`, custom fork). Initialize with `git submodule update --init --recursive` after cloning, otherwise `./build_whisper.sh` and the backend won't work.

## Build/dev gotchas

- **Entry to Tauri commands**: `frontend/src-tauri/src/lib.rs`. New `#[tauri::command]`s must be added to the `tauri::generate_handler![...]` list there. `lib_old_complex.rs` is dead — don't edit.
- **Hot-path logging**: `lib.rs` defines `perf_debug!` / `perf_trace!` macros that compile to no-ops in release. Use them in audio hot paths, not `log::debug!`.
- **Tauri config**: `frontend/src-tauri/tauri.conf.json` controls dev server (port 3118), CSP, bundle identifier (`com.meetily.ai`), and feature flags. CSP whitelists the local Ollama, backend, and whisper-server URLs — if you add a new local service, update it.
- **GPU features** (Cargo features on `frontend/src-tauri`): `metal`/`coreml` (macOS), `cuda`/`vulkan` (Win/Linux), `hipblas` (Linux AMD). Helper scripts: `frontend/scripts/tauri-auto.js` auto-detects GPU; the simpler `pnpm run tauri:dev:metal` etc. are hard-coded.
- **Cross-platform runner scripts** are in `frontend/` (not `frontend/src-tauri/`): `clean_run.sh`, `clean_build.sh`, `clean_run_windows.bat`, `build-gpu.sh` / `build-gpu.ps1`. macOS vs Windows paths diverge — keep `.sh` and `.bat`/`.ps1` in sync when changing build flow.
- **whisper-rs binary**: `frontend/src-tauri/binaries/` is gitignored. The build script downloads the right whisper.cpp binary on first compile — needs network.

## Audio module

`frontend/src-tauri/src/audio/` is the most-edited area. Real structure:
- `pipeline.rs` — ring buffer + professional mixing (RMS-based ducking) + VAD.
- `recording_manager.rs` / `recording_state.rs` — orchestration and shared state (`Arc<RwLock<...>>` + `AtomicBool`).
- `recording_commands.rs` / `system_audio_commands.rs` — Tauri command surface.
- `devices/discovery.rs` + `devices/platform/{windows,macos,linux}.rs` — platform-specific enumeration. **Add new platforms there**, not in `mod.rs`.
- `capture/{microphone,system,core_audio}.rs` — capture streams.
- There is also an `audio_v2/` directory alongside `audio/` at the same level — it is an in-progress rewrite, not production code. Don't add new commands to it without checking with maintainers.

`core-old.rs`, `recording_commands.rs.backup`, `recording_saver_old.rs` are leftovers from the modularization refactor — leave them.

## Backend

- Single entrypoint: `backend/app/main.py`. DB layer: `backend/app/db.py` (`DatabaseManager` async via `aiosqlite`). Default port **5167**, default Whisper server port **8178**.
- `requirements.txt` is the source of truth for Python deps — there is no `pyproject.toml`.
- CORS is wide-open (`"*"`) for dev only; tighten before any non-local deploy.
- Whisper model downloads go via `build_whisper.sh small` (or any of `tiny`, `base`, `small`, `medium`, `large-v1/v2/v3`, `large-v3-turbo`).

## Lint / typecheck / test

- **Frontend**: `pnpm run lint` (next lint via `eslint.config.mjs`), `pnpm run build` does the TS typecheck. No `pnpm test` is wired up.
- **Rust**: `cargo check` / `cargo clippy` from repo root works for both workspace members. There are no first-party Rust unit tests of note — the audio system is tested manually.
- **Backend**: no test framework configured; verify via the Swagger UI at `http://localhost:5167/docs`.

There is no `make` target and no `pre-commit` config. The CI workflows in `.github/workflows/` are all **`workflow_dispatch`-only** — they do not run on push or PR. The composite `build.yml` is called by `build-{macos,windows,linux,test,devtest}.yml`.

## Git / workflow conventions

- Active branches: `main` (release) and `devtest` (integration). Feature work forks from `devtest`, not `main`.
- Commit format follows Conventional Commits (`feat:`, `fix:`, `chore:`, etc.) per `CONTRIBUTING.md`.
- Don't update git config, push, force-push, or commit unless the user explicitly asks.
- PR template lives at `.github/pull_request_template.md`.

## Style reminders

- **No new comments in code** unless the user asks (per repo's coding rules).
- Audio devices are named `microphone` / `system` everywhere — never `input` / `output`.
- macOS system audio requires both microphone **and** screen-recording permissions; the app checks both.
- `.env` is gitignored — never commit secrets; use Tauri's path APIs (`downloadDir`, etc.) rather than hardcoded paths.
