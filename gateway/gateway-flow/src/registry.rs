//! Operator registry for built-in and external operators.

use std::collections::HashMap;
use gateway_sdk::{Operable, PluginMeta};

/// Registration entry: operator name -> boxed operable + metadata.
pub struct OperatorEntry {
    pub meta: PluginMeta,
    pub create: Box<dyn Fn() -> Box<dyn Operable> + Send + Sync>,
}

/// Registry of available operators (built-in + future external).
pub struct OperatorRegistry {
    entries: HashMap<String, OperatorEntry>,
}

impl OperatorRegistry {
    pub fn new() -> Self {
        let mut registry = Self { entries: HashMap::new() };
        registry.register_builtin_operators();
        registry
    }
    
    /// Register an operator by name with a factory function.
    pub fn register(
        &mut self,
        name: &str,
        meta: PluginMeta,
        factory: impl Fn() -> Box<dyn Operable> + Send + Sync + 'static,
    ) {
        self.entries.insert(name.to_string(), OperatorEntry {
            meta,
            create: Box::new(factory),
        });
    }
    
    /// Get operator metadata by name.
    pub fn meta(&self, name: &str) -> Option<&PluginMeta> {
        self.entries.get(name).map(|e| &e.meta)
    }
    
    /// List all registered operator names.
    pub fn list(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }
    
    /// Create a new instance of an operator by name.
    pub fn create(&self, name: &str) -> Option<Box<dyn Operable>> {
        self.entries.get(name).map(|e| (e.create)())
    }
    
    /// Returns true if operator is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }
    
    fn register_builtin_operators(&mut self) {
        use crate::operators::alarm;
        use crate::operators::filter;
        use crate::operators::transform;
        use crate::operators::aggregate;
        use crate::operators::router;
        use crate::operators::buffer;
        use crate::operators::json_path;
        use crate::operators::deadband;
        use crate::operators::formula;
        use crate::operators::clamp;
        use crate::operators::round;
        use crate::operators::change;
        use crate::operators::range;

        self.register(
            "filter",
            gateway_sdk::PluginMeta {
                name: "filter",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Pass through data only when condition is true"),
                version: "0.1.0",
                name_zh: Some("过滤器"),
                name_en: Some("Filter"),
                description_zh: Some("根据条件表达式过滤数据"),
                description_en: Some("Pass through data only when condition evaluates to true"),
            },
            || Box::new(filter::FilterOperator::new("true")),
        );

        self.register(
            "transform",
            gateway_sdk::PluginMeta {
                name: "transform",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Transform fields using rhai expressions"),
                version: "0.1.0",
                name_zh: Some("转换器"),
                name_en: Some("Transform"),
                description_zh: Some("使用 rhai 表达式对字段进行转换"),
                description_en: Some("Transform fields using rhai expressions"),
            },
            || Box::new(transform::TransformOperator::new("[]")),
        );

        self.register(
            "aggregate",
            gateway_sdk::PluginMeta {
                name: "aggregate",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Aggregate data over time windows"),
                version: "0.1.0",
                name_zh: Some("聚合器"),
                name_en: Some("Aggregate"),
                description_zh: Some("按时间窗口聚合数据，支持 Sum/Avg/Min/Max/Count"),
                description_en: Some("Aggregate data over time windows with Sum/Avg/Min/Max/Count"),
            },
            || Box::new(aggregate::AggregateOperator::new(60, "[]")),
        );

        self.register(
            "router",
            gateway_sdk::PluginMeta {
                name: "router",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Route data to different outputs based on conditions"),
                version: "0.1.0",
                name_zh: Some("路由器"),
                name_en: Some("Router"),
                description_zh: Some("根据条件将数据路由到不同的输出端口"),
                description_en: Some("Route data to different output ports based on conditions"),
            },
            || Box::new(router::RouterOperator::new("[]", "default")),
        );

        self.register(
            "buffer",
            gateway_sdk::PluginMeta {
                name: "buffer",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Buffer data and emit in batches"),
                version: "0.1.0",
                name_zh: Some("缓冲器"),
                name_en: Some("Buffer"),
                description_zh: Some("缓冲数据并批量输出"),
                description_en: Some("Buffer data and emit in batches"),
            },
            || Box::new(buffer::BufferOperator::new(100, 5)),
        );

        self.register(
            "alarm",
            gateway_sdk::PluginMeta {
                name: "alarm",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Alarm rule engine with threshold/rate-of-change/state-change rules"),
                version: "0.1.0",
                name_zh: Some("告警"),
                name_en: Some("Alarm"),
                description_zh: Some("告警规则引擎，支持阈值告警、变化率告警、状态变化告警"),
                description_en: Some("Alarm rule engine with threshold, rate-of-change, and state-change rules"),
            },
            || Box::new(alarm::AlarmOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "json-path",
            gateway_sdk::PluginMeta {
                name: "json-path",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Extract values from JSON using JSONPath"),
                version: "0.1.0",
                name_zh: Some("JSON路径提取"),
                name_en: Some("JSON Path"),
                description_zh: Some("使用JSONPath从JSON数据中提取字段值"),
                description_en: Some("Extract values from JSON data using JSONPath expressions"),
            },
            || Box::new(json_path::JsonPathOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "deadband",
            gateway_sdk::PluginMeta {
                name: "deadband",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Filter data based on deadband threshold"),
                version: "0.1.0",
                name_zh: Some("死区过滤"),
                name_en: Some("Deadband"),
                description_zh: Some("当数据变化量超过死区阈值时才会传递数据"),
                description_en: Some("Only pass data when value change exceeds deadband threshold"),
            },
            || Box::new(deadband::DeadbandOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "formula",
            gateway_sdk::PluginMeta {
                name: "formula",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Evaluate mathematical expressions on data fields"),
                version: "0.1.0",
                name_zh: Some("公式解析"),
                name_en: Some("Formula"),
                description_zh: Some("对数据字段进行数学公式求值"),
                description_en: Some("Evaluate mathematical expressions on data fields using formula syntax"),
            },
            || Box::new(formula::FormulaOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "clamp",
            gateway_sdk::PluginMeta {
                name: "clamp",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Clamp values to a specified range"),
                version: "0.1.0",
                name_zh: Some("值域限幅"),
                name_en: Some("Clamp"),
                description_zh: Some("将数值限制在指定范围内"),
                description_en: Some("Clamp numeric values to a minimum and maximum range"),
            },
            || Box::new(clamp::ClampOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "round",
            gateway_sdk::PluginMeta {
                name: "round",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Round numeric values to specified precision"),
                version: "0.1.0",
                name_zh: Some("数值取整"),
                name_en: Some("Round"),
                description_zh: Some("将数值四舍五入到指定精度"),
                description_en: Some("Round numeric values to specified decimal precision"),
            },
            || Box::new(round::RoundOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "change",
            gateway_sdk::PluginMeta {
                name: "change",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Replace, rename, or delete fields"),
                version: "0.1.0",
                name_zh: Some("字段变更"),
                name_en: Some("Change"),
                description_zh: Some("对字段进行替换/重命名/删除操作"),
                description_en: Some("Replace, rename, or delete fields in data payload"),
            },
            || Box::new(change::ChangeOperator::new(&gateway_sdk::PluginConfig::new())),
        );

        self.register(
            "range",
            gateway_sdk::PluginMeta {
                name: "range",
                kind: gateway_sdk::PluginKind::Operator,
                description: Some("Map values from one range to another (linear transformation)"),
                version: "0.1.0",
                name_zh: Some("线性变换"),
                name_en: Some("Range"),
                description_zh: Some("将数值从一个范围线性映射到另一个范围"),
                description_en: Some("Linear transformation: map a value from an input range to an output range"),
            },
            || Box::new(range::RangeOperator::new(&gateway_sdk::PluginConfig::new())),
        );
    }
}

impl Default for OperatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
