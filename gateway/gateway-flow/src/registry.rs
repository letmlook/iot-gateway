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
        // Built-in operators will be registered here once implemented in Stage 3.
        // Format: self.register("filter", filter_meta(), || Box::new(filter::FilterOperator::new()));
        // Placeholder for now — Stage 3 will fill this in.
    }
}

impl Default for OperatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
