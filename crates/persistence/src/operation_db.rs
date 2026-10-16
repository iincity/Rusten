use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use application::{
    AppError, MaintenanceAuditLog, MaintenanceAuditRecord, OperationLog, OperationRecord,
};
use rusqlite::{Connection, params};

use crate::migrations::{INITIAL_SCHEMA, MIGRATION_2, MIGRATION_3};

pub struct OperationDb {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl OperationDb {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(&path).map_err(io::Error::other)?;
        connection
            .execute_batch(INITIAL_SCHEMA)
            .map_err(io::Error::other)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", params![], |row| row.get(0))
            .map_err(io::Error::other)?;
        if version < 1 {
            connection
                .execute_batch("PRAGMA user_version = 1;")
                .map_err(io::Error::other)?;
            connection.execute(
                "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(1, strftime('%s','now'))",
                params![],
            ).map_err(io::Error::other)?;
        }
        if version < 2 {
            connection
                .execute_batch(MIGRATION_2)
                .map_err(io::Error::other)?;
            connection
                .execute_batch("PRAGMA user_version = 2;")
                .map_err(io::Error::other)?;
            connection.execute(
                "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(2, strftime('%s','now'))",
                params![],
            ).map_err(io::Error::other)?;
        }
        if version < 3 {
            connection
                .execute_batch(MIGRATION_3)
                .map_err(io::Error::other)?;
            connection
                .execute_batch("PRAGMA user_version = 3;")
                .map_err(io::Error::other)?;
            connection
                .execute(
                    "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES(3, strftime('%s','now'))",
                    params![],
                )
                .map_err(io::Error::other)?;
        }
        Ok(Self {
            path,
            connection: Mutex::new(connection),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn count(&self) -> io::Result<u64> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| io::Error::other("operation database lock unavailable"))?;
        connection
            .query_row("SELECT COUNT(*) FROM operations", params![], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count.max(0) as u64)
            .map_err(io::Error::other)
    }

    pub fn maintenance_audits(&self) -> io::Result<Vec<MaintenanceAuditRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| io::Error::other("operation database lock unavailable"))?;
        let mut statement = connection
            .prepare(
                "SELECT operation_id, action, command, log_path, preflight, exit_code, result, timestamp FROM maintenance_audits ORDER BY timestamp, operation_id",
            )
            .map_err(io::Error::other)?;
        let rows = statement
            .query_map(params![], |row| {
                Ok(MaintenanceAuditRecord {
                    operation_id: row.get(0)?,
                    action: row.get(1)?,
                    command: row.get(2)?,
                    log_path: row.get(3)?,
                    preflight: row.get(4)?,
                    exit_code: row.get(5)?,
                    result: row.get(6)?,
                    timestamp: row.get::<_, i64>(7)?.max(0) as u64,
                })
            })
            .map_err(io::Error::other)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)
    }
}

impl OperationLog for OperationDb {
    fn append(&mut self, record: OperationRecord) -> Result<(), AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AppError::Log("operation database lock unavailable".into()))?;
        let size = i64::try_from(record.size)
            .map_err(|_| AppError::Log("operation size exceeds SQLite integer range".into()))?;
        let timestamp = i64::try_from(record.timestamp).map_err(|_| {
            AppError::Log("operation timestamp exceeds SQLite integer range".into())
        })?;
        connection.execute(
            "INSERT INTO operations(operation_id, rule_id, path, file_identity, backup_id, size, timestamp, result) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![record.operation_id, record.rule_id, record.path, record.file_identity, record.backup_id, size, timestamp, record.result],
        ).map(|_| ()).map_err(|error| AppError::Log(error.to_string()))
    }
}

impl MaintenanceAuditLog for OperationDb {
    fn append_maintenance(&mut self, record: MaintenanceAuditRecord) -> Result<(), AppError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AppError::Log("operation database lock unavailable".into()))?;
        let timestamp = i64::try_from(record.timestamp).map_err(|_| {
            AppError::Log("maintenance timestamp exceeds SQLite integer range".into())
        })?;
        let exit_code = record.exit_code.map(i64::from);
        connection
            .execute(
                "INSERT INTO maintenance_audits(operation_id, action, command, log_path, preflight, exit_code, result, timestamp) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![record.operation_id, record.action, record.command, record.log_path, record.preflight, exit_code, record.result, timestamp],
            )
            .map(|_| ())
            .map_err(|error| AppError::Log(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::OperationLog;

    #[test]
    fn opens_schema_and_appends_structured_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("operations.sqlite");
        let mut database = OperationDb::open(&path).unwrap();
        database
            .append(OperationRecord {
                operation_id: "op-1".into(),
                rule_id: Some("temp".into()),
                path: "C:\\Temp\\a.tmp".into(),
                file_identity: "1:2".into(),
                backup_id: None,
                size: 4,
                timestamp: 5,
                result: "MovedToRecycleBin".into(),
            })
            .unwrap();
        assert_eq!(database.count().unwrap(), 1);
        assert!(path.is_file());
    }
}
