//! Opt-in local invocation history. The database is not automatically granted to guests.
use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DATABASE: &str = "executions.sqlite3";
const ENABLED: &str = "dashboard.enabled";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

// Short-lived locks coordinate reset with readers/writers across SAT processes.
// They are independent of SQLite's own transaction locks.
struct HistoryLock(File);
impl HistoryLock {
    fn acquire(home: &Path, exclusive: bool) -> Result<Self> {
        fs::create_dir_all(home)?;
        let file = private_file(&home.join("dashboard.lock"))?;
        if exclusive {
            FileExt::lock_exclusive(&file)?;
        } else {
            FileExt::lock_shared(&file)?;
        }
        Ok(Self(file))
    }
}
impl Drop for HistoryLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn database(home: &Path, create: bool) -> Result<Connection> {
    let path = home.join(DATABASE);
    if create {
        private_file(&path)?;
    }
    let flags = if create {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let db = Connection::open_with_flags(path, flags)?;
    db.busy_timeout(Duration::from_secs(5))?;
    if create {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS executions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token TEXT NOT NULL UNIQUE,
            tool TEXT NOT NULL,
            args_json TEXT NOT NULL,
            started_at_ms INTEGER NOT NULL,
            duration_ms INTEGER,
            status TEXT NOT NULL CHECK(status IN ('running', 'success', 'failed')),
            exit_code INTEGER,
            error TEXT
        );
        CREATE INDEX IF NOT EXISTS executions_tool_id ON executions(tool, id DESC);
        CREATE INDEX IF NOT EXISTS executions_status_id ON executions(status, id DESC);",
        )?;
    }
    Ok(db)
}

pub fn set_enabled(home: &Path, enabled: bool) -> Result<()> {
    let _lock = HistoryLock::acquire(home, true)?;
    if enabled {
        database(home, true)?;
        private_file(&home.join(ENABLED))?;
    } else {
        remove_if_exists(&home.join(ENABLED))?;
    }
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn reset(home: &Path) -> Result<()> {
    let _lock = HistoryLock::acquire(home, true)?;
    // No connections can remain open under the exclusive lock, including on Windows.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        remove_if_exists(&home.join(format!("{DATABASE}{suffix}")))?;
    }
    Ok(())
}

pub struct Invocation {
    home: PathBuf,
    token: String,
}
impl Invocation {
    pub fn begin(home: &Path, tool: &str, args: &[String]) -> Result<Option<Self>> {
        if !home.join(ENABLED).is_file() {
            return Ok(None);
        }
        let _lock = HistoryLock::acquire(home, false)?;
        if !home.join(ENABLED).is_file() {
            return Ok(None);
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let token = format!(
            "{}-{}-{}",
            std::process::id(),
            now.as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        database(home, true)?.execute(
            "INSERT INTO executions (token,tool,args_json,started_at_ms,status) VALUES (?1,?2,?3,?4,'running')",
            params![token, tool, serde_json::to_string(args)?, now.as_millis() as i64],
        )?;
        Ok(Some(Self {
            home: home.into(),
            token,
        }))
    }

    pub fn finish(self, duration: Duration, exit_code: u8, error: Option<&str>) -> Result<()> {
        let _lock = HistoryLock::acquire(&self.home, false)?;
        if !self.home.join(DATABASE).is_file() {
            return Ok(());
        }
        // A token prevents a pre-reset invocation from updating a reused row ID.
        database(&self.home, false)?.execute(
            "UPDATE executions SET duration_ms=?1,status=?2,exit_code=?3,error=?4 WHERE token=?5",
            params![
                duration.as_millis().min(i64::MAX as u128) as i64,
                if exit_code == 0 { "success" } else { "failed" },
                exit_code,
                error,
                self.token
            ],
        )?;
        Ok(())
    }
}

#[derive(Default, Serialize)]
pub struct Totals {
    pub executions: i64,
    pub successes: i64,
    pub failures: i64,
    pub running: i64,
    pub duration_ms: i64,
    pub tools: i64,
}
#[derive(Serialize)]
pub struct ToolUsage {
    pub name: String,
    pub count: i64,
    pub successes: i64,
    pub failures: i64,
    pub duration_ms: i64,
}
#[derive(Serialize)]
pub struct DailyUsage {
    pub day: String,
    pub count: i64,
}
#[derive(Serialize)]
pub struct Overview {
    pub enabled: bool,
    pub database: String,
    pub totals: Totals,
    pub tools: Vec<ToolUsage>,
    pub daily: Vec<DailyUsage>,
}

pub fn overview(home: &Path) -> Result<Overview> {
    let _lock = HistoryLock::acquire(home, false)?;
    let mut result = Overview {
        enabled: home.join(ENABLED).is_file(),
        database: home.join(DATABASE).display().to_string(),
        totals: Totals::default(),
        tools: vec![],
        daily: vec![],
    };
    if !home.join(DATABASE).is_file() {
        return Ok(result);
    }
    let db = database(home, false)?;
    result.totals = db.query_row(
        "SELECT COUNT(*), COALESCE(SUM(status='success'),0),
        COALESCE(SUM(status='failed'),0), COALESCE(SUM(status='running'),0),
        COALESCE(SUM(duration_ms),0), COUNT(DISTINCT tool) FROM executions",
        [],
        |row| {
            Ok(Totals {
                executions: row.get(0)?,
                successes: row.get(1)?,
                failures: row.get(2)?,
                running: row.get(3)?,
                duration_ms: row.get(4)?,
                tools: row.get(5)?,
            })
        },
    )?;
    result.tools = db
        .prepare(
            "SELECT tool,COUNT(*),SUM(status='success'),SUM(status='failed'),
        COALESCE(SUM(duration_ms),0) FROM executions GROUP BY tool ORDER BY COUNT(*) DESC, tool",
        )?
        .query_map([], |row| {
            Ok(ToolUsage {
                name: row.get(0)?,
                count: row.get(1)?,
                successes: row.get(2)?,
                failures: row.get(3)?,
                duration_ms: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    result.daily = db
        .prepare(
            "SELECT date(started_at_ms/1000,'unixepoch') AS day, COUNT(*)
        FROM executions WHERE started_at_ms >= (unixepoch('now','start of day','-6 days')*1000)
        GROUP BY day ORDER BY day",
        )?
        .query_map([], |row| {
            Ok(DailyUsage {
                day: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(result)
}

#[derive(Default)]
pub struct ExecutionQuery {
    pub tool: Option<String>,
    pub status: Option<String>,
    pub search: String,
    pub offset: u32,
    pub limit: u32,
}
#[derive(Serialize)]
pub struct Execution {
    pub id: i64,
    pub tool: String,
    pub args: Vec<String>,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub exit_code: Option<u8>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct ExecutionPage {
    pub total: i64,
    pub items: Vec<Execution>,
}

pub fn executions(home: &Path, query: &ExecutionQuery) -> Result<ExecutionPage> {
    let _lock = HistoryLock::acquire(home, false)?;
    if !home.join(DATABASE).is_file() {
        return Ok(ExecutionPage {
            total: 0,
            items: vec![],
        });
    }
    let db = database(home, false)?;
    let filter = " WHERE (?1 IS NULL OR tool=?1) AND (?2 IS NULL OR status=?2)
        AND (?3='' OR instr(lower(tool),lower(?3))>0 OR instr(lower(args_json),lower(?3))>0)";
    let total = db.query_row(
        &format!("SELECT COUNT(*) FROM executions{filter}"),
        params![query.tool, query.status, query.search],
        |row| row.get(0),
    )?;
    let records = db
        .prepare(&format!(
            "SELECT id,tool,args_json,started_at_ms,duration_ms,status,exit_code,error
        FROM executions{filter} ORDER BY id DESC LIMIT ?4 OFFSET ?5"
        ))?
        .query_map(
            params![
                query.tool,
                query.status,
                query.search,
                query.limit.clamp(1, 100),
                query.offset
            ],
            |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    Execution {
                        id: row.get(0)?,
                        tool: row.get(1)?,
                        args: vec![],
                        started_at_ms: row.get(3)?,
                        duration_ms: row.get(4)?,
                        status: row.get(5)?,
                        exit_code: row.get(6)?,
                        error: row.get(7)?,
                    },
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let items = records
        .into_iter()
        .map(|(args, mut record)| {
            record.args = serde_json::from_str(&args).context("invalid stored argument list")?;
            Ok(record)
        })
        .collect::<Result<_>>()?;
    Ok(ExecutionPage { total, items })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TempHome(PathBuf);
    impl TempHome {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                    "sat-history-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )))
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn opt_in_lifecycle_preserves_arguments_and_reset_preference() {
        let home = TempHome::new();
        let args = vec![
            "-c".into(),
            "print('日本語')\n# exact input".into(),
            "--read=a path::/data".into(),
        ];
        assert!(
            Invocation::begin(&home.0, "python", &args)
                .unwrap()
                .is_none()
        );
        assert!(!home.0.exists(), "disabled logging must not create state");
        set_enabled(&home.0, true).unwrap();
        let invocation = Invocation::begin(&home.0, "python", &args)
            .unwrap()
            .unwrap();
        assert_eq!(overview(&home.0).unwrap().totals.running, 1);
        invocation
            .finish(Duration::from_millis(123), 0, None)
            .unwrap();
        let failed = Invocation::begin(&home.0, "uutils", &["head".into()])
            .unwrap()
            .unwrap();
        failed
            .finish(Duration::from_millis(42), 2, Some("example error"))
            .unwrap();
        let summary = overview(&home.0).unwrap();
        assert_eq!(
            (
                summary.totals.executions,
                summary.totals.successes,
                summary.totals.failures,
                summary.totals.tools
            ),
            (2, 1, 1, 2)
        );
        assert_eq!(summary.totals.duration_ms, 165);
        let page = executions(
            &home.0,
            &ExecutionQuery {
                tool: Some("python".into()),
                limit: 25,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].args, args);
        assert_eq!(page.items[0].duration_ms, Some(123));
        let page = executions(
            &home.0,
            &ExecutionQuery {
                status: Some("failed".into()),
                search: "head".into(),
                limit: 25,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.items[0].exit_code, Some(2));
        set_enabled(&home.0, false).unwrap();
        assert!(
            Invocation::begin(&home.0, "ignored", &[])
                .unwrap()
                .is_none()
        );
        assert_eq!(overview(&home.0).unwrap().totals.executions, 2);
        reset(&home.0).unwrap();
        assert!(!home.0.join(DATABASE).exists());
        let summary = overview(&home.0).unwrap();
        assert!(!summary.enabled);
        assert_eq!(summary.totals.executions, 0);
        assert!(
            !home.0.join(DATABASE).exists(),
            "reading an empty dashboard must not recreate the database"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            set_enabled(&home.0, true).unwrap();
            assert_eq!(
                fs::metadata(home.0.join(DATABASE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn concurrent_invocations_and_reset_do_not_cross_update_records() {
        let home = TempHome::new();
        set_enabled(&home.0, true).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..12 {
                scope.spawn(|| {
                    Invocation::begin(&home.0, "parallel", &[])
                        .unwrap()
                        .unwrap()
                        .finish(Duration::from_millis(1), 0, None)
                        .unwrap();
                });
            }
        });
        assert_eq!(overview(&home.0).unwrap().totals.successes, 12);
        let old = Invocation::begin(&home.0, "old", &[]).unwrap().unwrap();
        reset(&home.0).unwrap();
        assert!(overview(&home.0).unwrap().enabled);
        assert!(!home.0.join(DATABASE).exists());
        let new = Invocation::begin(&home.0, "new", &[]).unwrap().unwrap();
        old.finish(Duration::from_secs(1), 1, Some("old failure"))
            .unwrap();
        let summary = overview(&home.0).unwrap();
        assert_eq!(summary.totals.executions, 1);
        assert_eq!(summary.totals.running, 1);
        new.finish(Duration::ZERO, 0, None).unwrap();
        assert_eq!(overview(&home.0).unwrap().totals.successes, 1);
    }
}
