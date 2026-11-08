//! 流程引擎错误类型。

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum FlowError {
    #[error("flow validation: {0}")]
    Validation(String),
    #[error("node {0} not found in flow")]
    NodeNotFound(Uuid),
    #[error("cycle detected in flow graph")]
    Cycle,
    #[error("operator `{0}`: {1}")]
    Operator(String, String),
    #[error("runtime: {0}")]
    Runtime(String),
}
