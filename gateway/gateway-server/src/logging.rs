//! 日志初始化：主程序级别、文件输出、按节点分文件。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::Event;
use tracing_subscriber::field::Visit;
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;
use tracing_subscriber::registry::LookupSpan;

/// 从 Event 中提取 node_id 与 message 的 Visitor
#[derive(Default)]
struct NodeIdVisitor {
    node_id: Option<String>,
    message: Option<String>,
}

impl Visit for NodeIdVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        if name == "node_id" {
            let s = format!("{:?}", value);
            let id = s
                .strip_prefix("NodeId(")
                .and_then(|t| t.strip_suffix(')'))
                .unwrap_or(&s);
            let id = id.trim_matches('"');
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                self.node_id = Some(id.to_string());
            } else {
                self.node_id = Some(
                    s.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_"),
                );
            }
        } else if name == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "node_id" {
            self.node_id = Some(value.to_string());
        } else if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

/// 按 node_id 将事件写入对应节点日志文件的 Layer
struct NodeFileLayer {
    dir: PathBuf,
    writers: Mutex<HashMap<String, std::fs::File>>,
}

impl NodeFileLayer {
    fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            writers: Mutex::new(HashMap::new()),
        }
    }

    fn writer_for(&self, node_id: &str) -> std::io::Result<std::fs::File> {
        let mut map = self.writers.lock().unwrap();
        if let Some(f) = map.get_mut(node_id) {
            return f.try_clone();
        }
        let path = self.dir.join(format!("{}.log", node_id));
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let out = f.try_clone()?;
        map.insert(node_id.to_string(), f);
        Ok(out)
    }
}

impl<S> Layer<S> for NodeFileLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = NodeIdVisitor::default();
        event.record(&mut visitor);
        let Some(ref node_id) = visitor.node_id else {
            return;
        };
        let level = event.metadata().level();
        let target = event.metadata().target();
        let msg = visitor
            .message
            .as_deref()
            .unwrap_or("<no message>");
        let line = format!(
            "{} {} {} {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
            level,
            target,
            msg
        );
        if let Ok(mut f) = self.writer_for(node_id) {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

/// 初始化全局日志：控制台 + 可选主日志文件 + 可选按节点分文件
pub fn init_logging(config: &crate::config::Config) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_appender::rolling::{RollingFileAppender, Rotation};

    let effective_filter = if config.log_level.trim().is_empty() {
        config.log_filter.clone()
    } else {
        format!("{},{}", config.log_level.trim(), config.log_filter)
    };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| effective_filter.as_str().into());

    let console = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_ansi(true);

    let reg = tracing_subscriber::registry()
        .with(env_filter)
        .with(console);

    let file_path = config.log_file.clone();
    let nodes_dir = config.log_dir_nodes.clone();

    macro_rules! add_file_layer {
        ($reg:expr, $path:expr) => {{
            if let Some(parent) = $path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let file_appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .build($path)
                .expect("create log file");
            let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
            std::mem::forget(guard);
            $reg.with(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_level(true)
                    .with_ansi(false)
                    .with_writer(non_blocking),
            )
        }};
    }

    macro_rules! add_node_layer {
        ($reg:expr, $dir:expr) => {
            $reg.with(NodeFileLayer::new($dir.clone()))
        };
    }

    match (file_path.as_ref(), nodes_dir.as_ref()) {
        (Some(path), Some(dir)) => add_node_layer!(add_file_layer!(reg, path), dir).init(),
        (Some(path), None) => add_file_layer!(reg, path).init(),
        (None, Some(dir)) => add_node_layer!(reg, dir).init(),
        (None, None) => reg.init(),
    }
}
