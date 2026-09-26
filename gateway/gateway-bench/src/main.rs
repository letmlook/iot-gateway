//! 性能基线压测（`gateway-bench`）。
//!
//! 目的：把已做的优化（总线按组分区、WAL + 增量写、一致性备份、Modbus 长连接与合并读）
//! 从「理论上更快」变成**可复现的数字**，并作为后续改造的对照基线。
//!
//! ```bash
//! cargo run --release -p gateway-bench                    # 完整规模
//! cargo run --release -p gateway-bench -- --quick         # 小规模，用于本地/CI 冒烟
//! cargo run --release -p gateway-bench -- --out docs/bench/baseline.md
//! ```
//!
//! 说明：这里的数字是**单机基线**，用于横向比较改动前后的相对变化；
//! 绝对数值随机器差异较大，跨机器比较没有意义。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gateway_core::{Bus, Node, Snapshot, SNAPSHOT_VERSION};
use gateway_sdk::{DataValue, Group, GroupId, NodeId, NodeKind, PluginConfig, Tag, TagId};

// ---------- 规模 ----------

struct Scale {
    label: &'static str,
    bus_messages: u64,
    bus_partitions: usize,
    persist_nodes: usize,
    persist_tags_per_node: usize,
    persist_iters: usize,
    merge_tags: usize,
    json_iters: usize,
    json_tags: usize,
}

impl Scale {
    fn full() -> Self {
        Self {
            label: "full",
            bus_messages: 200_000,
            bus_partitions: 200,
            persist_nodes: 100,
            persist_tags_per_node: 50,
            persist_iters: 10,
            merge_tags: 1_000,
            json_iters: 20_000,
            json_tags: 100,
        }
    }

    fn quick() -> Self {
        Self {
            label: "quick",
            bus_messages: 20_000,
            bus_partitions: 20,
            persist_nodes: 20,
            persist_tags_per_node: 20,
            persist_iters: 3,
            merge_tags: 200,
            json_iters: 2_000,
            json_tags: 100,
        }
    }

    fn bus_capacity(&self) -> usize {
        4096
    }
}

// ---------- 报告 ----------

struct Row {
    metric: String,
    value: String,
    unit: &'static str,
    note: String,
}

struct Report {
    title: String,
    rows: Vec<Row>,
    notes: Vec<String>,
}

impl Report {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn add(
        &mut self,
        metric: impl Into<String>,
        value: impl Into<String>,
        unit: &'static str,
        note: impl Into<String>,
    ) {
        self.rows.push(Row {
            metric: metric.into(),
            value: value.into(),
            unit,
            note: note.into(),
        });
    }

    fn note(&mut self, s: impl Into<String>) {
        self.notes.push(s.into());
    }

    fn markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", self.title));
        out.push_str(&format!(
            "环境：{} {}，{}，profile=release\n\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            rustc_version()
        ));
        out.push_str("| 指标 | 数值 | 单位 | 说明 |\n|---|---:|---|---|\n");
        for r in &self.rows {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                r.metric, r.value, r.unit, r.note
            ));
        }
        if !self.notes.is_empty() {
            out.push_str("\n## 解读\n\n");
            for n in &self.notes {
                out.push_str(&format!("- {}\n", n));
            }
        }
        out
    }
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 千分位整数
fn num(v: f64) -> String {
    let s = format!("{:.0}", v);
    let neg = s.starts_with('-');
    let digits = if neg { &s[1..] } else { &s[..] };
    let mut outp = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            outp.push(',');
        }
        outp.push(c);
    }
    if neg {
        format!("-{}", outp)
    } else {
        outp
    }
}

fn per_sec(count: u64, d: Duration) -> f64 {
    if d.as_secs_f64() == 0.0 {
        return 0.0;
    }
    count as f64 / d.as_secs_f64()
}

fn micros(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1e6)
}

// ---------- 构造数据 ----------

fn group_data(south: NodeId, group: GroupId, tags: usize) -> gateway_sdk::GroupData {
    gateway_sdk::GroupData {
        node_id: south,
        group_id: group,
        ts: chrono::Utc::now(),
        values: (0..tags)
            .map(|i| (TagId::new(), DataValue::Float64(i as f64)))
            .collect(),
        node_name: Some("bench-south".to_string()),
        group_name: Some("bench-group".to_string()),
        tag_names: None,
    }
}

fn snapshot(nodes: usize, tags_per_node: usize) -> Snapshot {
    let mut snap = Snapshot {
        version: SNAPSHOT_VERSION,
        nodes: Vec::with_capacity(nodes),
        groups: Vec::with_capacity(nodes),
        tags: Vec::with_capacity(nodes * tags_per_node),
        subscriptions: Vec::new(),
    };
    for n in 0..nodes {
        let node = Node::new(
            format!("bench-{}", n),
            NodeKind::South,
            "sim",
            PluginConfig::new(),
        );
        let nid = node.id();
        let g = Group::new(format!("g{}", n), 1000);
        let gid = g.id;
        snap.nodes.push(node);
        snap.groups.push((nid, g));
        for t in 0..tags_per_node {
            snap.tags.push((
                nid,
                Tag::new(format!("t{}", t), format!("4x!{}", t + 1), gid),
            ));
        }
    }
    // 若干带凭据的北向节点：让「敏感字段加密」这一项有真实负载可比
    for n in 0..10 {
        let mut cfg = PluginConfig::new();
        cfg.insert("host".into(), serde_json::json!("127.0.0.1"));
        cfg.insert("port".into(), serde_json::json!(1883));
        cfg.insert(
            "client_id".into(),
            serde_json::json!(format!("bench-{}", n)),
        );
        cfg.insert("password".into(), serde_json::json!("bench-secret-value"));
        cfg.insert("username".into(), serde_json::json!("bench-user"));
        snap.nodes.push(Node::new(
            format!("bench-north-{}", n),
            NodeKind::North,
            "mqtt",
            cfg,
        ));
    }
    snap
}

// ---------- 1. 总线：单分区投递 ----------

/// 返回 (发布耗时, 消费追平耗时, 已投递, 被丢弃)
async fn bench_bus_single(
    bus: &Bus,
    count: u64,
    cooperative: bool,
) -> (Duration, Duration, u64, u64) {
    let south = NodeId::new();
    let group = GroupId::new();
    let mut rx = bus
        .subscribe_groups(&[(south, group)])
        .pop()
        .expect("subscribed receiver");

    let delivered = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));
    let (d, s) = (delivered.clone(), skipped.clone());
    let consumer = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(_) => {
                    d.fetch_add(1, Ordering::Relaxed);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    s.fetch_add(n, Ordering::Relaxed);
                }
                Err(_) => break,
            }
        }
    });

    let payload = Arc::new(group_data(south, group, 10));
    let t = Instant::now();
    for i in 0..count {
        let _ = bus.publish(payload.clone());
        if cooperative && i % 32 == 0 {
            tokio::task::yield_now().await;
        }
    }
    let publish_elapsed = t.elapsed();

    // 等待消费者追平（上限 20s，避免慢机器上无限等待）
    let deadline = Instant::now() + Duration::from_secs(20);
    while delivered.load(Ordering::Relaxed) + skipped.load(Ordering::Relaxed) < count
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let total = t.elapsed();
    consumer.abort();

    bus.forget_group(&(south, group));
    (
        publish_elapsed,
        total,
        delivered.load(Ordering::Relaxed),
        skipped.load(Ordering::Relaxed),
    )
}

/// 多分区扇出：K 个分组各自一个订阅者，发布在分组间轮转
fn bench_bus_fanout(bus: &Bus, partitions: usize, count: u64) -> Duration {
    let keys: Vec<(NodeId, GroupId)> = (0..partitions)
        .map(|_| (NodeId::new(), GroupId::new()))
        .collect();
    let mut receivers = bus.subscribe_groups(&keys);
    // 消费者仅做「尽快取走」，避免容量积压影响发布侧
    let counters: Vec<Arc<AtomicU64>> = (0..partitions)
        .map(|_| Arc::new(AtomicU64::new(0)))
        .collect();
    let mut handles = Vec::with_capacity(partitions);
    for (i, mut rx) in receivers.drain(..).enumerate() {
        let c = counters[i].clone();
        handles.push(tokio::spawn(async move {
            while let Ok(_v) = rx.recv().await {
                c.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }

    let payloads: Vec<Arc<gateway_sdk::GroupData>> = keys
        .iter()
        .map(|(n, g)| Arc::new(group_data(*n, *g, 10)))
        .collect();

    let t = Instant::now();
    for i in 0..count {
        let p = &payloads[i as usize % partitions];
        let _ = bus.publish(p.clone());
    }
    let elapsed = t.elapsed();

    for h in handles {
        h.abort();
    }
    for k in &keys {
        bus.forget_group(k);
    }
    elapsed
}

/// 背压观测：故意用「小容量通道 + 定期休眠的消费者」，让 Lagged 必然发生，
/// 量化「慢消费者会被跳过多少条」。这不是缺陷，而是总线背压语义的实测值。
async fn bench_bus_backpressure(capacity: usize, publish: u64, settle: Duration) -> u64 {
    let bus = Bus::with_capacity(capacity);
    let south = NodeId::new();
    let group = GroupId::new();
    let mut rx = bus
        .subscribe_groups(&[(south, group)])
        .pop()
        .expect("subscribed receiver");

    let skipped = Arc::new(AtomicU64::new(0));
    let s = skipped.clone();
    let consumer = tokio::spawn(async move {
        let mut seen: u64 = 0;
        loop {
            match rx.recv().await {
                Ok(_) => {
                    seen += 1;
                    // 模拟处理耗时：每 64 条休息 1ms（≈64k msg/s）
                    if seen.is_multiple_of(64) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    s.fetch_add(n, Ordering::Relaxed);
                    seen += n;
                }
                Err(_) => break,
            }
        }
    });

    let payload = Arc::new(group_data(south, group, 10));
    for _ in 0..publish {
        let _ = bus.publish(payload.clone());
    }
    tokio::time::sleep(settle).await;
    consumer.abort();
    bus.forget_group(&(south, group));
    skipped.load(Ordering::Relaxed)
}

// ---------- 2. 持久化 ----------

async fn bench_persist(
    dir: &std::path::Path,
    snap: &Snapshot,
    iters: usize,
    secret: Option<&str>,
) -> Result<(Duration, u64), String> {
    let path = dir.join("bench.db");
    let _ = std::fs::remove_file(&path);
    // 首次写入包含建表与全量插入，不计入均值
    gateway_core::persist_save_secret(&path, snap, secret)
        .await
        .map_err(|e| e.to_string())?;
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let t = Instant::now();
        gateway_core::persist_save_secret(&path, snap, secret)
            .await
            .map_err(|e| e.to_string())?;
        total += t.elapsed();
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok((total / iters as u32, size))
}

async fn bench_persist_load(
    dir: &std::path::Path,
    secret: Option<&str>,
) -> Result<Duration, String> {
    let path = dir.join("bench.db");
    let t = Instant::now();
    let loaded = gateway_core::persist_load_secret(&path, secret)
        .await
        .map_err(|e| e.to_string())?;
    let elapsed = t.elapsed();
    if loaded.is_none() {
        return Err("bench.db 不存在".to_string());
    }
    Ok(elapsed)
}

// ---------- 3. Modbus 合并规划 ----------

/// 返回 (连续地址的请求数, 间隔 20 的请求数)
fn bench_merge_plan(tags: usize) -> ((usize, usize), (usize, usize)) {
    let contiguous: Vec<String> = (0..tags).map(|i| format!("4x!{}", i + 1)).collect();
    let strided: Vec<String> = (0..tags).map(|i| format!("4x!{}", i * 20 + 1)).collect();
    (
        plugin_modbus_tcp::merge_plan_stats(&contiguous, 1),
        plugin_modbus_tcp::merge_plan_stats(&strided, 1),
    )
}

fn bench_merge_plan_time(tags: usize, iters: usize) -> Duration {
    let addrs: Vec<String> = (0..tags).map(|i| format!("4x!{}", i + 1)).collect();
    let t = Instant::now();
    for _ in 0..iters {
        let _ = plugin_modbus_tcp::merge_plan_stats(&addrs, 1);
    }
    t.elapsed() / iters as u32
}

// ---------- 4. FFI 边界：JSON 编解码 ----------

fn bench_json_roundtrip(tags: usize, iters: usize) -> (Duration, usize) {
    let data = group_data(NodeId::new(), GroupId::new(), tags);
    let t = Instant::now();
    let mut bytes = 0;
    for _ in 0..iters {
        let s = serde_json::to_string(&data).expect("serialize");
        bytes = s.len();
        let back: gateway_sdk::GroupData = serde_json::from_str(&s).expect("deserialize");
        std::hint::black_box(&back);
    }
    (t.elapsed() / iters as u32, bytes)
}

// ---------- main ----------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let out = args
        .windows(2)
        .find(|w| w[0] == "--out")
        .map(|w| w[1].clone());

    let scale = if quick { Scale::quick() } else { Scale::full() };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let mut report = Report::new(format!(
        "性能基线（规模：{}，{}）",
        scale.label,
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));

    // 1) 总线：单分组（容量 4096）
    let cap = scale.bus_capacity();
    let bus = Bus::with_capacity(cap);
    let (pub_d, total_d, delivered, cooperative_skipped) =
        rt.block_on(bench_bus_single(&bus, scale.bus_messages, true));
    report.add(
        "总线发布（单分组）",
        num(per_sec(scale.bus_messages, pub_d)),
        "msg/s",
        format!("每 32 条让出一次（模拟轮询循环），容量 {}", cap),
    );
    report.add(
        "总线端到端（发布 + 消费追平）",
        num(per_sec(scale.bus_messages, total_d)),
        "msg/s",
        format!(
            "{} 条，已投递 {} 条、丢弃 {} 条；单条消息含 10 个点位",
            num(scale.bus_messages as f64),
            num(delivered as f64),
            num(cooperative_skipped as f64)
        ),
    );

    // 2) 总线：背压语义实测（小容量 + 慢消费者）
    let backpressure = rt.block_on(bench_bus_backpressure(
        64,
        scale.bus_messages / 4,
        Duration::from_millis(150),
    ));
    report.add(
        "总线背压丢弃（容量 64 + 慢消费者）",
        num(backpressure as f64),
        "msg",
        "故意构造：消费者每 64 条休息 1ms。>0 即 Lagged 语义生效，生产按 north_lagged_by_node 告警",
    );

    // 3) 总线：多分区扇出
    let bus3 = Bus::with_capacity(cap);
    // 该函数内部用 tokio::spawn 起消费者，必须在 runtime 上下文内调用
    let fanout_d =
        rt.block_on(async { bench_bus_fanout(&bus3, scale.bus_partitions, scale.bus_messages) });
    report.add(
        format!("总线扇出（{} 个分组各 1 个订阅者）", scale.bus_partitions),
        num(per_sec(scale.bus_messages, fanout_d)),
        "msg/s",
        "分组间轮转发布；分区后投递成本与订阅者数量无关",
    );

    // 4) 持久化
    let snap = snapshot(scale.persist_nodes, scale.persist_tags_per_node);
    let tmp = std::env::temp_dir().join(format!("gw-bench-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("create bench dir");
    let total_tags = scale.persist_nodes * scale.persist_tags_per_node;
    let rt_handle = &rt;
    let (plain, size) = rt_handle
        .block_on(bench_persist(&tmp, &snap, scale.persist_iters, None))
        .expect("plain save");
    report.add(
        format!(
            "持久化保存（{} 节点 / {} 点位，增量 UPSERT）",
            scale.persist_nodes, total_tags
        ),
        micros(plain),
        "µs/次",
        format!("库大小 {} KiB；WAL + 每次变更一次事务", size / 1024),
    );

    let (encrypted, _) = rt_handle
        .block_on(bench_persist(
            &tmp,
            &snap,
            scale.persist_iters,
            Some("bench-secret"),
        ))
        .expect("encrypted save");
    report.add(
        "持久化保存（敏感字段 AES-256-GCM 加密）",
        micros(encrypted),
        "µs/次",
        format!(
            "相对明文 +{:.0}%（仅为敏感字段加解密开销）",
            (encrypted.as_secs_f64() / plain.as_secs_f64() - 1.0) * 100.0
        ),
    );

    let load_plain = rt_handle
        .block_on(bench_persist_load(&tmp, None))
        .expect("load");
    report.add(
        "持久化加载（启动时全量）",
        format!("{:.1}", load_plain.as_secs_f64() * 1e3),
        "ms",
        format!("{} 节点 / {} 点位", scale.persist_nodes, total_tags),
    );
    let _ = std::fs::remove_dir_all(&tmp);

    // 5) Modbus 合并规划
    let (contiguous, strided) = bench_merge_plan(scale.merge_tags);
    let plan_time = bench_merge_plan_time(scale.merge_tags, 200);
    report.add(
        format!("Modbus 合并规划（{} 点位，连续地址）", scale.merge_tags),
        format!("{} 批量 + {} 逐点", contiguous.0, contiguous.1),
        "请求/周期",
        format!(
            "未合并时需 {} 次请求；规划耗时 {} µs",
            scale.merge_tags,
            micros(plan_time)
        ),
    );
    report.add(
        format!("Modbus 合并规划（{} 点位，间隔 20）", scale.merge_tags),
        format!("{} 批量 + {} 逐点", strided.0, strided.1),
        "请求/周期",
        "地址稀疏时合并收益下降，用于评估现场布线影响",
    );

    // 6) FFI 边界成本
    let (json_d, bytes) = bench_json_roundtrip(scale.json_tags, scale.json_iters);
    report.add(
        format!("FFI 边界 JSON 往返（{} 点位/帧）", scale.json_tags),
        micros(json_d),
        "µs/帧",
        format!(
            "单帧 {} B；序列化+反序列化各一次，跨 C ABI 的固有成本",
            bytes
        ),
    );

    report.note(format!(
        "总线：合作式发布 {} 条、丢弃 {} 条（每 32 条让出一次仍可能被调度抖动顶到容量上限）；\
         容量 64 + 慢消费者时丢弃 {} 条 —— 这是 Lagged 的预期语义，生产环境应通过 \
         `gateway_north_lagged_by_node` 告警，而不是期望「不丢」。",
        num(scale.bus_messages as f64),
        num(cooperative_skipped as f64),
        num(backpressure as f64)
    ));
    report.note(
        "以上为**单次运行**结果。绝对值随机器与后台负载波动明显（本机重复运行的差异可达 2 倍），\
         因此只应用于「同机同负载下改造前后对比」，并建议各跑 3 次取中位数。",
    );
    report.note(
        "持久化保存曾是最大瓶颈（5000 点位约 83 ms/次）：`save_to_db` 对每一行都重新 prepare \
         语句，删除差集还会拼出约 10 KB 的 `NOT IN (...)`。已用 `prepare_cached` + 临时键表修掉，\
         同机复测降到约 13 ms/次（见 baseline-2026-09-26-persist-fix.md）。",
    );
    let md = report.markdown();
    print!("{}", md);

    if let Some(path) = out {
        let p = std::path::PathBuf::from(&path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&p, &md).expect("write report");
        println!("\n（已写入 {}）", p.display());
    }
}
