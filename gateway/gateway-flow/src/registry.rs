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
        use crate::operators::filter;
        use crate::operators::transform;
        use crate::operators::aggregate;
        use crate::operators::router;

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
    }
}

impl Default for OperatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
