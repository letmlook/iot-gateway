//! 历史数据（本地时序存储）。
//!
//! # 定位与边界（写在最前面，避免被误用）
//!
//! 这是**单机、小规模**的历史存储：把采集值批量写进一个独立的 SQLite 文件
//! （`<data_dir>/history.db`，与配置库分开，便于单独备份/清理），提供按时间范围的
//! 降采样查询，用于管理台画趋势图和事后追查。
//!
//! 它**不是**时序数据库：
//! - 写入路径是「内存缓冲 + 定期批量事务」，靠 `max_rows` + 保留时长控制磁盘占用；
//! - 查询走 SQL 分桶聚合，适合「看趋势」，不适合高频点查与复杂分析；
//! - 需要长期保留、高写入速率或多维分析时，应导出到外部 TSDB（本模块不承担该职责）。
//!
//! # 数据来源
//!
//! 通过 [`Bus::subscribe_all`] 的旁路订阅拿到**所有**分组的发布，因此不需要维护订阅关系；
//! 旁路消费者跟不上时会按通道容量丢弃（`Lagged`），并计入 `history_write_err` 之外的
//! `history_lagged` 指标，不会拖慢主链路。
//!
//! # 线程模型
//!
//! ```text
//! tokio 任务（消费旁路订阅） --tokio mpsc--> 写线程（rusqlite，批量事务）
//! ```
//!
//! 写线程是普通 std 线程：SQLite 是阻塞 API，放在 tokio 运行时里会占住 worker；
//! 用独立线程反而让「异步采集」与「同步落库」各司其职。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gateway_core::{Bus, DataFlowMetrics};
use gateway_sdk::{GroupData, TagId};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing::{info, warn};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS samples (
  ts        INTEGER NOT NULL,
  node_id   TEXT    NOT NULL,
  group_id  TEXT    NOT NULL,
  tag       TEXT    NOT NULL,
  value     REAL    NOT NULL,
  tenant_id TEXT    NOT NULL DEFAULT 'default'
);
"#;

/// 旧索引名（v1）：迁移时删除，由 idx_samples_series_t 取代
const LEGACY_INDEX: &str = "idx_samples_series";
/// 现行索引：域为最左前缀，服务「按域过滤的序列查询」形状
const SERIES_INDEX: &str = "idx_samples_series_t";

/// 建库 / 迁移（幂等）：
/// - 新库：v2 形状直接建立；
/// - v1 旧库：samples 无 tenant_id 列 → ALTER 补列（存量样本归 default 域），
///   删除旧索引并按 (tenant_id, node_id, group_id, tag, ts) 重建。
pub fn ensure_history_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    let has_tenant: bool = conn
        .prepare("PRAGMA table_info(samples)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .any(|c| c.as_deref() == Ok("tenant_id"));
    if !has_tenant {
        conn.execute_batch(
            "ALTER TABLE samples ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
             DROP INDEX IF EXISTS idx_samples_series;",
        )?;
    }
    if LEGACY_INDEX != SERIES_INDEX {
        conn.execute_batch(&format!("DROP INDEX IF EXISTS {LEGACY_INDEX};"))?;
    }
    conn.execute_batch(&format!(
        "CREATE INDEX IF NOT EXISTS {SERIES_INDEX} ON samples(tenant_id, node_id, group_id, tag, ts);"
    ))?;
    Ok(())
}

/// 采样点
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub ts_ms: i64,
    pub node_id: String,
    pub group_id: String,
    pub tag: String,
    pub value: f64,
    /// 写入时从节点解析的租户域（节点删除后历史仍按保留策略留存，故冗余落列）
    pub tenant_id: String,
}

/// 历史存储配置
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    pub enabled: bool,
    pub db_path: PathBuf,
    /// 保留时长（小时）
    pub retention_hours: u64,
    /// 落盘批间隔（毫秒）
    pub flush_ms: u64,
    /// 单次事务最大批量
    pub batch_size: usize,
    /// 行数上限；超过后从最旧的开始删除
    pub max_rows: u64,
    /// 内存在途样本上限（超过则对旁路消费者形成背压）
    pub channel_capacity: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: PathBuf::from("data/history.db"),
            retention_hours: 72,
            flush_ms: 1000,
            batch_size: 500,
            max_rows: 5_000_000,
            channel_capacity: 8192,
        }
    }
}

/// 运行时统计。
///
/// 计数值同时镜像到 `DataFlowMetrics`，`/api/history/stats` 与 Prometheus 指标都从那里读，
/// 避免出现两套口径。
#[derive(Debug, Default)]
pub struct HistoryStats {
    pub rows_written: AtomicU64,
    pub rows_pruned: AtomicU64,
    pub write_errors: AtomicU64,
    /// 旁路订阅丢帧次数（写线程跟不上时，订阅端按容量丢弃）
    pub lagged: AtomicU64,
}

/// 租户解析器：node_id（字符串）→ 该节点归属的租户域；None = 节点不存在（样本丢弃）
pub type HistoryTenantResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// 历史存储句柄：持有写线程，Drop 时通知其退出
pub struct HistoryRecorder {
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HistoryRecorder {
    /// 启动历史落库：订阅总线旁路，把数值型点位写入 SQLite。
    ///
    /// `bus` 为 `Some` 时挂旁路订阅；为 `None` 时不订阅（仅初始化库，便于测试与迁移）。
    /// `tenant_resolver` 为 `Some` 时按节点盖章租户域（节点缺失则丢弃样本并计 write_err）；
    /// 为 `None` 时一律归 default 域（仅测试/迁移场景）。
    pub fn start(
        cfg: HistoryConfig,
        bus: Option<Bus>,
        metrics: Arc<DataFlowMetrics>,
        stats: Arc<HistoryStats>,
        tenant_resolver: Option<HistoryTenantResolver>,
    ) -> Result<Self, String> {
        if let Some(parent) = cfg.db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // 先在启动线程把库建好：配置错误要在启动阶段暴露，而不是在写线程里静默失败
        {
            let conn = open_db(&cfg.db_path).map_err(|e| e.to_string())?;
            ensure_history_schema(&conn).map_err(|e| e.to_string())?;
        }

        let (tx, rx) = mpsc::channel::<Sample>(cfg.channel_capacity);
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

        // 写线程：阻塞式 SQLite，批量事务 + 定期清理
        let write_cfg = cfg.clone();
        let write_stats = stats.clone();
        let write_metrics = metrics.clone();
        let thread = std::thread::Builder::new()
            .name("gateway-history".to_string())
            .spawn(move || {
                write_loop(write_cfg, rx, write_stats, write_metrics);
            })
            .map_err(|e| e.to_string())?;

        // 订阅任务：把旁路订阅转换为样本流
        if let Some(bus) = bus {
            let (tap_id, mut tap_rx) = bus.subscribe_all();
            let sub_stats = stats.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => break,
                        msg = tap_rx.recv() => match msg {
                            Ok(data) => {
                                for mut s in samples_from_group_data(&data) {
                                    // 租户盖章：节点缺失则丢弃样本并计入写错误
                                    match tenant_resolver.as_ref().and_then(|r| r(&s.node_id)) {
                                        Some(t) => s.tenant_id = t,
                                        None if tenant_resolver.is_some() => {
                                            sub_stats.write_errors.fetch_add(1, Ordering::Relaxed);
                                            continue;
                                        }
                                        None => s.tenant_id = "default".to_string(),
                                    }
                                    // 写线程跟不上时在这里形成背压：通道满会等待，
                                    // 等待期间旁路订阅自身按容量丢弃并计数（见 Lagged 分支）
                                    if tx.send(s).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                sub_stats.lagged.fetch_add(n, Ordering::Relaxed);
                            }
                            Err(_) => break,
                        }
                    }
                }
                bus.unsubscribe_all(tap_id);
            });
        }

        Ok(Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        })
    }
}

impl Drop for HistoryRecorder {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// 把一条 GroupData 转成样本：只保留数值型点位，且需要知道点位名称。
/// tenant_id 不在此处盖章——由订阅任务经 resolver 解析后回填（节点缺失则整条丢弃）。
pub fn samples_from_group_data(data: &GroupData) -> Vec<Sample> {
    let Some(names) = data.tag_names.as_ref() else {
        // 没有点位名映射时无法建立稳定的序列标识，跳过（避免用 tag_id 当序列名）
        return Vec::new();
    };
    let ts_ms = data.ts.timestamp_millis();
    let node_id = data.node_id.0.to_string();
    let group_id = data.group_id.0.to_string();
    let mut out = Vec::with_capacity(data.values.len());
    for (tid, v) in &data.values {
        let Some(name) = tag_name(names, tid) else {
            continue;
        };
        if let Some(f) = numeric(v) {
            out.push(Sample {
                ts_ms,
                node_id: node_id.clone(),
                group_id: group_id.clone(),
                tag: name,
                value: f,
                tenant_id: String::new(),
            });
        }
    }
    out
}

fn tag_name(names: &std::collections::HashMap<TagId, String>, tid: &TagId) -> Option<String> {
    // tag_names 的 key 是 TagId，直接按 key 取即可
    names.get(tid).cloned()
}

/// 数值转换：布尔按 0/1，字符串与非有限值不计入历史
fn numeric(v: &gateway_sdk::types::DataValue) -> Option<f64> {
    use gateway_sdk::types::DataValue::*;
    let f = match v {
        Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => match v.as_f64() {
            Some(x) => x,
            None => v.as_i64().map(|i| i as f64)?,
        },
    };
    f.is_finite().then_some(f)
}

fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    // 与配置库同样的参数：WAL（读写不互相阻塞）+ busy_timeout。
    // 用 execute_batch 而非 pragma_update：journal_mode 会返回一行结果。
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

fn write_loop(
    cfg: HistoryConfig,
    mut rx: mpsc::Receiver<Sample>,
    stats: Arc<HistoryStats>,
    metrics: Arc<DataFlowMetrics>,
) {
    let conn = match open_db(&cfg.db_path).and_then(|c| {
        ensure_history_schema(&c)?;
        Ok(c)
    }) {
        Ok(c) => c,
        Err(e) => {
            warn!(path = %cfg.db_path.display(), "history db open failed: {}", e);
            return;
        }
    };
    info!(
        path = %cfg.db_path.display(),
        retention_hours = cfg.retention_hours,
        flush_ms = cfg.flush_ms,
        "history recorder started"
    );

    let mut batch: Vec<Sample> = Vec::with_capacity(cfg.batch_size);
    let mut last_prune = Instant::now();
    let flush_every = Duration::from_millis(cfg.flush_ms.max(50));

    loop {
        // 有数据就收，没数据就等一个 flush 间隔
        let got = rx.blocking_recv();
        let deadline = Instant::now() + flush_every;
        match got {
            Some(s) => batch.push(s),
            None => break, // 通道关闭（进程退出）
        }
        while batch.len() < cfg.batch_size {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.try_recv() {
                Ok(s) => batch.push(s),
                Err(_) => break,
            }
        }

        if !batch.is_empty() {
            match write_batch(&conn, &batch) {
                Ok(n) => {
                    stats.rows_written.fetch_add(n, Ordering::Relaxed);
                    metrics.history_rows_written.fetch_add(n, Ordering::Relaxed);
                }
                Err(e) => {
                    stats.write_errors.fetch_add(1, Ordering::Relaxed);
                    metrics.history_write_err.fetch_add(1, Ordering::Relaxed);
                    warn!("history write failed: {}", e);
                }
            }
            batch.clear();
        }

        if last_prune.elapsed() >= Duration::from_secs(60) {
            last_prune = Instant::now();
            match prune(&conn, &cfg) {
                Ok(n) if n > 0 => {
                    stats.rows_pruned.fetch_add(n, Ordering::Relaxed);
                    metrics.history_rows_pruned.fetch_add(n, Ordering::Relaxed);
                    info!(rows = n, "history pruned");
                }
                Ok(_) => {}
                Err(e) => warn!("history prune failed: {}", e),
            }
        }
    }

    // 退出前把最后一批落盘
    if !batch.is_empty() {
        if let Ok(n) = write_batch(&conn, &batch) {
            stats.rows_written.fetch_add(n, Ordering::Relaxed);
        }
    }
    info!("history recorder stopped");
}

fn write_batch(conn: &Connection, batch: &[Sample]) -> rusqlite::Result<u64> {
    let tx = conn.unchecked_transaction()?;
    {
        // 批量写入同样使用语句缓存：每条样本都重新 prepare 会让吞吐下降一个数量级
        let mut stmt = tx.prepare_cached(
            "INSERT INTO samples (ts, node_id, group_id, tag, value, tenant_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for s in batch {
            stmt.execute(params![
                s.ts_ms,
                s.node_id,
                s.group_id,
                s.tag,
                s.value,
                s.tenant_id
            ])?;
        }
    }
    tx.commit()?;
    Ok(batch.len() as u64)
}

/// 按保留时长与行数上限清理历史
fn prune(conn: &Connection, cfg: &HistoryConfig) -> rusqlite::Result<u64> {
    let mut removed = 0u64;
    let cutoff = now_ms() - (cfg.retention_hours as i64) * 3_600_000;
    removed += conn.execute("DELETE FROM samples WHERE ts < ?1", params![cutoff])? as u64;

    // 行数上限：分批删除最旧的，避免一次删太多长时间持锁
    let total: i64 = conn.query_row("SELECT COUNT(1) FROM samples", [], |r| r.get(0))?;
    if (total as u64) > cfg.max_rows {
        let excess = (total as u64).saturating_sub(cfg.max_rows);
        let mut left = excess.min(200_000);
        while left > 0 {
            let step = left.min(50_000);
            let n = conn.execute(
                "DELETE FROM samples WHERE rowid IN (SELECT rowid FROM samples ORDER BY ts ASC LIMIT ?1)",
                params![step as i64],
            )?;
            removed += n as u64;
            left = left.saturating_sub(step);
            if n == 0 {
                break;
            }
        }
    }
    Ok(removed)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------- 查询 ----------

/// 一个降采样桶
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Bucket {
    /// 桶起始时间（毫秒）
    pub ts: i64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub count: u64,
}

/// 查询参数
#[derive(Debug, Clone)]
pub struct SeriesQuery {
    pub node_id: String,
    pub group_id: String,
    pub tag: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub max_points: u32,
    /// 租户域过滤："*" 表示不过滤（仅 All 作用域传入），其余按域精确匹配
    pub tenant_id: String,
}

/// 桶宽度：把区间切成不超过 `max_points` 个桶，最小 1ms
pub fn bucket_ms(from_ms: i64, to_ms: i64, max_points: u32) -> i64 {
    let span = (to_ms - from_ms).max(1);
    let n = max_points.max(1) as i64;
    ((span + n - 1) / n).max(1)
}

/// 查询降采样序列：SQL 侧分桶聚合，返回的点数不超过 `max_points`
pub fn query_series(db_path: &Path, q: &SeriesQuery) -> Result<Vec<Bucket>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let width = bucket_ms(q.from_ms, q.to_ms, q.max_points);
    let mut stmt = conn
        .prepare(
            "SELECT (ts / ?4) * ?4 AS b, AVG(value), MIN(value), MAX(value), COUNT(1)
             FROM samples
             WHERE node_id = ?1 AND group_id = ?2 AND tag = ?3 AND ts >= ?5 AND ts <= ?6
               AND (?7 = '*' OR tenant_id = ?7)
             GROUP BY b
             ORDER BY b ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                q.node_id,
                q.group_id,
                q.tag,
                width,
                q.from_ms,
                q.to_ms,
                q.tenant_id
            ],
            |row| {
                Ok(Bucket {
                    ts: row.get(0)?,
                    avg: row.get(1)?,
                    min: row.get(2)?,
                    max: row.get(3)?,
                    count: row.get::<_, i64>(4)? as u64,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// 库里有哪些序列（点位名），用于前端下拉选择；按租户域过滤（"*" = 不过滤）
pub fn list_series(
    db_path: &Path,
    limit: u32,
    tenant_id: &str,
) -> Result<Vec<serde_json::Value>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT node_id, group_id, tag, COUNT(1), MIN(ts), MAX(ts)
             FROM samples
             WHERE (?2 = '*' OR tenant_id = ?2)
             GROUP BY node_id, group_id, tag
             ORDER BY MAX(ts) DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit as i64, tenant_id], |row| {
            Ok(serde_json::json!({
                "node_id": row.get::<_, String>(0)?,
                "group_id": row.get::<_, String>(1)?,
                "tag": row.get::<_, String>(2)?,
                "count": row.get::<_, i64>(3)?,
                "first_ts": row.get::<_, i64>(4)?,
                "last_ts": row.get::<_, i64>(5)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// 库级统计
pub fn db_stats(db_path: &Path, cfg: &HistoryConfig) -> Result<serde_json::Value, String> {
    let mut rows = 0i64;
    let mut oldest: Option<i64> = None;
    let mut newest: Option<i64> = None;
    if db_path.exists() {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        rows = conn
            .query_row("SELECT COUNT(1) FROM samples", [], |r| r.get(0))
            .unwrap_or(0);
        oldest = conn
            .query_row("SELECT MIN(ts) FROM samples", [], |r| r.get(0))
            .ok()
            .flatten();
        newest = conn
            .query_row("SELECT MAX(ts) FROM samples", [], |r| r.get(0))
            .ok()
            .flatten();
    }
    let size = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "enabled": cfg.enabled,
        "path": cfg.db_path.display().to_string(),
        "rows": rows,
        "bytes": size,
        "oldest_ts": oldest,
        "newest_ts": newest,
        "retention_hours": cfg.retention_hours,
        "max_rows": cfg.max_rows,
        "flush_ms": cfg.flush_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gw-hist-{}-{}.db", tag, uuid::Uuid::new_v4()));
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", p.display(), s));
        }
        p
    }

    fn sample(ts: i64, tag: &str, v: f64) -> Sample {
        Sample {
            ts_ms: ts,
            node_id: "n1".to_string(),
            group_id: "g1".to_string(),
            tag: tag.to_string(),
            value: v,
            tenant_id: "default".to_string(),
        }
    }

    #[test]
    fn bucket_width_covers_the_range() {
        assert_eq!(bucket_ms(0, 1000, 10), 100);
        assert_eq!(bucket_ms(0, 1000, 3), 334, "向上取整，保证不超点数");
        assert_eq!(bucket_ms(0, 0, 10), 1, "空区间也要有合法宽度");
        assert_eq!(bucket_ms(0, 100, 0), 100, "max_points=0 按 1 处理");
    }

    #[test]
    fn schema_and_roundtrip() {
        let db = tmp_db("roundtrip");
        let conn = open_db(&db).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let batch: Vec<Sample> = (0..10)
            .map(|i| sample(i * 1000, "temperature", i as f64))
            .collect();
        assert_eq!(write_batch(&conn, &batch).unwrap(), 10);

        let q = SeriesQuery {
            node_id: "n1".into(),
            group_id: "g1".into(),
            tag: "temperature".into(),
            from_ms: 0,
            to_ms: 10_000,
            max_points: 5,
            tenant_id: "*".into(),
        };
        let series = query_series(&db, &q).unwrap();
        assert!(
            series.len() <= 5,
            "点数不得超过 max_points，实际 {}",
            series.len()
        );
        assert!(!series.is_empty());
        // 聚合值应覆盖原始数据范围
        let min = series.iter().map(|b| b.min).fold(f64::MAX, f64::min);
        let max = series.iter().map(|b| b.max).fold(f64::MIN, f64::max);
        assert_eq!(min, 0.0);
        assert_eq!(max, 9.0);
        let total: u64 = series.iter().map(|b| b.count).sum();
        assert_eq!(total, 10, "所有点都应落进某个桶");

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn query_ignores_other_series_and_range() {
        let db = tmp_db("filter");
        let conn = open_db(&db).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        write_batch(
            &conn,
            &[
                sample(1000, "temperature", 1.0),
                sample(2000, "temperature", 2.0),
                Sample {
                    tag: "humidity".to_string(),
                    ..sample(1500, "humidity", 9.0)
                },
            ],
        )
        .unwrap();
        drop(conn);

        let q = SeriesQuery {
            node_id: "n1".into(),
            group_id: "g1".into(),
            tag: "temperature".into(),
            from_ms: 1500,
            to_ms: 5000,
            max_points: 100,
            tenant_id: "*".into(),
        };
        let series = query_series(&db, &q).unwrap();
        let total: u64 = series.iter().map(|b| b.count).sum();
        assert_eq!(total, 1, "只应返回指定序列在时间窗内的点");
        assert_eq!(series[0].avg, 2.0);

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn retention_prunes_old_rows() {
        let db = tmp_db("retention");
        let conn = open_db(&db).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let now = now_ms();
        write_batch(
            &conn,
            &[
                sample(now - 10 * 3_600_000, "t", 1.0), // 10 小时前
                sample(now - 1000, "t", 2.0),           // 刚发生
            ],
        )
        .unwrap();
        let cfg = HistoryConfig {
            retention_hours: 1,
            ..Default::default()
        };
        let removed = prune(&conn, &cfg).unwrap();
        assert_eq!(removed, 1);
        let left: i64 = conn
            .query_row("SELECT COUNT(1) FROM samples", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn max_rows_prunes_oldest_first() {
        let db = tmp_db("maxrows");
        let conn = open_db(&db).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let now = now_ms();
        let batch: Vec<Sample> = (0..50)
            .map(|i| sample(now - (50 - i) * 1000, "t", i as f64))
            .collect();
        write_batch(&conn, &batch).unwrap();
        let cfg = HistoryConfig {
            retention_hours: 24 * 365,
            max_rows: 10,
            ..Default::default()
        };
        prune(&conn, &cfg).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(1) FROM samples", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 10, "行数上限生效");
        // 保留的应是最新的（值最大的那批）
        let min_left: f64 = conn
            .query_row("SELECT MIN(value) FROM samples", [], |r| r.get(0))
            .unwrap();
        assert_eq!(min_left, 40.0, "应从最旧的开始删");

        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn group_data_is_converted_to_samples() {
        use gateway_sdk::types::{DataValue, NodeId};
        use gateway_sdk::{GroupId, TagId};
        use std::collections::HashMap;

        let t_temp = TagId::new();
        let t_text = TagId::new();
        let t_bool = TagId::new();
        let mut names = HashMap::new();
        names.insert(t_temp, "temperature".to_string());
        names.insert(t_text, "note".to_string());
        names.insert(t_bool, "alarm".to_string());

        let data = GroupData {
            node_id: NodeId::new(),
            group_id: GroupId::new(),
            ts: chrono::Utc::now(),
            values: vec![
                (t_temp, DataValue::Float64(21.5)),
                (t_text, DataValue::String("hello".to_string())),
                (t_bool, DataValue::Bool(true)),
                (TagId::new(), DataValue::Int32(7)), // 无名字映射 → 跳过
            ],
            node_name: Some("n".to_string()),
            group_name: Some("g".to_string()),
            tag_names: Some(names),
        };
        let samples = samples_from_group_data(&data);
        assert_eq!(samples.len(), 2, "字符串点位不计入历史");
        let names: Vec<&str> = samples.iter().map(|s| s.tag.as_str()).collect();
        assert!(names.contains(&"temperature"));
        assert!(names.contains(&"alarm"), "布尔按 0/1 计入");
        assert_eq!(
            samples.iter().find(|s| s.tag == "alarm").unwrap().value,
            1.0
        );
    }

    #[test]
    fn group_data_without_names_is_skipped() {
        use gateway_sdk::types::{DataValue, NodeId};
        use gateway_sdk::{GroupId, TagId};
        let data = GroupData {
            node_id: NodeId::new(),
            group_id: GroupId::new(),
            ts: chrono::Utc::now(),
            values: vec![(TagId::new(), DataValue::Int32(1))],
            node_name: None,
            group_name: None,
            tag_names: None,
        };
        assert!(samples_from_group_data(&data).is_empty());
    }

    /// §4.2-6：v1 旧库迁移——补 tenant_id 列、删旧索引建新索引、旧行归 default 域可查
    #[test]
    fn migrate_v1_db_adds_tenant_column() {
        let db = tmp_db("migrate");
        // 手工建 v1 形状（无 tenant_id，旧索引名）
        {
            let conn = open_db(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE samples (ts INTEGER NOT NULL, node_id TEXT NOT NULL,
                   group_id TEXT NOT NULL, tag TEXT NOT NULL, value REAL NOT NULL);
                 CREATE INDEX idx_samples_series ON samples(node_id, group_id, tag, ts);
                 INSERT INTO samples VALUES (1000, 'n1', 'g1', 't', 1.0);",
            )
            .unwrap();
        }
        {
            let conn = open_db(&db).unwrap();
            ensure_history_schema(&conn).unwrap();
            // 旧行仍在且归 default 域
            let (n, tenant): (i64, String) = conn
                .query_row(
                    "SELECT COUNT(1), tenant_id FROM samples GROUP BY tenant_id",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(n, 1);
            assert_eq!(tenant, "default");
            // 新索引存在、旧索引已删
            let idx: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master WHERE type='index' AND name=?1",
                    params![SERIES_INDEX],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(idx, 1, "新索引 {SERIES_INDEX} 应存在");
            let old: i64 = conn
                .query_row(
                    "SELECT COUNT(1) FROM sqlite_master WHERE type='index' AND name=?1",
                    params![LEGACY_INDEX],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(old, 0, "旧索引应已删除");
        }
        // 迁移后可按 default 域查询；其他域查不到
        let q_all = SeriesQuery {
            node_id: "n1".into(),
            group_id: "g1".into(),
            tag: "t".into(),
            from_ms: 0,
            to_ms: 2000,
            max_points: 10,
            tenant_id: "*".into(),
        };
        assert_eq!(query_series(&db, &q_all).unwrap().len(), 1);
        let q_default = SeriesQuery {
            tenant_id: "default".into(),
            ..q_all.clone()
        };
        assert_eq!(query_series(&db, &q_default).unwrap().len(), 1);
        let q_other = SeriesQuery {
            tenant_id: "tenant-b".into(),
            ..q_all
        };
        assert!(query_series(&db, &q_other).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    /// §4.2-9（历史段）：同序列不同域的样本互不可见
    #[test]
    fn query_filters_by_tenant() {
        let db = tmp_db("tenantfilter");
        let conn = open_db(&db).unwrap();
        ensure_history_schema(&conn).unwrap();
        let mut a = sample(1000, "t", 1.0);
        a.tenant_id = "tenant-a".into();
        let mut b = sample(2000, "t", 2.0);
        b.tenant_id = "tenant-b".into();
        write_batch(&conn, &[a, b]).unwrap();
        drop(conn);

        let base = SeriesQuery {
            node_id: "n1".into(),
            group_id: "g1".into(),
            tag: "t".into(),
            from_ms: 0,
            to_ms: 3000,
            max_points: 10,
            tenant_id: "*".into(),
        };
        assert_eq!(query_series(&db, &base).unwrap().len(), 2);
        let qa = SeriesQuery {
            tenant_id: "tenant-a".into(),
            ..base.clone()
        };
        let sa = query_series(&db, &qa).unwrap();
        assert_eq!(sa.len(), 1);
        assert_eq!(sa[0].avg, 1.0);
        let qb = SeriesQuery {
            tenant_id: "tenant-b".into(),
            ..base
        };
        let sb = query_series(&db, &qb).unwrap();
        assert_eq!(sb.len(), 1);
        assert_eq!(sb[0].avg, 2.0);
        let _ = std::fs::remove_file(&db);
    }
}
