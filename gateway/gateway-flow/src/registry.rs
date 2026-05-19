//! Operator registry for built-in and external operators.

use std::collections::HashMap;
use gateway_sdk::{Operable, PluginMeta};

/// Registry of available operators (built-in + future external).
pub struct OperatorRegistry {
    operators: HashMap<String, Box<dyn Operable>>,
}

impl OperatorRegistry {
    pub fn new() -> Self {
        Self {
            operators: HashMap::new(),
        }
    }
    
    pub fn register(&mut self, name: &str, op: impl Operable + 'static) {
        self.operators.insert(name.to_string(), Box::new(op));
    }
    
    pub fn get(&self, name: &str) -> Option<&dyn Operable> {
        self.operators.get(name).map(|b| b.as_ref())
    }
}

impl Default for OperatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
