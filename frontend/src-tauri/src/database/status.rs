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

/// Managed state that tracks database readiness and holds the pool.
///
/// Uses a single mutex over the (status, manager) pair so that transitions
/// between Ready ↔ Unavailable are always atomic — no TOCTOU window where
/// `require_db` could see `Ready` but find the manager already cleared.
pub struct DatabaseState {
    inner: Mutex<(DatabaseStatus, Option<DatabaseManager>)>,
}

impl DatabaseState {
    pub fn initializing() -> Self {
        Self {
            inner: Mutex::new((
                DatabaseStatus::Unavailable {
                    reason: "Initializing database…".to_string(),
                },
                None,
            )),
        }
    }

    pub fn ready(db_manager: DatabaseManager) -> Self {
        Self {
            inner: Mutex::new((DatabaseStatus::Ready, Some(db_manager))),
        }
    }

    pub fn unavailable(reason: String) -> Self {
        Self {
            inner: Mutex::new((DatabaseStatus::Unavailable { reason }, None)),
        }
    }

    /// Atomically transition to Ready — both fields update under one lock.
    pub fn set_ready(&self, db_manager: DatabaseManager) {
        let mut guard = self.inner.lock();
        *guard = (DatabaseStatus::Ready, Some(db_manager));
    }

    /// Atomically transition to Unavailable — clears the manager under one lock.
    pub fn set_unavailable(&self, reason: String) {
        let mut guard = self.inner.lock();
        *guard = (DatabaseStatus::Unavailable { reason }, None);
    }

    pub fn status(&self) -> DatabaseStatus {
        self.inner.lock().0.clone()
    }

    pub fn db_manager(&self) -> Option<DatabaseManager> {
        self.inner.lock().1.clone()
    }

    pub fn dto(&self) -> DatabaseStatusDto {
        DatabaseStatusDto::from(&self.status())
    }

    /// Convenience: get a Cloneable SqlitePool reference from the managed DB state.
    pub fn pool(&self) -> Result<sqlx::SqlitePool, String> {
        let mgr = require_db(self)?;
        Ok(mgr.pool())
    }
}

/// Atomically check readiness and return the manager in a single lock acquisition.
///
/// Returns `Err` if the status is Unavailable, or if the manager is missing
/// despite Ready status (defensive — should never happen in practice).
pub fn require_db(state: &DatabaseState) -> Result<DatabaseManager, String> {
    let guard = state.inner.lock();
    match &guard.0 {
        DatabaseStatus::Ready => {}
        DatabaseStatus::Unavailable { reason } => {
            return Err(format!("DATABASE_UNAVAILABLE: {}", reason));
        }
    }
    guard.1.clone()
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
            inner: parking_lot::Mutex::new((DatabaseStatus::Ready, None)),
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
