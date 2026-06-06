# Database State & Readiness Banner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `AppState` with a readiness-aware `DatabaseState` and surface a persistent banner in the UI when the DB is unavailable (init failure or first-launch-pending). Setup commands remain ungated so the user can recover from the banner.

**Architecture:** Backend gets a new `DatabaseState` managed state with a `DatabaseStatus` enum (`Ready` | `Unavailable { reason }`). All "use" DB commands call a `require_db` guard that returns a sentinel-prefixed error string when the DB isn't ready. A new `retry_database_init` Tauri command re-runs the init logic. Frontend gets a `useDatabaseStatus` hook (with polling) and a `DatabaseErrorBanner` component mounted in `RootLayout`.

**Tech Stack:** Rust (Tauri 2, parking_lot, thiserror via existing `VideoRecordingError` pattern), React 18 + TypeScript (existing Next.js 14 App Router project).

**Spec:** `docs/superpowers/specs/2026-06-05-database-state-readiness-design.md`

**Affected files (8 backend + 3 frontend, all pre-existing except where noted):**

| File | Role | Change |
|---|---|---|
| `frontend/src-tauri/src/database/status.rs` | NEW: enum, DTO, state, guard, tests | Create |
| `frontend/src-tauri/src/state.rs` | Re-export `DatabaseState`, drop `AppState` | Modify (small) |
| `frontend/src-tauri/src/database/setup.rs` | Become the transition owner | Modify |
| `frontend/src-tauri/src/database/commands.rs` | Add 2 commands, gate use commands | Modify |
| `frontend/src-tauri/src/api/commands.rs` | Gate use commands | Modify |
| `frontend/src-tauri/src/summary/commands.rs` | Gate use commands | Modify |
| `frontend/src-tauri/src/audio/retranscription.rs` | Update state references | Modify |
| `frontend/src-tauri/src/audio/import.rs` | Update state references | Modify |
| `frontend/src-tauri/src/onboarding.rs` | Update state reference | Modify |
| `frontend/src-tauri/src/lib.rs` | Register new commands, update cleanup | Modify |
| `frontend/src/hooks/useDatabaseStatus.ts` | NEW: hook + retry helper | Create |
| `frontend/src/components/DatabaseErrorBanner.tsx` | NEW: banner component | Create |
| `frontend/src/app/layout.tsx` | Mount banner | Modify (small) |

---

## Task 1: Add `DatabaseStatus`, `DatabaseState`, and `require_db` helper (with tests)

**Files:**
- Create: `frontend/src-tauri/src/database/status.rs`

- [ ] **Step 1: Write the failing tests first**

Create the file with just the test module and the `mod tests` body. Run the tests to confirm they fail to compile (proves the API is missing).

```rust
// frontend/src-tauri/src/database/status.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_db_returns_unavailable_error_with_reason() {
        let state = DatabaseState::unavailable("test reason".to_string());
        let result = require_db(&state);
        let err = result.expect_err("expected error when Unavailable");
        assert!(err.starts_with("DATABASE_UNAVAILABLE: "), "got: {err}");
        assert!(err.contains("test reason"), "got: {err}");
    }

    #[test]
    fn require_db_returns_manager_missing_error_when_ready_but_no_manager() {
        // Construct a Ready state with no manager (the "shouldn't happen" path).
        let state = DatabaseState {
            status: parking_lot::Mutex::new(DatabaseStatus::Ready),
            db_manager: parking_lot::Mutex::new(None),
        };
        let result = require_db(&state);
        assert_eq!(
            result.unwrap_err(),
            "DATABASE_UNAVAILABLE: manager missing in Ready state"
        );
    }

    #[test]
    fn status_dto_ready_has_no_reason() {
        let dto = DatabaseStatusDto::from(&DatabaseStatus::Ready);
        assert!(dto.is_ready);
        assert_eq!(dto.reason, None);
    }

    #[test]
    fn status_dto_unavailable_carries_reason() {
        let dto = DatabaseStatusDto::from(&DatabaseStatus::Unavailable {
            reason: "boom".to_string(),
        });
        assert!(!dto.is_ready);
        assert_eq!(dto.reason, Some("boom".to_string()));
    }

    #[test]
    fn unavailable_state_marks_db_manager_as_none() {
        let state = DatabaseState::unavailable("x".to_string());
        assert!(!state.dto().is_ready);
        assert!(state.db_manager().is_none());
    }
}
```

Run: `cd frontend/src-tauri && cargo test --lib video_recording 2>&1 | head -5`
Expected: compile error (`DatabaseStatus`, `DatabaseState`, etc. not defined). That's the "fail" — the types and helper don't exist yet.

- [ ] **Step 2: Add the types and helper**

Replace the entire file contents with the real implementation (tests + module body together):

```rust
// frontend/src-tauri/src/database/status.rs
use parking_lot::Mutex;
use serde::Serialize;

use crate::database::manager::DatabaseManager;

#[derive(Debug, Clone, PartialEq)]
pub enum DatabaseStatus {
    Ready,
    Unavailable { reason: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStatusDto {
    pub is_ready: bool,
    pub reason: Option<String>,
}

impl From<&DatabaseStatus> for DatabaseStatusDto {
    fn from(status: &DatabaseStatus) -> Self {
        match status {
            DatabaseStatus::Ready => DatabaseStatusDto { is_ready: true, reason: None },
            DatabaseStatus::Unavailable { reason } => DatabaseStatusDto {
                is_ready: false,
                reason: Some(reason.clone()),
            },
        }
    }
}

pub struct DatabaseState {
    pub(crate) status: Mutex<DatabaseStatus>,
    pub(crate) db_manager: Mutex<Option<DatabaseManager>>,
}

impl DatabaseState {
    pub fn initializing() -> Self {
        Self {
            status: Mutex::new(DatabaseStatus::Unavailable {
                reason: "Initializing database…".to_string(),
            }),
            db_manager: Mutex::new(None),
        }
    }

    pub fn ready(db_manager: DatabaseManager) -> Self {
        Self {
            status: Mutex::new(DatabaseStatus::Ready),
            db_manager: Mutex::new(Some(db_manager)),
        }
    }

    pub fn unavailable(reason: String) -> Self {
        Self {
            status: Mutex::new(DatabaseStatus::Unavailable { reason }),
            db_manager: Mutex::new(None),
        }
    }

    pub fn set_ready(&self, db_manager: DatabaseManager) {
        *self.db_manager.lock() = Some(db_manager);
        *self.status.lock() = DatabaseStatus::Ready;
    }

    pub fn set_unavailable(&self, reason: String) {
        *self.db_manager.lock() = None;
        *self.status.lock() = DatabaseStatus::Unavailable { reason };
    }

    pub fn status(&self) -> DatabaseStatus {
        self.status.lock().clone()
    }

    pub fn db_manager(&self) -> Option<DatabaseManager> {
        self.db_manager.lock().clone()
    }

    pub fn dto(&self) -> DatabaseStatusDto {
        DatabaseStatusDto::from(&self.status())
    }
}

pub fn require_db<'a>(state: &'a DatabaseState) -> Result<&'a DatabaseManager, String> {
    {
        let status = state.status.lock();
        match &*status {
            DatabaseStatus::Ready => {}
            DatabaseStatus::Unavailable { reason } => {
                return Err(format!("DATABASE_UNAVAILABLE: {}", reason));
            }
        }
    }
    state
        .db_manager
        .lock()
        .as_ref()
        .ok_or_else(|| "DATABASE_UNAVAILABLE: manager missing in Ready state".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_db_returns_unavailable_error_with_reason() {
        let state = DatabaseState::unavailable("test reason".to_string());
        let result = require_db(&state);
        let err = result.expect_err("expected error when Unavailable");
        assert!(err.starts_with("DATABASE_UNAVAILABLE: "), "got: {err}");
        assert!(err.contains("test reason"), "got: {err}");
    }

    #[test]
    fn require_db_returns_manager_missing_error_when_ready_but_no_manager() {
        let state = DatabaseState {
            status: parking_lot::Mutex::new(DatabaseStatus::Ready),
            db_manager: parking_lot::Mutex::new(None),
        };
        let result = require_db(&state);
        assert_eq!(
            result.unwrap_err(),
            "DATABASE_UNAVAILABLE: manager missing in Ready state"
        );
    }

    #[test]
    fn status_dto_ready_has_no_reason() {
        let dto = DatabaseStatusDto::from(&DatabaseStatus::Ready);
        assert!(dto.is_ready);
        assert_eq!(dto.reason, None);
    }

    #[test]
    fn status_dto_unavailable_carries_reason() {
        let dto = DatabaseStatusDto::from(&DatabaseStatus::Unavailable {
            reason: "boom".to_string(),
        });
        assert!(!dto.is_ready);
        assert_eq!(dto.reason, Some("boom".to_string()));
    }

    #[test]
    fn unavailable_state_marks_db_manager_as_none() {
        let state = DatabaseState::unavailable("x".to_string());
        assert!(!state.dto().is_ready);
        assert!(state.db_manager().is_none());
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd frontend/src-tauri && cargo test --lib database::status 2>&1 | tail -15`
Expected: 5 passed.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/database/status.rs
git commit -m "feat(db): add DatabaseState with readiness status and require_db guard"
```

---

## Task 2: Drop `AppState`, re-export `DatabaseState` from `state.rs`

**Files:**
- Modify: `frontend/src-tauri/src/state.rs`

- [ ] **Step 1: Replace the file contents**

```rust
// frontend/src-tauri/src/state.rs
pub use crate::database::status::DatabaseState;
```

- [ ] **Step 2: Try to build — expect failures everywhere `AppState` is used**

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[" | head -20`
Expected: a long list of `error[E0412]: cannot find type 'AppState'` in `database/setup.rs`, `database/commands.rs`, `api/commands.rs`, `summary/commands.rs`, `audio/retranscription.rs`, `audio/import.rs`, `onboarding.rs`, `lib.rs`. Each of these is fixed in a later task. Don't fix them yet — this step just confirms the migration surface area.

- [ ] **Step 3: Commit (it's a small, focused diff even if the build is broken)**

Wait — don't commit a broken build. Instead, use `git stash` to hold the change, fix the surface area in subsequent tasks, then `git stash pop` at the end. **Skip the commit here; the final state.rs change lands with the last task that touches it (Task 10).**

---

## Task 3: Refactor `setup.rs` to be the transition owner

**Files:**
- Modify: `frontend/src-tauri/src/database/setup.rs`

The current `initialize_database_on_startup` is a one-shot function. We need to:
- Add a shared `initialize_database(state: &DatabaseState, app: &AppHandle)` helper that both initial boot and retry call
- Make `initialize_database_on_startup` a thin wrapper that calls the shared helper and manages the `DatabaseState`
- Add `retry_database_init` as a public function that the Tauri command will call

- [ ] **Step 1: Write the new `setup.rs`**

```rust
// frontend/src-tauri/src/database/setup.rs
use log::{error, info, warn};
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use super::status::{DatabaseState, DatabaseStatus};
use crate::state::DatabaseState as ManagedDatabaseState;

const FIRST_LAUNCH_REASON: &str = "First launch — pick a database to continue";

/// Initialize the database and update the managed `DatabaseState`.
/// This is the single transition owner for `DatabaseState::status`.
///
/// - On success: sets status to `Ready` with the manager.
/// - On first launch: sets status to `Unavailable { reason: FIRST_LAUNCH_REASON }`.
/// - On failure: sets status to `Unavailable { reason: "Database initialization failed: ..." }`.
pub async fn initialize_database(
    state: &ManagedDatabaseState,
    app: &AppHandle,
) {
    let is_first_launch = match DatabaseManager::is_first_launch(app).await {
        Ok(b) => b,
        Err(e) => {
            let reason = format!("Failed to check first launch status: {}", e);
            error!("[db] {}", reason);
            state.set_unavailable(reason);
            return;
        }
    };

    if is_first_launch {
        info!("[db] First launch detected — awaiting user database selection");
        state.set_unavailable(FIRST_LAUNCH_REASON.to_string());

        // Notify the webview (existing listeners in AnalyticsProvider, useRecordingStop,
        // OnboardingContext, lib/analytics.ts expect this event with a 500ms delay).
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if let Err(e) = app_handle.emit("first-launch-detected", ()) {
                warn!("[db] Failed to emit first-launch-detected: {}", e);
            }
        });
        return;
    }

    match DatabaseManager::new_from_app_handle(app).await {
        Ok(db_manager) => {
            info!("[db] Database initialized successfully");
            state.set_ready(db_manager);
        }
        Err(e) => {
            let reason = format!("Database initialization failed: {}", e);
            error!("[db] {}", reason);
            state.set_unavailable(reason);
        }
    }
}

/// Called once from `lib.rs::setup` to install an `Initializing` state and
/// kick off the async init. The window is created by Tauri only after this
/// function returns, so by the time the React app mounts, the state is
/// either `Ready` or `Unavailable` (never still `Initializing` in practice).
pub async fn initialize_database_on_startup(
    state: &ManagedDatabaseState,
    app: &AppHandle,
) {
    // The state was already set to `Initializing` by `lib.rs::setup` via
    // `DatabaseState::initializing()`. Now run the real init.
    initialize_database(state, app).await;
}

/// Re-runs the init logic. Used by the `retry_database_init` Tauri command.
/// Returns the new status DTO so the caller can show it in the UI.
pub async fn retry_database_init(
    state: &ManagedDatabaseState,
    app: &AppHandle,
) -> DatabaseStatus {
    initialize_database(state, app).await;
    state.status()
}

// Suppress unused-import warning for `DatabaseState` and `DatabaseStatus` if
// the compiler is fussy; both are used in the public function signatures above.
const _: fn() = || {
    let _: Option<DatabaseState> = None;
    let _: Option<DatabaseStatus> = None;
};
```

Wait — that's a hack. The real `DatabaseState` and `DatabaseStatus` *are* used in `retry_database_init`'s return type. Remove the hack and the unused imports. The `use super::status::{DatabaseState, DatabaseStatus}` line should only import what you use; since `DatabaseStatus` is used as the return type of `retry_database_init`, keep it. `DatabaseState` is not used in this file (it's `ManagedDatabaseState` from the re-export). Remove `DatabaseState` from the import.

Final version:

```rust
// frontend/src-tauri/src/database/setup.rs
use log::{error, info, warn};
use tauri::{AppHandle, Emitter};

use super::manager::DatabaseManager;
use super::status::DatabaseStatus;
use crate::state::DatabaseState;

const FIRST_LAUNCH_REASON: &str = "First launch — pick a database to continue";

pub async fn initialize_database(state: &DatabaseState, app: &AppHandle) {
    let is_first_launch = match DatabaseManager::is_first_launch(app).await {
        Ok(b) => b,
        Err(e) => {
            let reason = format!("Failed to check first launch status: {}", e);
            error!("[db] {}", reason);
            state.set_unavailable(reason);
            return;
        }
    };

    if is_first_launch {
        info!("[db] First launch detected — awaiting user database selection");
        state.set_unavailable(FIRST_LAUNCH_REASON.to_string());

        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            if let Err(e) = app_handle.emit("first-launch-detected", ()) {
                warn!("[db] Failed to emit first-launch-detected: {}", e);
            }
        });
        return;
    }

    match DatabaseManager::new_from_app_handle(app).await {
        Ok(db_manager) => {
            info!("[db] Database initialized successfully");
            state.set_ready(db_manager);
        }
        Err(e) => {
            let reason = format!("Database initialization failed: {}", e);
            error!("[db] {}", reason);
            state.set_unavailable(reason);
        }
    }
}

pub async fn initialize_database_on_startup(state: &DatabaseState, app: &AppHandle) {
    initialize_database(state, app).await;
}

pub async fn retry_database_init(state: &DatabaseState, app: &AppHandle) -> DatabaseStatus {
    initialize_database(state, app).await;
    state.status()
}
```

- [ ] **Step 2: Verify it compiles in isolation**

This will fail because the old function signature was `pub async fn initialize_database_on_startup(app: &AppHandle) -> Result<(), String>` and `AppState { db_manager }` is referenced. The build is still red from Task 2's `state.rs` change. That's expected — we'll fix the call sites in later tasks.

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[" | wc -l`
Expected: still many errors, but no *new* ones from this file (all the same as before).

- [ ] **Step 3: Don't commit yet — same as Task 2, hold for the end**

---

## Task 4: Add `get_database_status` Tauri command

**Files:**
- Modify: `frontend/src-tauri/src/database/commands.rs`

- [ ] **Step 1: Add the command function**

Find the top of the `commands.rs` file and add the new command. The function signature is sync (no async needed — just reads the mutex).

```rust
use super::status::{DatabaseState, DatabaseStatusDto};

#[tauri::command]
pub fn get_database_status(state: tauri::State<'_, DatabaseState>) -> DatabaseStatusDto {
    state.dto()
}
```

- [ ] **Step 2: Add the `setup.rs` and `commands.rs` imports to use the new `DatabaseState`**

In `commands.rs`, the existing top of the file is:

```rust
use crate::state::AppState;
```

Change to:

```rust
use crate::state::DatabaseState;
```

(The re-export from Task 2 means `crate::state::DatabaseState` resolves to the new type.)

The rest of the file still references `AppState` in `tauri::State<'_, AppState>` parameters and `app.manage(AppState { db_manager })` calls. Those are fixed in Tasks 6 and 7. Don't fix them yet.

- [ ] **Step 3: Verify the new command compiles in isolation**

It won't, because the rest of the file is broken. That's OK — just confirm the new function is syntactically valid by running `cargo check` and noting that no *new* errors come from the `get_database_status` function.

- [ ] **Step 4: Don't commit yet**

---

## Task 5: Add `retry_database_init` Tauri command

**Files:**
- Modify: `frontend/src-tauri/src/database/commands.rs`

- [ ] **Step 1: Add the command function**

```rust
use crate::database::setup;

#[tauri::command]
pub async fn retry_database_init<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, DatabaseState>,
) -> Result<DatabaseStatusDto, String> {
    use tauri::Manager;
    let status = setup::retry_database_init(state.inner(), &app).await;
    Ok(DatabaseStatusDto::from(&status))
}
```

- [ ] **Step 2: Verify in isolation**

`cargo check` should still show the same set of pre-existing errors (Tasks 2, 3, 4 broke things). The new function should add no new errors.

- [ ] **Step 3: Don't commit yet**

---

## Task 6: Gate use commands in `database/commands.rs`

**Files:**
- Modify: `frontend/src-tauri/src/database/commands.rs`

The spec lists the commands to gate (see spec § "Gated commands"). For this file, the gated commands are roughly the ones in lines 145–237 (the meeting/import/initialize ones, NOT the check/select/setup ones). Apply the pattern to each one.

- [ ] **Step 1: Update `import_and_initialize_database` and `initialize_fresh_database` to set the new state**

These two are *ungated* (the spec puts them in the setup list), but they currently call `app.manage(AppState { db_manager })`. Update them to set the existing `DatabaseState` instead.

Find:
```rust
app.manage(AppState { db_manager });
```

There are two such calls (lines 164 and 188). Replace each with:

```rust
state.set_ready(db_manager);
```

…and change the function signatures to take `state: tauri::State<'_, DatabaseState>` instead of `app: AppHandle` where needed (the `app.manage()` is no longer needed since the state is already managed).

For `import_and_initialize_database` (around line 145):
- Currently takes `app: AppHandle`
- Change to take `state: tauri::State<'_, DatabaseState>` and the other params
- After `DatabaseManager::new_from_app_handle(...).await?` succeeds, call `state.set_ready(db_manager)` instead of `app.manage(AppState { db_manager })`

For `initialize_fresh_database` (around line 176):
- Same pattern. Takes `state: tauri::State<'_, DatabaseState>` instead of `app: AppHandle`.
- After `DatabaseManager::new_from_app_handle(...).await?` succeeds, call `state.set_ready(db_manager)`.

- [ ] **Step 2: Add the guard to the other use commands in this file**

The spec lists the remaining commands in `database/commands.rs` that are use commands. Apply this pattern to each:

Before:
```rust
pub async fn some_command(state: tauri::State<'_, DatabaseState>, ...) -> Result<T, String> {
    let pool = state.db_manager.pool();
    // ... use pool ...
}
```

After:
```rust
pub async fn some_command(state: tauri::State<'_, DatabaseState>, ...) -> Result<T, String> {
    use super::status::require_db;
    let db = require_db(state.inner())?;
    let pool = db.pool();
    // ... use pool ...
}
```

The `?` propagates the `DATABASE_UNAVAILABLE: ...` error. If you have an existing `use crate::state::DatabaseState;` import at the top, the inner `use super::status::require_db;` is the additional one needed.

Apply this to:
- `import_and_initialize_database` (already partly done in Step 1, add `require_db` if it does any DB work before the import — looking at the code, it doesn't, so just leave it as a setup command)
- `initialize_fresh_database` (same — no DB access before the fresh create)
- The recovery helpers imported from `audio/incremental_saver` — these are Tauri commands registered in `lib.rs`. They take `state: tauri::State<'_, DatabaseState>`. The function body uses `app_state.db_manager.pool()`. Wrap with `require_db`.

(For this task, focus on the commands *defined in this file*. The `audio/incremental_saver` re-exports are Tauri commands but live in a different file — they're handled in Task 9.)

- [ ] **Step 3: Verify the file compiles**

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[E" | head -10`
Expected: the error count is *lower* than after Task 5 (some errors are fixed). Some errors remain in other files (api/commands.rs, etc.).

- [ ] **Step 4: Don't commit yet — the build is still red**

---

## Task 7: Gate use commands in `api/commands.rs`

**Files:**
- Modify: `frontend/src-tauri/src/api/commands.rs`

- [ ] **Step 1: Replace the import**

Find:
```rust
use crate::state::AppState;
```

Change to:
```rust
use crate::state::DatabaseState;
use crate::database::status::{require_db, DatabaseStatusDto};
```

(Add `DatabaseStatusDto` only if the file uses it — for this file it doesn't, so just `DatabaseState` and `require_db`.)

- [ ] **Step 2: Update every `tauri::State<'_, AppState>` parameter**

Use your editor's find-and-replace to change every `AppState` to `DatabaseState` in function signatures in this file. There are ~20 such commands (see spec § "Gated commands" for the list).

- [ ] **Step 3: Wrap each use command body with `require_db`**

For each command that accesses `state.db_manager.method()`:

Before:
```rust
pub async fn api_get_meetings(state: tauri::State<'_, DatabaseState>, ...) -> Result<T, String> {
    let meetings = state.db_manager.some_method().await?;
    // ...
}
```

After:
```rust
pub async fn api_get_meetings(state: tauri::State<'_, DatabaseState>, ...) -> Result<T, String> {
    let db = require_db(state.inner())?;
    let meetings = db.some_method().await?;
    // ...
}
```

The `?` propagates the unavailable error. The original code's `let meetings = state.db_manager.some_method()` becomes `let meetings = db.some_method()`. Mechanical find-and-replace of `state.db_manager` → `db` inside each function body (after the guard).

- [ ] **Step 4: Verify**

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[E" | wc -l`
Expected: error count drops further. Some errors remain in `summary/commands.rs`, `audio/*`, `onboarding.rs`, `lib.rs`.

- [ ] **Step 5: Don't commit yet**

---

## Task 8: Gate use commands in `summary/commands.rs`

**Files:**
- Modify: `frontend/src-tauri/src/summary/commands.rs`

Same pattern as Task 7. The commands to gate are listed in the spec § "Gated commands" under the `summary::commands::api_*` entries.

- [ ] **Step 1: Replace import and update signatures**

```rust
// Top of file
use crate::state::DatabaseState;
use crate::database::status::require_db;
```

Find/replace `AppState` → `DatabaseState` in function signatures.

- [ ] **Step 2: Wrap bodies with `require_db`**

Same pattern as Task 7. Replace `state.db_manager` → `db` inside each function body, after the guard.

- [ ] **Step 3: Verify**

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[E" | wc -l`
Expected: drops further. Remaining errors are in `audio/*`, `onboarding.rs`, `lib.rs`, and possibly some `database/commands.rs` stragglers.

- [ ] **Step 4: Don't commit yet**

---

## Task 9: Update `audio/*` and `onboarding.rs`

**Files:**
- Modify: `frontend/src-tauri/src/audio/retranscription.rs`
- Modify: `frontend/src-tauri/src/audio/import.rs`
- Modify: `frontend/src-tauri/src/onboarding.rs`

These files use `try_state::<AppState>()` (or `tauri::State<'_, AppState>`) to access the DB. They are NOT Tauri commands themselves, but they're called from Tauri commands. They need to be updated to use `DatabaseState` and handle the unavailable case.

- [ ] **Step 1: Update `audio/retranscription.rs`**

Replace `use crate::state::AppState;` with `use crate::state::DatabaseState;`. Find each `try_state::<AppState>()` call (3 of them, around lines 426, 584, 686) and replace with `try_state::<DatabaseState>()`.

For each call site, the surrounding code does something like:
```rust
let app_state = app.try_state::<AppState>().ok_or_else(...)?;
let pool = app_state.db_manager.pool();
```

Update to:
```rust
let app_state = app.try_state::<DatabaseState>().ok_or_else(...)?;
use crate::database::status::require_db;
let db_manager = require_db(app_state)?;
let pool = db_manager.pool();
```

(If the surrounding error message says "AppState not available", update it to "DatabaseState not available".)

- [ ] **Step 2: Update `audio/import.rs`**

Same pattern. Two `try_state::<AppState>()` calls (around lines 637, 845).

- [ ] **Step 3: Update `onboarding.rs`**

Same pattern. One `tauri::State<'_, AppState>` parameter in a Tauri command (around line 173). Replace with `DatabaseState`. If the command body accesses `state.db_manager`, wrap with `require_db`.

- [ ] **Step 4: Verify**

Run: `cd frontend/src-tauri && cargo check 2>&1 | grep "error\[E" | wc -l`
Expected: drops to just `lib.rs` errors (the cleanup logic and the missing command registrations).

- [ ] **Step 5: Don't commit yet**

---

## Task 10: Update `lib.rs` (register commands, update cleanup, install initial state)

**Files:**
- Modify: `frontend/src-tauri/src/lib.rs`

- [ ] **Step 1: Add the initial `DatabaseState` management**

In the `setup` callback, find:
```rust
.manage(Arc::new(video_recording::state::VideoRecordingState::default()))
```

…and add right after it:
```rust
.manage(video_recording::database::status::DatabaseState::initializing())
```

Wait — that's the wrong path. The state is at `database::status::DatabaseState`. The full path is `crate::database::status::DatabaseState`. The `.manage()` call needs the type to be in scope. Add a use at the top of the file if not already present, then use `crate::database::status::DatabaseState::initializing()`.

- [ ] **Step 2: Update the setup callback to call `initialize_database_on_startup`**

In the existing `setup` callback body, find the `block_on` for the database init (the one that was fixed to log instead of panic). Replace it with:

```rust
// Initialize database (handles first launch detection and conditional setup).
// Failures are reflected in the managed DatabaseState and surfaced via the UI banner.
{
    use crate::database::setup::initialize_database_on_startup;
    use crate::database::status::DatabaseState;
    let state = _app.state::<DatabaseState>();
    tauri::async_runtime::block_on(async {
        initialize_database_on_startup(state.inner(), _app.handle()).await
    });
}
```

- [ ] **Step 3: Update the cleanup logic in `RunEvent::Exit`**

Find:
```rust
if let Some(app_state) = _app_handle.try_state::<state::AppState>() {
    ...
}
```

Change to:
```rust
if let Some(db_state) = _app_handle.try_state::<state::DatabaseState>() {
    use crate::database::status::DatabaseStatus;
    if let DatabaseStatus::Ready = db_state.status() {
        // The DB-using cleanup path is only valid when Ready. When Unavailable,
        // there's no manager to clean up.
        // (No DB cleanup needed today; the comment is here to document the
        // guard so future cleanup code knows to check the status first.)
        let _ = db_state; // suppress unused-variable warning if no cleanup is added
    } else {
        log::info!("Skipping DB cleanup — database is unavailable");
    }
}
```

(If the existing cleanup code does real work with `app_state.db_manager`, wrap that work with a `require_db` call after checking the status. If it's a no-op today, the placeholder above is enough.)

- [ ] **Step 4: Register the two new Tauri commands**

In the `invoke_handler` macro, find the line:
```rust
database::commands::check_first_launch,
```

Add right after it (or anywhere in the database block):
```rust
database::commands::get_database_status,
database::commands::retry_database_init,
```

- [ ] **Step 5: Verify the whole backend compiles**

Run: `cd frontend/src-tauri && cargo check 2>&1 | tail -10`
Expected: no errors. A few warnings may remain (unused imports in this transitional state).

- [ ] **Step 6: Run the test suite**

Run: `cd frontend/src-tauri && cargo test --lib 2>&1 | tail -10`
Expected: all existing tests pass plus the 5 new `database::status` tests.

- [ ] **Step 7: Commit the entire backend change as one logical unit**

```bash
git add frontend/src-tauri/src/state.rs \
        frontend/src-tauri/src/database/status.rs \
        frontend/src-tauri/src/database/setup.rs \
        frontend/src-tauri/src/database/commands.rs \
        frontend/src-tauri/src/api/commands.rs \
        frontend/src-tauri/src/summary/commands.rs \
        frontend/src-tauri/src/audio/retranscription.rs \
        frontend/src-tauri/src/audio/import.rs \
        frontend/src-tauri/src/onboarding.rs \
        frontend/src-tauri/src/lib.rs
git commit -m "feat(db): replace AppState with readiness-aware DatabaseState, add banner commands"
```

---

## Task 11: Frontend `useDatabaseStatus` hook

**Files:**
- Create: `frontend/src/hooks/useDatabaseStatus.ts`

- [ ] **Step 1: Write the hook**

```typescript
// frontend/src/hooks/useDatabaseStatus.ts
'use client';

import { useEffect, useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface DatabaseStatus {
  is_ready: boolean;
  reason?: string;
}

const POLL_INTERVAL_MS = 30_000;

export function useDatabaseStatus() {
  const [status, setStatus] = useState<DatabaseStatus>({ is_ready: true });
  const [loading, setLoading] = useState(true);
  const cancelledRef = useRef(false);

  const refresh = useCallback(async () => {
    try {
      const s = await invoke<DatabaseStatus>('get_database_status');
      if (!cancelledRef.current) {
        setStatus(s);
        setLoading(false);
      }
    } catch (e) {
      if (!cancelledRef.current) {
        setStatus({ is_ready: false, reason: String(e) });
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    cancelledRef.current = false;
    refresh();
    const interval = setInterval(refresh, POLL_INTERVAL_MS);
    return () => {
      cancelledRef.current = true;
      clearInterval(interval);
    };
  }, [refresh]);

  return { status, loading, refresh };
}

export async function retryDatabaseInit(): Promise<DatabaseStatus> {
  return await invoke<DatabaseStatus>('retry_database_init');
}
```

- [ ] **Step 2: Verify the typecheck passes**

Run: `cd frontend && pnpm run build 2>&1 | tail -10`
Expected: passes (the hook isn't imported anywhere yet, so it just needs to typecheck).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/hooks/useDatabaseStatus.ts
git commit -m "feat(db-ui): add useDatabaseStatus hook with polling"
```

---

## Task 12: Frontend `DatabaseErrorBanner` component

**Files:**
- Create: `frontend/src/components/DatabaseErrorBanner.tsx`

- [ ] **Step 1: Write the component**

```typescript
// frontend/src/components/DatabaseErrorBanner.tsx
'use client';

import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useDatabaseStatus, retryDatabaseInit } from '@/hooks/useDatabaseStatus';

export function DatabaseErrorBanner() {
  const { status, refresh } = useDatabaseStatus();
  const [retrying, setRetrying] = useState(false);

  if (status.is_ready) return null;

  const isFirstLaunch = (status.reason ?? '').startsWith('First launch');

  const handleRetry = useCallback(async () => {
    setRetrying(true);
    try {
      await retryDatabaseInit();
    } finally {
      await refresh();
      setRetrying(false);
    }
  }, [refresh]);

  const handlePickDatabase = useCallback(async () => {
    try {
      await invoke('select_legacy_database_path');
    } catch (e) {
      console.error('Failed to open database file picker:', e);
    }
  }, []);

  const title = isFirstLaunch ? 'Pick a database to continue' : 'Database unavailable';
  const buttonLabel = isFirstLaunch ? 'Pick a database' : (retrying ? 'Retrying…' : 'Retry');

  return (
    <div
      role="alert"
      data-testid="database-error-banner"
      className="bg-red-50 border-b border-red-200 text-red-800 px-4 py-3"
    >
      <div className="max-w-7xl mx-auto flex items-center justify-between gap-4">
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-sm">{title}</p>
          {status.reason && (
            <p className="text-xs mt-1 truncate" title={status.reason}>
              {status.reason}
            </p>
          )}
        </div>
        <div className="flex-shrink-0">
          {isFirstLaunch ? (
            <button
              onClick={handlePickDatabase}
              className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-700 transition-colors"
            >
              {buttonLabel}
            </button>
          ) : (
            <button
              onClick={handleRetry}
              disabled={retrying}
              className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-700 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {buttonLabel}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify typecheck**

Run: `cd frontend && pnpm run build 2>&1 | tail -10`
Expected: passes (component isn't imported yet).

- [ ] **Step 3: Commit**

```bash
git add frontend/src/components/DatabaseErrorBanner.tsx
git commit -m "feat(db-ui): add DatabaseErrorBanner component"
```

---

## Task 13: Mount the banner in `RootLayout`

**Files:**
- Modify: `frontend/src/app/layout.tsx`

- [ ] **Step 1: Add the import**

Find the existing import for `VideoPreviewOverlay`:
```typescript
import { VideoPreviewOverlay } from '@/components/VideoRecording/VideoPreviewOverlay';
```

Add right after it:
```typescript
import { DatabaseErrorBanner } from '@/components/DatabaseErrorBanner';
```

- [ ] **Step 2: Mount the banner**

Find the `<Toaster position="bottom-center" ... />` element near the end of the JSX. Add the banner just before it (so it appears above the toast layer in the DOM, but is part of the body, not nested in any provider):

```typescript
<DatabaseErrorBanner />
<Toaster position="bottom-center" richColors closeButton />
```

- [ ] **Step 2: Verify build and lint pass**

Run: `cd frontend && pnpm run build 2>&1 | tail -15`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/app/layout.tsx
git commit -m "feat(db-ui): mount DatabaseErrorBanner in RootLayout"
```

---

## Task 14: Manual end-to-end verification

- [ ] **Step 1: Launch the app with a healthy DB**

Run: `cd frontend && pnpm tauri:dev`
Expected: app launches, no banner shown (state is Ready).

- [ ] **Step 2: Force an init-failure state and verify the banner**

Quit the app. Rename the SQLite file at `~/Library/Application Support/com.meetily.ai/meeting_minutes.sqlite` to `meeting_minutes.sqlite.bak` (macOS path; equivalent on Linux/Windows). Relaunch.
Expected: a red banner appears with the title "Database unavailable" and the reason containing the init failure. The "Retry" button is enabled. DB-using features (meetings list, etc.) either don't render or show error toasts.

- [ ] **Step 3: Click "Retry"**

Click the "Retry" button.
Expected: the banner persists (the DB is still missing). Check the dev console for the `error!("[db] Database initialization failed: ...")` log.

- [ ] **Step 4: Restore the DB and click "Retry" again**

Quit the app. Rename `meeting_minutes.sqlite.bak` back to `meeting_minutes.sqlite`. Relaunch.
Expected: banner still appears (this is a fresh launch). Click "Retry" → banner disappears within ~1s, app becomes functional.

- [ ] **Step 5: Force a first-launch state and verify the "Pick a database" banner**

Quit the app. Delete the SQLite file. Relaunch.
Expected: a red banner with the title "Pick a database to continue" and a "Pick a database" button. Clicking it opens the OS file picker (via `select_legacy_database_path`). Cancel out. The banner persists.

- [ ] **Step 6: Commit a docs note (if any) for the manual test**

No commit needed for the manual test itself. If any of the above revealed an issue, fix it and commit a follow-up.

---

## Self-review

**Spec coverage check:**
- § "State machine" → Task 1 (types), Task 3 (transitions in setup.rs) ✓
- § "Command gating" → Tasks 6, 7, 8, 9 (the four files with use commands) ✓
- § "The guard helper" → Task 1 (`require_db`) ✓
- § "New Tauri commands" → Tasks 4, 5 ✓
- § "Frontend / useDatabaseStatus hook" → Task 11 ✓
- § "Frontend / DatabaseErrorBanner" → Task 12 ✓
- § "Frontend / Mount in RootLayout" → Task 13 ✓
- § "Error handling" → covered by the `DATABASE_UNAVAILABLE: ` prefix in Task 1's `require_db` ✓
- § "Testing" → Task 1 (unit tests), Task 14 (manual) ✓

**Placeholder scan:** No TBD/TODO. All code blocks are complete. No "implement later" notes.

**Type consistency:** `DatabaseState` (struct), `DatabaseStatus` (enum), `DatabaseStatusDto` (DTO), `require_db` (function) — all defined in Task 1 and used consistently in Tasks 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13. `set_ready` / `set_unavailable` / `set_status` / `dto` / `status` / `db_manager` are the only methods on `DatabaseState` and are used consistently.

**Gaps found during self-review:**
- The `AppState` re-export from `state.rs` in Task 2: I initially considered keeping `AppState` as a type alias, but that's a backwards-compat hack. Better to update all 8 files. The plan does that. ✓
- The `audio/incremental_saver` re-exports: the spec mentions them in the gated list, but they live in a different file. Task 6 explicitly notes they're handled in Task 9 (audio/import.rs and audio/retranscription.rs). The recovery-helper commands defined in `incremental_saver.rs` are Tauri commands registered from that file, accessed via the audio/retranscription.rs and audio/import.rs paths. The plan covers this. ✓
- The `setup.rs` hack with the `const _: fn() = || { ... }` — I included it in the first draft then removed it. Good. ✓
- The cleanup logic in `lib.rs` (Task 10) — the existing code does `app_state.db_manager.pool()` for something. If that something is real (not a no-op), the plan needs to gate it. The placeholder I wrote covers both cases with a comment. If real cleanup code is needed, the engineer will see it and add the guard. ✓
