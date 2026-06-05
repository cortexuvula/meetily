# Database State & Readiness Banner — Design Spec

**Date:** 2026-06-05
**Status:** Approved — ready for implementation planning
**Author:** OpenCode brainstorming session
**Related:** supersedes the previous "panic on DB init failure" fix; the `block_on` in `lib.rs::setup` and `.expect()` were replaced with a logged `if let Err` in the prior change. This spec adds the missing half: surfacing that degraded state to the user.

## Overview

Today, when the SQLite database can't be opened (corrupted file, permission denied, disk full) or when it's a first launch and the user hasn't picked a database yet, the Tauri app:

- Either panics the process (init failure — fixed previously to log instead, but the user gets no UI feedback)
- Or silently leaves `AppState` unmanaged (first launch — every DB command then fails with a confusing "state not found" error)

The user has no way to know which situation they're in or what to do about it.

This spec adds a managed `DatabaseState` that tracks readiness, gates DB-using commands behind a `require_db` guard, and surfaces a non-dismissible banner in the UI with the appropriate action button ("Pick a database" or "Retry"). Setup commands remain ungated so the user can recover from the banner.

## Goals

- When the DB is unavailable for any reason, the user sees a clear, persistent banner explaining the situation and offering the next action.
- DB-using commands fail fast with a typed error string the frontend can recognize, instead of producing cryptic `state not found` or `Result::Err` panics.
- The user can resolve the situation without restarting the app (via "Pick a database" or "Retry").
- The first-launch flow continues to work; the existing `first-launch-detected` event keeps firing for backwards compat with the 4 existing listeners (`AnalyticsProvider`, `useRecordingStop`, `OnboardingContext`, `lib/analytics.ts`).

## Non-goals

- Replacing the `first-launch-detected` event with a fully state-driven first-launch flow. The event stays; this is a follow-up if desired.
- Adding a tray icon indicator for DB state.
- A keyboard shortcut to trigger the banner's action.
- Auto-retry with backoff. Retry is user-initiated only.
- Migrating DB commands to a new error type beyond the `String` prefix marker described in §4. Existing `Result<T, String>` contracts stay.

## Design decisions (locked during brainstorming)

| Decision | Choice |
|---|---|
| State shape | `DatabaseStatus::Ready` or `Unavailable { reason }` |
| Scope of "unavailable" | One unified state covering both init failure and first-launch-pending |
| UI prominence | Persistent, non-dismissible red banner with action button(s) |
| Gating strategy | Only "use" commands gated; "setup" commands always allowed |
| First-launch event | Kept; no breakage of existing listeners |
| Error signaling | Sentinel string prefix `DATABASE_UNAVAILABLE: <reason>` in the existing `Result<T, String>` contract |
| Retry mechanism | New `retry_database_init` Tauri command, user-initiated only |

## State machine

### Rust

```rust
pub enum DatabaseStatus {
    Ready,
    Unavailable { reason: String },
}

pub struct DatabaseState {
    status: parking_lot::Mutex<DatabaseStatus>,
    db_manager: parking_lot::Mutex<Option<DatabaseManager>>,
}
```

- The `db_manager` is `Option` because it's only present when `status == Ready`. This makes the "is the manager present?" question impossible to ask inconsistently with the status.
- `parking_lot::Mutex` (matches the existing `video_recording::state::VideoRecordingState` pattern in this codebase).

### Transitions

```
[App boot]
    │
    ▼
Unavailable("Initializing database…")
    │
    ├─► Ready                                    // DB opened successfully
    │
    ├─► Unavailable("First launch — pick a database to continue")
    │       │
    │       └─► Ready                           // after import_and_initialize_database
    │                                            // or initialize_fresh_database
    │
    └─► Unavailable("Database initialization failed: <err>")
            │
            └─► Ready                           // after retry_database_init succeeds
            │
            └─► Unavailable (unchanged)         // after retry fails
```

The state machine has exactly one transition owner: `database::setup::initialize_database_on_startup` (and its retry variant). No other code mutates `status` or `db_manager`.

### DTO sent to the frontend

```rust
#[derive(Serialize, Clone)]
pub struct DatabaseStatusDto {
    pub is_ready: bool,
    pub reason: Option<String>,
}
```

Flat shape, not the Rust enum. The frontend doesn't need to know the internal variants.

## Command gating

### Always allowed (ungated)

These are the commands the user needs to call to fix an unavailable DB:

- `check_first_launch`
- `select_legacy_database_path`
- `detect_legacy_database`
- `check_default_legacy_database`
- `check_homebrew_database`
- `import_and_initialize_database`
- `initialize_fresh_database`
- `get_database_directory`
- `open_database_folder`

### Gated (return `DATABASE_UNAVAILABLE: <reason>` if status ≠ Ready)

Everything else in `database/commands.rs` that calls into `state.db_manager`. Concretely this includes all of:

- `api_get_meetings`, `api_search_transcripts`, `api_get_meeting`, `api_get_meeting_metadata`, `api_get_meeting_transcripts`
- `api_save_meeting_title`, `api_save_transcript`, `api_delete_meeting`
- `api_get_profile`, `api_save_profile`, `api_update_profile`
- `api_get_model_config`, `api_save_model_config`
- `api_get_api_key`, `api_get_transcript_config`, `api_save_transcript_config`, `api_get_transcript_api_key`
- `api_save_custom_openai_config`, `api_get_custom_openai_config`, `api_test_custom_openai_connection`
- `api_process_transcript`, `api_get_summary`, `api_save_meeting_summary`
- `api_get_meeting_summary_language`, `api_save_meeting_summary_language`
- `api_get_meeting_detected_summary_language`, `api_save_meeting_detected_summary_language`
- `api_detect_transcript_summary_language`, `api_cancel_summary`
- `api_test_backend_connection`, `api_debug_backend_connection`
- The transcript-recovery helpers (`recover_audio_from_checkpoints`, `cleanup_checkpoints`, `has_audio_checkpoints`) — these touch the DB too

### The guard helper

```rust
fn require_db(state: &DatabaseState) -> Result<&DatabaseManager, String> {
    let status = state.status.lock();
    match &*status {
        DatabaseStatus::Ready => {}
        DatabaseStatus::Unavailable { reason } => {
            return Err(format!("DATABASE_UNAVAILABLE: {}", reason));
        }
    }
    drop(status);
    state
        .db_manager
        .lock()
        .as_ref()
        .ok_or_else(|| "DATABASE_UNAVAILABLE: manager missing in Ready state".to_string())
}
```

Call sites change from `let m = &state.db_manager;` (or `state.db_manager.method()`) to `let m = require_db(&state)?;`. The `?` propagates the typed error to the command's `Result<T, String>` return. Mechanical change across ~30 commands.

## New Tauri commands

```rust
#[tauri::command]
pub fn get_database_status(state: State<'_, DatabaseState>) -> DatabaseStatusDto { ... }

#[tauri::command]
pub async fn retry_database_init(
    state: State<'_, DatabaseState>,
) -> Result<DatabaseStatusDto, String> { ... }
```

`get_database_status` is sync (just reads the mutex).
`retry_database_init` re-runs the same logic as `initialize_database_on_startup` — checks first-launch, runs the appropriate init path, updates the state.

## Frontend

### Hook: `useDatabaseStatus.ts`

```typescript
export interface DatabaseStatus { is_ready: boolean; reason?: string }

export function useDatabaseStatus() {
  // On mount: invoke('get_database_status')
  // Poll every 30s as a safety net
  // Returns { status, refresh() }
  // refresh() re-queries and updates state
}

export async function retryDatabaseInit(): Promise<DatabaseStatus> { ... }
```

Polling at 30s is a safety net for state changes that happen via another code path (e.g., the first-launch event triggering an import via the existing listeners). It's not the primary update mechanism — the banner's action buttons call `refresh()` immediately after their backend call returns.

### Banner: `DatabaseErrorBanner.tsx`

- Mounted in `RootLayout` (the existing layout file at `frontend/src/app/layout.tsx`), between the providers and the `<div className="flex"><Sidebar/><MainContent/></div>` wrapper, so it appears above the sidebar.
- Returns `null` if `is_ready`.
- Otherwise renders a persistent red banner:

| `reason` starts with… | Title | Body | Action button |
|---|---|---|---|
| `"First launch"` | "Pick a database to continue" | the reason | "Pick a database" |
| anything else | "Database unavailable" | the reason | "Retry" |

Detection is by `reason.startsWith("First launch")`. The Rust side sets the reason to exactly `"First launch — pick a database to continue"` so the frontend can match it.

### Action button wiring

- **"Pick a database"** (first launch): triggers the existing first-launch flow. Concretely, the button calls `invoke('select_legacy_database_path')` to open the OS file picker; on success, the user is shown the legacy DB import flow. This is the same path the existing `first-launch-detected` event takes. No new Tauri command needed.
- **"Retry"** (init failure): calls `retryDatabaseInit()`, which invokes `retry_database_init`, gets the new status, calls `refresh()` on the hook.

## Error handling

### Backend

The guard returns `Result<&DatabaseManager, String>`. The string is formatted as:

```
DATABASE_UNAVAILABLE: <reason>
```

Commands that use `require_db` propagate this string via `?`. The frontend sees the same `String` in the rejected promise that other command errors use — no new error type or Tauri error handling changes.

### Frontend

When a Tauri command rejects with a string that starts with `"DATABASE_UNAVAILABLE:"`, the frontend shows the banner (which it already does based on the hook). It does **not** also show a toast — that would be redundant.

For other errors (e.g., a constraint violation, an IO error mid-query), the existing per-call error toasts still fire. Only the "the DB isn't even open" case is silenced in favor of the banner.

## Files touched

### Backend

- `frontend/src-tauri/src/state.rs` — remove or repurpose `AppState`. Introduce `DatabaseState` and `DatabaseStatus`. (Keep `AppState` if anything else depends on it; grep for `AppState` first.)
- `frontend/src-tauri/src/database/setup.rs` — make `initialize_database_on_startup` the transition owner. Add `retry_database_init` (or factor a shared private helper used by both).
- `frontend/src-tauri/src/database/commands.rs` — add `get_database_status`, `retry_database_init`. Convert each "use" command to use `require_db`.
- `frontend/src-tauri/src/lib.rs` — replace the `app.manage(AppState { db_manager })` calls (currently in `setup.rs` and `commands.rs`) with `app.manage(DatabaseState::initializing())` at boot, and let the init function set the state. Wire up the two new commands in the `invoke_handler`.

### Frontend

- New: `frontend/src/hooks/useDatabaseStatus.ts`
- New: `frontend/src/components/DatabaseErrorBanner.tsx`
- Modified: `frontend/src/app/layout.tsx` — mount the banner
- (No changes to existing first-launch listeners — they keep working)

### Docs

- `docs/superpowers/specs/2026-06-05-database-state-readiness-design.md` — this file
- `docs/superpowers/plans/2026-06-05-database-state-readiness.md` — implementation plan (created via writing-plans skill)

## Testing strategy

### Backend unit tests

- `DatabaseStatus` transitions: simulated init paths produce the expected status and `db_manager` presence.
- `require_db` returns `Ok(&manager)` when Ready with manager; returns the sentinel error when Unavailable; returns the "manager missing" error when Ready but no manager (defensive).
- Setup commands continue to work regardless of status (verified by calling them in both states in a test).

### Frontend component test

- Banner renders nothing when `is_ready`.
- Banner renders with "Pick a database" button when `reason` starts with "First launch".
- Banner renders with "Retry" button for any other reason.
- "Retry" calls `retryDatabaseInit()`; "Pick a database" calls `invoke('select_legacy_database_path')`.

### Manual test

1. Delete or rename the SQLite file at `~/Library/Application Support/com.meetily.ai/meeting_minutes.sqlite` (macOS path).
2. Launch the app. The banner should appear with the init-failure reason and a "Retry" button.
3. Click "Retry". It should re-attempt and fail again (the file is still missing). Banner persists.
4. Restore the SQLite file. Click "Retry". Banner should disappear.
5. On a clean machine, delete the DB and launch. Banner should show "First launch — pick a database" with a "Pick a database" button.
6. Click "Pick a database". File picker opens. Pick a folder containing a legacy DB or cancel. The flow continues as it does today.

## Open questions

None at design time. Implementation may surface details:

- Whether the existing `AppState { db_manager }` in `state.rs` is used anywhere besides `database/commands.rs` and `database/setup.rs`. If yes, keep `AppState` as-is and add `DatabaseState` alongside; if no, replace it.
- Whether `import_and_initialize_database` and `initialize_fresh_database` should be treated as "setup" (always allowed) or "use" (gated). They're in the setup list above — the rationale is the user calls them to *fix* the unavailable state, so they must be reachable. To be verified during implementation: do these commands currently assume `state.db_manager` is set? If yes, they need a small refactor (they need to construct their own manager, which is what the current code does — they call `DatabaseManager::new_from_app_handle` themselves, then `app.manage(AppState { db_manager })`. So they're already ungated in practice; we just need to update them to manage `DatabaseState` instead.)
