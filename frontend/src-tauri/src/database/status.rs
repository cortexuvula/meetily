use parking_lot::Mutex;
use serde::Serialize;

use crate::database::manager::DatabaseManager;

impl DatabaseState {
    /// Convenience: get a Cloneable SqlitePool reference from the managed DB state.
    pub fn pool(&self) -> Result<sqlx::SqlitePool, String> {
        let mgr = require_db(self)?;
        Ok(mgr.pool())
    }

    /// Returns the inner DatabaseManager if available.
    pub fn manager(&self) -> Option<DatabaseManager> {
        self.db_manager.lock().clone()
    }
}

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

/// Check the status flag first, then return a copy of the DB manager (DatabaseManager is Clone).
pub fn require_db(state: &DatabaseState) -> Result<DatabaseManager, String> {
    match &*state.status.lock() {
        DatabaseStatus::Ready => {},
        DatabaseStatus::Unavailable { reason } => {
            return Err(format!("DATABASE_UNAVAILABLE: {}", reason));
         }
       }
    state.db_manager.lock().clone()
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
