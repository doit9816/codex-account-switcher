use chrono::Utc;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_PROVIDER_ID: &str = "openai";
const STATE_DB_FILE: &str = "state_5.sqlite";
const SQLITE_DIR_NAME: &str = "sqlite";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SessionVisibilityRepairReport {
    pub(crate) target_provider: String,
    pub(crate) checked_databases: usize,
    pub(crate) updated_rows: usize,
    pub(crate) skipped_databases: usize,
}

impl SessionVisibilityRepairReport {
    pub(crate) fn changed(&self) -> bool {
        self.updated_rows > 0
    }

    pub(crate) fn summary(&self) -> String {
        if self.updated_rows == 0 {
            if self.skipped_databases == 0 {
                format!(
                    "Codex 会话可见性已是当前 provider ({})",
                    self.target_provider
                )
            } else {
                format!(
                    "Codex 会话可见性无可写变化，跳过 {} 个不可用数据库",
                    self.skipped_databases
                )
            }
        } else if self.skipped_databases == 0 {
            format!(
                "已修复 {} 条 Codex 会话可见性记录到 provider {}",
                self.updated_rows, self.target_provider
            )
        } else {
            format!(
                "已修复 {} 条 Codex 会话可见性记录到 provider {}，跳过 {} 个不可用数据库",
                self.updated_rows, self.target_provider, self.skipped_databases
            )
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ThreadsTableColumns {
    model_provider: bool,
    has_user_event: bool,
    first_user_message: bool,
    thread_source: bool,
}

pub(crate) fn repair_session_visibility_for_current_provider(
    codex_home: &Path,
) -> Result<SessionVisibilityRepairReport, String> {
    let target_provider = read_target_provider(codex_home)?;
    repair_session_visibility(codex_home, &target_provider)
}

pub(crate) fn repair_session_visibility(
    codex_home: &Path,
    target_provider: &str,
) -> Result<SessionVisibilityRepairReport, String> {
    validate_provider_id(target_provider)?;
    let mut report = SessionVisibilityRepairReport {
        target_provider: target_provider.to_string(),
        ..SessionVisibilityRepairReport::default()
    };

    for db_path in official_state_db_candidate_paths(codex_home) {
        if !db_path.exists() {
            continue;
        }
        report.checked_databases += 1;
        match repair_session_visibility_db(&db_path, target_provider) {
            Ok(updated) => report.updated_rows += updated,
            Err(error) if is_unusable_sqlite_error_text(&error) => {
                report.skipped_databases += 1;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(report)
}

pub(crate) fn read_target_provider(codex_home: &Path) -> Result<String, String> {
    let config_path = codex_home.join("config.toml");
    let Ok(content) = fs::read_to_string(&config_path) else {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    };
    if content.trim().is_empty() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }
    let document = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("config.toml 解析失败: {error}"))?;
    let provider = document
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn repair_session_visibility_db(db_path: &Path, target_provider: &str) -> Result<usize, String> {
    reject_symlink_if_exists(db_path)?;
    let mut connection = Connection::open(db_path).map_err(|error| {
        format!(
            "打开 Codex 会话数据库失败 ({}): {}",
            db_path.display(),
            error
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| format!("设置 Codex 会话数据库超时失败: {error}"))?;

    let Some(columns) = read_threads_table_columns(&connection).map_err(|error| {
        format!(
            "读取 Codex threads 表结构失败 ({}): {}",
            db_path.display(),
            error
        )
    })?
    else {
        return Ok(0);
    };
    let Some(where_clause) = build_threads_repair_where_clause(columns) else {
        return Ok(0);
    };
    let set_clause = build_threads_repair_set_clause(columns);
    if set_clause.is_empty() {
        return Ok(0);
    }

    let count_sql = format!("SELECT COUNT(*) FROM threads WHERE {where_clause}");
    let rows_to_update: usize = if columns.model_provider {
        connection
            .query_row(count_sql.as_str(), [target_provider], |row| row.get(0))
            .map_err(|error| format_sqlite_write_error(db_path, &error))?
    } else {
        connection
            .query_row(count_sql.as_str(), [], |row| row.get(0))
            .map_err(|error| format_sqlite_write_error(db_path, &error))?
    };
    if rows_to_update == 0 {
        return Ok(0);
    }
    backup_state_db(db_path)?;

    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let sql = format!("UPDATE threads SET {set_clause} WHERE {where_clause}");
    let updated_rows = if columns.model_provider {
        transaction
            .execute(sql.as_str(), [target_provider])
            .map_err(|error| format_sqlite_write_error(db_path, &error))?
    } else {
        transaction
            .execute(sql.as_str(), [])
            .map_err(|error| format_sqlite_write_error(db_path, &error))?
    };
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updated_rows)
}

fn backup_state_db(db_path: &Path) -> Result<(), String> {
    let parent = db_path
        .parent()
        .ok_or_else(|| format!("无法定位 Codex 会话数据库目录: {}", db_path.display()))?;
    let backup_dir = parent.join(".account-switcher-backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("创建 Codex 会话数据库备份目录失败: {error}"))?;
    let file_stem = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state_5.sqlite");
    let backup_path = backup_dir.join(format!(
        "{}.session-visibility-{}.backup",
        file_stem,
        Utc::now().format("%Y%m%d%H%M%S")
    ));
    fs::copy(db_path, &backup_path).map_err(|error| {
        format!(
            "备份 Codex 会话数据库失败 ({} -> {}): {}",
            db_path.display(),
            backup_path.display(),
            error
        )
    })?;
    Ok(())
}

fn read_threads_table_columns(
    connection: &Connection,
) -> Result<Option<ThreadsTableColumns>, rusqlite::Error> {
    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut names = Vec::new();
    for row in rows {
        names.push(row?);
    }
    if names.is_empty() {
        return Ok(None);
    }
    Ok(Some(ThreadsTableColumns {
        model_provider: names.iter().any(|name| name == "model_provider"),
        has_user_event: names.iter().any(|name| name == "has_user_event"),
        first_user_message: names.iter().any(|name| name == "first_user_message"),
        thread_source: names.iter().any(|name| name == "thread_source"),
    }))
}

fn build_threads_repair_where_clause(columns: ThreadsTableColumns) -> Option<String> {
    let mut predicates = Vec::new();
    if columns.model_provider {
        predicates.push("COALESCE(model_provider, '') <> ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(has_user_event, 0) <> 1)");
    }
    if columns.thread_source && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(thread_source, '') = '')");
    }
    if predicates.is_empty() {
        None
    } else {
        Some(predicates.join(" OR "))
    }
}

fn build_threads_repair_set_clause(columns: ThreadsTableColumns) -> String {
    let mut assignments = Vec::new();
    if columns.model_provider {
        assignments.push("model_provider = ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        assignments.push(
            "has_user_event = CASE WHEN COALESCE(first_user_message, '') <> '' THEN 1 ELSE has_user_event END",
        );
    }
    if columns.thread_source && columns.first_user_message {
        assignments.push(
            "thread_source = CASE WHEN COALESCE(thread_source, '') = '' AND COALESCE(first_user_message, '') <> '' THEN 'user' ELSE thread_source END",
        );
    }
    assignments.join(", ")
}

fn official_state_db_candidate_paths(codex_home: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique_path(
        &mut paths,
        codex_home.join(SQLITE_DIR_NAME).join(STATE_DB_FILE),
    );
    push_unique_path(&mut paths, codex_home.join(STATE_DB_FILE));
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 Codex 会话数据库: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "读取 Codex 会话数据库路径属性失败 ({}): {}",
            path.display(),
            error
        )),
    }
}

fn validate_provider_id(provider_id: &str) -> Result<(), String> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err("provider 不能为空".to_string());
    }
    if trimmed != provider_id || trimmed.len() > 200 || trimmed.chars().any(char::is_control) {
        return Err("provider 包含非法字符".to_string());
    }
    Ok(())
}

fn is_missing_threads_table_error(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table: threads")
}

fn is_unusable_sqlite_error_text(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("file is not a database")
        || lower.contains("database disk image is malformed")
        || lower.contains("not an error")
}

fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("database is locked") || lower.contains("database busy") {
        return format!(
            "Codex 会话数据库正被占用，请关闭 Codex 后重试 ({}): {}",
            path.display(),
            message
        );
    }
    format!(
        "更新 Codex 会话可见性失败 ({}): {}",
        path.display(),
        message
    )
}

pub(crate) fn is_conversation_metadata_path(path: &str) -> bool {
    let first = path.split('/').next().unwrap_or(path);
    matches!(
        first,
        "sessions"
            | "archived_sessions"
            | SESSION_INDEX_FILE
            | "logs_2.sqlite"
            | "logs_2.sqlite-shm"
            | "logs_2.sqlite-wal"
            | STATE_DB_FILE
            | "state_5.sqlite-shm"
            | "state_5.sqlite-wal"
            | SQLITE_DIR_NAME
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_threads_db(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES
                 ('old-visible', 'old-provider', 1, 'hello', 'user'),
                 ('same-provider', 'new-provider', 1, 'hello', 'user'),
                 ('hidden', 'old-provider', 0, 'hi', '')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn repairs_state_db_provider_and_visibility_flags() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join(STATE_DB_FILE);
        create_threads_db(&db_path);

        let report = repair_session_visibility(dir.path(), "new-provider").unwrap();
        assert_eq!(report.updated_rows, 2);
        assert!(report.changed());

        let connection = Connection::open(db_path).unwrap();
        let rows = connection
            .prepare(
                "SELECT id, model_provider, has_user_event, thread_source FROM threads ORDER BY id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "hidden".to_string(),
                    "new-provider".to_string(),
                    1,
                    "user".to_string()
                ),
                (
                    "old-visible".to_string(),
                    "new-provider".to_string(),
                    1,
                    "user".to_string()
                ),
                (
                    "same-provider".to_string(),
                    "new-provider".to_string(),
                    1,
                    "user".to_string()
                ),
            ]
        );
    }

    #[test]
    fn reads_target_provider_from_config_or_defaults_to_openai() {
        let dir = tempdir().unwrap();
        assert_eq!(read_target_provider(dir.path()).unwrap(), "openai");
        fs::write(
            dir.path().join("config.toml"),
            "model_provider = \"custom\"\n",
        )
        .unwrap();
        assert_eq!(read_target_provider(dir.path()).unwrap(), "custom");
    }

    #[test]
    fn detects_conversation_metadata_paths() {
        assert!(is_conversation_metadata_path("sessions/rollout.jsonl"));
        assert!(is_conversation_metadata_path("archived_sessions/old.jsonl"));
        assert!(is_conversation_metadata_path("sqlite/state_5.sqlite"));
        assert!(is_conversation_metadata_path("state_5.sqlite"));
        assert!(!is_conversation_metadata_path("memories/user.md"));
    }
}
