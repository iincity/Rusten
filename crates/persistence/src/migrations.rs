pub const INITIAL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT PRIMARY KEY,
    rule_id TEXT,
    path TEXT NOT NULL,
    file_identity TEXT NOT NULL,
    size INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    result TEXT NOT NULL
);
";

pub const MIGRATION_2: &str = "ALTER TABLE operations ADD COLUMN backup_id TEXT;";

pub const MIGRATION_3: &str = "
CREATE TABLE IF NOT EXISTS maintenance_audits (
    operation_id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    command TEXT NOT NULL,
    log_path TEXT,
    preflight TEXT NOT NULL,
    exit_code INTEGER,
    result TEXT NOT NULL,
    timestamp INTEGER NOT NULL
);
";
