//! Script Python operator: execute Python code via subprocess (python3 -c).
//!
//! Configuration fields:
//! - script: Python code to execute
//! - input_fields: Vec<String> - names of input variables to inject
//! - output_fields: Vec<String> - names of output variables to extract
//! - timeout_ms: u64 - execution timeout in milliseconds (default 1000)

use async_trait::async_trait;
use gateway_sdk::{
    Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, PluginError, PluginErrorCode,
    DataValue, NodeId,
};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::io::AsyncReadExt;
use tokio::time::timeout;

pub struct ScriptPythonOperator {
    script: String,
    input_fields: Vec<String>,
    output_fields: Vec<String>,
    timeout_ms: u64,
}

impl ScriptPythonOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let script = config
            .get("script")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let input_fields: Vec<String> = config
            .get("input_fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let output_fields: Vec<String> = config
            .get("output_fields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let timeout_ms = config
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        Self { script, input_fields, output_fields, timeout_ms }
    }

    fn build_python_code(&self, input: &HashMap<String, DataValue>) -> String {
        let mut lines = Vec::new();

        for field in &self.input_fields {
            if let Some(val) = input.get(field) {
                let py_val = Self::data_value_to_python(val);
                lines.push(format!("{} = {}", field, py_val));
            }
        }

        if !self.script.is_empty() {
            lines.push(self.script.clone());
        }

        if !self.output_fields.is_empty() {
            let items: Vec<String> = self.output_fields.iter()
                .map(|f| format!("'{}': {{{}}}", f, f))
                .collect();
            lines.push(format!("import json; print(json.dumps({{{}}}))", items.join(", ")));
        } else {
            lines.push("print('ok')".to_string());
        }

        lines.join("\n")
    }

    fn data_value_to_python(val: &DataValue) -> String {
        match val {
            DataValue::Bool(v) => v.to_string(),
            DataValue::Int8(v) => v.to_string(),
            DataValue::Int16(v) => v.to_string(),
            DataValue::Int32(v) => v.to_string(),
            DataValue::Int64(v) => v.to_string(),
            DataValue::UInt8(v) => v.to_string(),
            DataValue::UInt16(v) => v.to_string(),
            DataValue::UInt32(v) => v.to_string(),
            DataValue::UInt64(v) => v.to_string(),
            DataValue::Float32(v) => v.to_string(),
            DataValue::Float64(v) => v.to_string(),
            DataValue::String(v) => format!("\"{}\"", v.replace('\\', "\\\\").replace('"', ".")),
            DataValue::Bytes(v) => format!("bytes([{}])", v.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")),
        }
    }

    async fn execute_script(&self, code: String) -> PluginResult<HashMap<String, DataValue>> {
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(&code)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PluginError::msg(format!("failed to spawn python3: {}", e)))?;

        let dur = Duration::from_millis(self.timeout_ms);

        let result = timeout(dur, async {
            let mut stdout_buf = vec![];
            child.stdout.take().unwrap().read_to_end(&mut stdout_buf).await.map_err(PluginError::Io)?;
            let status = child.wait().await.map_err(PluginError::Io)?;
            Ok((status, stdout_buf)) as Result<(std::process::ExitStatus, Vec<u8>), PluginError>
        }).await;

        match result {
            Ok(Ok((status, stdout_bytes))) => {
                if status.success() {
                    let stdout = String::from_utf8_lossy(&stdout_bytes);
                    let trimmed = stdout.trim();
                    if trimmed.is_empty() || trimmed == "{}" || trimmed == "'ok'" {
                        return Ok(HashMap::new());
                    }
                    let json_str = if trimmed.starts_with('\'') && trimmed.ends_with('\'') {
                        &trimmed[1..trimmed.len()-1]
                    } else {
                        trimmed
                    };
                    if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(json_str) {
                        let mut result_map = HashMap::new();
                        for (k, v) in map {
                            result_map.insert(k, Self::json_to_data_value(v));
                        }
                        return Ok(result_map);
                    }
                    Ok(HashMap::new())
                } else {
                    Err(PluginError::WithCode {
                        code: PluginErrorCode::Unknown,
                        message: "python script failed".to_string(),
                    })
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                let _ = child.kill().await;
                Err(PluginError::WithCode {
                    code: PluginErrorCode::Timeout,
                    message: "python execution timed out".to_string(),
                })
            }
        }
    }

    fn json_to_data_value(v: serde_json::Value) -> DataValue {
        use serde_json::Value;
        match v {
            Value::Null => DataValue::String("null".to_string()),
            Value::Bool(b) => DataValue::Bool(b),
            Value::Number(ref n) => {
                if let Some(f) = n.as_f64() {
                    DataValue::Float64(f)
                } else if let Some(i) = n.as_i64() {
                    DataValue::Int64(i)
                } else if let Some(u) = n.as_u64() {
                    DataValue::UInt64(u)
                } else {
                    DataValue::String(v.to_string())
                }
            }
            Value::String(s) => DataValue::String(s),
            Value::Array(_) | Value::Object(_) => DataValue::String(v.to_string()),
        }
    }
}

#[async_trait]
impl Operable for ScriptPythonOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "script-python".into(),
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Execute Python script (subprocess mode)".into()),
            version: "0.1.0".into(),
            name_zh: Some("Python脚本".into()),
            name_en: Some("Python Script".into()),
            description_zh: Some("通过 subprocess 执行 Python 代码片段".into()),
            description_en: Some("Execute Python code snippets via subprocess".into()),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        let code = self.build_python_code(&data.payload);
        let result = self.execute_script(code).await?;

        let mut output = PipelineData::new(_node_id);
        for field in &self.output_fields {
            if let Some(val) = result.get(field) {
                output.payload.insert(field.clone(), val.clone());
            }
        }

        Ok(vec![output])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_script() {
        let mut config = PluginConfig::new();
        config.insert("script".to_string(), serde_json::json!("x = 10\ny = 20\nresult = x + y"));
        config.insert("input_fields".to_string(), serde_json::json!([]));
        config.insert("output_fields".to_string(), serde_json::json!(["result"]));

        let op = ScriptPythonOperator::new(&config);
        let input = PipelineData::new(NodeId::new());

        let result = op.process(NodeId::new(), input).await;
        assert!(result.is_ok(), "script execution failed: {:?}", result);
    }
}