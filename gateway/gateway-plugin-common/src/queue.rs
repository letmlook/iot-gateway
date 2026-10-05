//! 北向插件共享离线队列（可选磁盘持久化）。
//!
//! # 记录格式
//!
//! ```text
//! [u32 小端 topic 长度][topic 字节][u32 小端 payload 长度][payload 字节]
//! ```
//!
//! 长度上限 `MAX_RECORD_BYTES`，超过则不落盘（防止异常数据把磁盘写满）。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// 单条记录上限（topic + payload 合计）
const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
/// 已消费字节超过文件这个比例时触发压缩
const COMPACT_THRESHOLD: f64 = 0.5;

/// 待发送的一条消息。topic 字段语义放宽为「路由说明」：
/// - MQTT/Kafka：实际 topic
/// - HTTP：URL 路径（用于日志/观测）
/// - 行协议插件：空串
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub topic: String,
    pub payload: Vec<u8>,
}

/// 队列统计（对外暴露用）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QueueStats {
    /// 当前待发条数
    pub queued: usize,
    /// 因队列满而丢弃的条数（丢弃最旧的）
    pub dropped_overflow: u64,
    /// 从磁盘恢复的条数（进程启动时）
    pub recovered: u64,
    /// 因记录损坏/截断而被丢弃的条数
    pub dropped_corrupt: u64,
    /// 是否启用磁盘持久化
    pub persisted: bool,
}

/// 离线队列。
///
/// 内存里保存待发记录的顺序副本；启用持久化时同步追加到文件。
pub struct OfflineQueue {
    path: Option<PathBuf>,
    mem: VecDeque<Record>,
    max: usize,
    dropped_overflow: u64,
    recovered: u64,
    dropped_corrupt: u64,
    /// 文件中已消费（已发出）的字节数，用于压缩判断
    consumed_bytes: u64,
    /// 文件当前长度
    file_len: u64,
}

impl OfflineQueue {
    /// 打开队列。
    ///
    /// - `path = None`：纯内存（磁盘不可用时的降级路径）
    /// - `max`：最大条数；为 0 时按 1 处理
    pub fn open(path: Option<PathBuf>, max: usize) -> Self {
        let max = max.max(1);
        let mut q = Self {
            path,
            mem: VecDeque::new(),
            max,
            dropped_overflow: 0,
            recovered: 0,
            dropped_corrupt: 0,
            consumed_bytes: 0,
            file_len: 0,
        };
        if let Some(p) = q.path.clone() {
            match Self::load_file(&p, max) {
                Ok((records, valid_len, corrupt, overflow)) => {
                    q.file_len = valid_len;
                    q.recovered = records.len() as u64;
                    q.dropped_corrupt = corrupt;
                    q.dropped_overflow = overflow;
                    q.mem = records;
                    if corrupt > 0
                        || valid_len != std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)
                    {
                        if let Err(e) = std::fs::OpenOptions::new()
                            .write(true)
                            .open(&p)
                            .and_then(|f| f.set_len(valid_len))
                        {
                            tracing::warn!(path = %p.display(), "truncate corrupt tail failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        path = %p.display(),
                        "offline queue load failed, falling back to memory only: {}", e
                    );
                    q.path = None;
                }
            }
        }
        q
    }

    /// 纯内存队列
    pub fn memory_only(max: usize) -> Self {
        Self::open(None, max)
    }

    /// 读取文件中的完整记录。
    ///
    /// 返回 `(记录, 有效字节数, 损坏条数, 因超上限而丢弃的条数)`。
    /// 超过 `max` 时保留**最新**的 max 条（断网期间旧数据价值更低）。
    fn load_file(path: &Path, max: usize) -> std::io::Result<(VecDeque<Record>, u64, u64, u64)> {
        if !path.exists() {
            return Ok((VecDeque::new(), 0, 0, 0));
        }
        let mut f = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        let mut all = VecDeque::new();
        let mut off = 0usize;
        let mut valid = 0usize;
        let mut corrupt = 0u64;
        while off + 4 <= buf.len() {
            let topic_len =
                u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
            let after_topic = off + 4 + topic_len;
            if topic_len > MAX_RECORD_BYTES || after_topic + 4 > buf.len() {
                corrupt += 1;
                break;
            }
            let payload_len = u32::from_le_bytes([
                buf[after_topic],
                buf[after_topic + 1],
                buf[after_topic + 2],
                buf[after_topic + 3],
            ]) as usize;
            let end = after_topic + 4 + payload_len;
            if payload_len > MAX_RECORD_BYTES || end > buf.len() {
                corrupt += 1;
                break;
            }
            let topic = String::from_utf8_lossy(&buf[off + 4..after_topic]).to_string();
            let payload = buf[after_topic + 4..end].to_vec();
            all.push_back(Record { topic, payload });
            off = end;
            valid = off;
        }
        // 尾部残余字节（不足一条记录）视为损坏
        if valid != buf.len() {
            corrupt += 1;
        }

        let mut overflow = 0u64;
        while all.len() > max {
            all.pop_front();
            overflow += 1;
        }
        Ok((all, valid as u64, corrupt, overflow))
    }

    /// 入队。队列满时丢弃最旧的一条并计数。
    pub fn push(&mut self, record: Record) {
        while self.mem.len() >= self.max {
            self.mem.pop_front();
            self.dropped_overflow += 1;
        }
        self.append_to_file(&record);
        self.mem.push_back(record);
    }

    /// 取出最早的一条（用于补发）
    pub fn pop_front(&mut self) -> Option<Record> {
        let r = self.mem.pop_front();
        if r.is_some() {
            // 出队即认为已消费：累计消费字节，必要时压缩文件
            if let Some(rec) = &r {
                self.consumed_bytes += Self::record_len(rec) as u64;
            }
            self.maybe_compact();
        }
        r
    }

    /// 放回队首（补发失败时回滚）
    pub fn push_front(&mut self, record: Record) {
        if self.consumed_bytes > 0 {
            let len = Self::record_len(&record) as u64;
            self.consumed_bytes = self.consumed_bytes.saturating_sub(len);
        }
        self.mem.push_front(record);
    }

    pub fn len(&self) -> usize {
        self.mem.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mem.is_empty()
    }

    pub fn stats(&self) -> QueueStats {
        QueueStats {
            queued: self.mem.len(),
            dropped_overflow: self.dropped_overflow,
            recovered: self.recovered,
            dropped_corrupt: self.dropped_corrupt,
            persisted: self.path.is_some(),
        }
    }

    fn record_len(r: &Record) -> usize {
        4 + r.topic.len() + 4 + r.payload.len()
    }

    fn append_to_file(&mut self, r: &Record) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let len = Self::record_len(r);
        if len > MAX_RECORD_BYTES {
            tracing::warn!(topic = %r.topic, "offline record too large, kept in memory only");
            return;
        }
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(dir = %parent.display(), "create cache dir failed: {}", e);
                self.path = None; // 降级为纯内存
                return;
            }
        }
        let res = (|| -> std::io::Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            f.write_all(&(r.topic.len() as u32).to_le_bytes())?;
            f.write_all(r.topic.as_bytes())?;
            f.write_all(&(r.payload.len() as u32).to_le_bytes())?;
            f.write_all(&r.payload)?;
            f.sync_data()?;
            Ok(())
        })();
        match res {
            Ok(()) => self.file_len += len as u64,
            Err(e) => {
                tracing::warn!(path = %path.display(), "offline queue append failed: {}", e);
                self.path = None; // 写失败即降级，避免反复报错
            }
        }
    }

    /// 已消费比例过大时重写文件，只保留未消费记录
    fn maybe_compact(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.file_len == 0 {
            return;
        }
        if (self.consumed_bytes as f64) / (self.file_len as f64) < COMPACT_THRESHOLD {
            return;
        }
        let tmp = path.with_extension("queue.tmp");
        let res = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            for r in &self.mem {
                f.write_all(&(r.topic.len() as u32).to_le_bytes())?;
                f.write_all(r.topic.as_bytes())?;
                f.write_all(&(r.payload.len() as u32).to_le_bytes())?;
                f.write_all(&r.payload)?;
            }
            f.sync_data()?;
            drop(f);
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                self.file_len = self.mem.iter().map(Self::record_len).sum::<usize>() as u64;
                self.consumed_bytes = 0;
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "offline queue compact failed: {}", e);
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    /// 主动压缩（关闭节点时调用，保证文件与内存一致）
    pub fn compact_now(&mut self) {
        self.consumed_bytes = self.file_len;
        self.maybe_compact();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("common-q-{}-{}.queue", tag, uuid::Uuid::new_v4()));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn rec(topic: &str, payload: &str) -> Record {
        Record {
            topic: topic.to_string(),
            payload: payload.as_bytes().to_vec(),
        }
    }

    #[test]
    fn memory_only_queue_keeps_order() {
        let mut q = OfflineQueue::memory_only(10);
        q.push(rec("a", "1"));
        q.push(rec("b", "2"));
        assert_eq!(q.pop_front(), Some(rec("a", "1")));
        assert_eq!(q.pop_front(), Some(rec("b", "2")));
        assert_eq!(q.pop_front(), None);
        assert!(!q.stats().persisted);
    }

    #[test]
    fn persisted_queue_survives_restart() {
        let path = tmp_path("restart");
        {
            let mut q = OfflineQueue::open(Some(path.clone()), 10);
            q.push(rec("t/1", "payload-1"));
            q.push(rec("t/2", "payload-2"));
            assert_eq!(q.len(), 2);
        }
        // 模拟重启：重新打开同一个文件
        let mut q2 = OfflineQueue::open(Some(path.clone()), 10);
        assert_eq!(q2.len(), 2, "重启后未发送的记录必须还在");
        assert_eq!(q2.stats().recovered, 2);
        assert_eq!(q2.pop_front(), Some(rec("t/1", "payload-1")));
        assert_eq!(q2.pop_front(), Some(rec("t/2", "payload-2")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn truncated_tail_is_tolerated() {
        let path = tmp_path("truncate");
        {
            let mut q = OfflineQueue::open(Some(path.clone()), 10);
            q.push(rec("t/1", "one"));
            q.push(rec("t/2", "two"));
            q.push(rec("t/3", "three"));
        }
        // 人为追加半条记录，模拟写入过程中崩溃
        {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(&100u32.to_le_bytes()).unwrap(); // 声明 100 字节 topic，后面没有内容
            f.write_all(b"partial").unwrap();
        }
        let q = OfflineQueue::open(Some(path.clone()), 10);
        assert_eq!(q.len(), 3, "完整记录应全部保留");
        assert!(q.stats().dropped_corrupt >= 1);
        // 文件应被截断到最后一个完整边界
        let len_after = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        // 三条完整记录的字节数之和（记录 = 4 + topic + 4 + payload）
        let expect: usize = [("t/1", "one"), ("t/2", "two"), ("t/3", "three")]
            .iter()
            .map(|(topic, payload)| 4 + topic.len() + 4 + payload.len())
            .sum();
        assert_eq!(len_after, expect as u64, "损坏尾巴应被截断");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn overflow_drops_oldest_and_counts() {
        let mut q = OfflineQueue::memory_only(2);
        for i in 0..5 {
            q.push(rec("t", &i.to_string()));
        }
        assert_eq!(q.len(), 2);
        assert_eq!(q.stats().dropped_overflow, 3);
        // 保留的是最新的两条
        assert_eq!(q.pop_front(), Some(rec("t", "3")));
        assert_eq!(q.pop_front(), Some(rec("t", "4")));
    }

    #[test]
    fn reload_beyond_max_keeps_newest() {
        let path = tmp_path("maxreload");
        {
            let mut q = OfflineQueue::open(Some(path.clone()), 100);
            for i in 0..10 {
                q.push(rec("t", &i.to_string()));
            }
        }
        let mut q = OfflineQueue::open(Some(path.clone()), 3);
        assert_eq!(q.len(), 3);
        assert_eq!(
            q.stats().dropped_overflow,
            7,
            "超上限被丢弃的条数应计入 dropped_overflow"
        );
        assert_eq!(q.pop_front(), Some(rec("t", "7")));
        assert_eq!(q.pop_front(), Some(rec("t", "8")));
        assert_eq!(q.pop_front(), Some(rec("t", "9")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn compaction_keeps_unconsumed_records() {
        let path = tmp_path("compact");
        let mut q = OfflineQueue::open(Some(path.clone()), 100);
        for i in 0..8 {
            q.push(rec("t", &i.to_string()));
        }
        let len_before = std::fs::metadata(&path).unwrap().len();
        // 消费掉大部分，触发压缩
        for _ in 0..6 {
            q.pop_front();
        }
        let len_after = std::fs::metadata(&path).unwrap().len();
        assert!(
            len_after < len_before,
            "压缩后文件应变小（{} → {}）",
            len_before,
            len_after
        );
        // 未消费的两条不能丢：重新打开验证
        let mut q2 = OfflineQueue::open(Some(path.clone()), 100);
        assert_eq!(q2.len(), 2, "压缩后未消费记录不应丢失");
        assert_eq!(q2.pop_front(), Some(rec("t", "6")));
        assert_eq!(q2.pop_front(), Some(rec("t", "7")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rollback_after_failed_flush_restores_order() {
        let mut q = OfflineQueue::memory_only(10);
        q.push(rec("a", "1"));
        q.push(rec("b", "2"));
        let first = q.pop_front().unwrap();
        // 补发失败 → 放回队首
        q.push_front(first);
        assert_eq!(q.pop_front(), Some(rec("a", "1")));
        assert_eq!(q.pop_front(), Some(rec("b", "2")));
    }

    #[test]
    fn binary_payload_roundtrip() {
        let path = tmp_path("binary");
        let payload: Vec<u8> = vec![0, 1, 2, 255, 254, 0, 10, 13];
        {
            let mut q = OfflineQueue::open(Some(path.clone()), 4);
            q.push(Record {
                topic: "t/bin".to_string(),
                payload: payload.clone(),
            });
        }
        let mut q = OfflineQueue::open(Some(path.clone()), 4);
        let r = q.pop_front().expect("record");
        assert_eq!(r.topic, "t/bin");
        assert_eq!(r.payload, payload, "二进制 payload 必须原样恢复");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unreadable_dir_falls_back_to_memory() {
        // 用一个不可能创建的目录作为 cache_dir：不应 panic，也不应阻止入队
        let bad = PathBuf::from("/proc/definitely-not-writable/gw-queue");
        let mut q = OfflineQueue::open(Some(bad.join("n.queue")), 4);
        q.push(rec("t", "x"));
        assert_eq!(q.len(), 1, "磁盘不可用时应退化为内存队列");
        assert_eq!(q.pop_front(), Some(rec("t", "x")));
    }
}
