//! XMLPath operator: extract values from XML using XPath expressions.
//!
//! This implementation uses regex-based extraction as a simpler alternative to full XPath.
//! For full XPath support, consider using sxd-xpath or xrust crates.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use std::collections::HashMap;
use regex::Regex;

pub struct XmlPathOperator {
    source_field: String,
    expressions: Vec<(String, String)>, // (output_field, xpath-like pattern)
    namespaces: HashMap<String, String>,
}

impl XmlPathOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let source_field = config.get("source_field")
            .and_then(|v| v.as_str())
            .unwrap_or("xml")
            .to_string();

        let expressions: Vec<(String, String)> = config
            .get("expressions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().filter_map(|item| {
                    let output_field = item.get("output_field")?.as_str()?.to_string();
                    let expression = item.get("expression")?.as_str()?.to_string();
                    Some((output_field, expression))
                }).collect()
            })
            .unwrap_or_default();

        let namespaces: HashMap<String, String> = config
            .get("namespaces")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter().filter_map(|(k, v)| {
                    v.as_str().map(|s| (k.clone(), s.to_string()))
                }).collect()
            })
            .unwrap_or_default();

        Self { source_field, expressions, namespaces }
    }

    fn extract_xpath_like(&self, xml: &str, pattern: &str) -> Option<String> {
        // Handle common XPath-like patterns
        // e.g., "//tag" or "/root/tag" or "//ns:tag" or "//@attr"
        
        let xml_str = xml;

        // Pattern: //tag or /root/tag - extract element content
        if pattern.starts_with('/') {
            let tag_pattern = pattern.trim_start_matches('/').trim_start_matches('/');
            // Remove any namespace prefix handling for now
            let tag_name = tag_pattern.split(':').last().unwrap_or(tag_pattern);
            
            // Build regex to match the tag and capture its content
            // This is a simplified approach - full XPath would need proper XML parsing
            let re = Regex::new(&format!(r#"<{}[^>]*>([^<]*)</{}>"#, tag_name, tag_name)).ok()?;
            if let Some(caps) = re.captures(xml_str) {
                return Some(caps.get(1)?.as_str().to_string());
            }
            
            // Try self-closing tag
            let re2 = Regex::new(&format!(r#"<{}[^>]*/>"#, tag_name)).ok()?;
            if re2.is_match(xml_str) {
                return Some(String::new());
            }
        }
        
        // Pattern: //@attr or /@attr - extract attribute value
        if pattern.contains("@attr") || pattern.starts_with("@") {
            let attr_name = pattern.trim_start_matches("@attr").trim_start_matches('@').trim_start_matches("attr");
            let re = Regex::new(&format!(r#"\{attr_name}=["']([^"']*)["']"#)).ok()?;
            if let Some(caps) = re.captures(xml_str) {
                return Some(caps.get(1)?.as_str().to_string());
            }
        }

        None
    }
}

#[async_trait]
impl Operable for XmlPathOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "xml-path",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Extract values from XML using XPath-like expressions"),
            version: "0.1.0",
            name_zh: Some("XML路径提取"),
            name_en: Some("XML Path"),
            description_zh: Some("使用XPath从XML数据中提取字段值"),
            description_en: Some("Extract values from XML data using XPath-like expressions"),
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn process(&self, _node_id: NodeId, mut data: PipelineData) -> PluginResult<Vec<PipelineData>> {
        // Get XML string from source field
        let xml_str = match data.payload.get(&self.source_field) {
            Some(DataValue::String(s)) => s.clone(),
            _ => return Ok(vec![data]), // No XML field, pass through
        };

        // For each expression, extract and add result to payload
        for (output_field, xpath_expr) in &self.expressions {
            if let Some(value) = self.extract_xpath_like(&xml_str, xpath_expr) {
                data.payload.insert(output_field.clone(), DataValue::String(value));
            }
        }

        Ok(vec![data])
    }

    async fn reset(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }
}

pub fn create_xml_path_operator(config: &PluginConfig) -> Box<dyn Operable> {
    Box::new(XmlPathOperator::new(config))
}
