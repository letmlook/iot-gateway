//! XMLPath operator: extract values from XML using XPath expressions.
//!
//! Uses sxd-document for XML parsing and sxd-xpath for XPath queries.

use async_trait::async_trait;
use gateway_sdk::{Operable, PipelineData, PluginConfig, PluginMeta, PluginResult, DataValue, NodeId};
use std::collections::HashMap;
use sxd_document::parser;
use sxd_xpath::{Context, Factory, Value};

pub struct XmlPathOperator {
    source_field: String,
    expressions: Vec<(String, String)>, // (output_field, xpath_expression)
    namespaces: HashMap<String, String>,
}

impl XmlPathOperator {
    pub fn new(config: &PluginConfig) -> Self {
        let source_field = config
            .get("source_field")
            .and_then(|v| v.as_str())
            .unwrap_or("xml")
            .to_string();

        let expressions: Vec<(String, String)> = config
            .get("expressions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let output_field = item.get("output_field")?.as_str()?.to_string();
                        let expression = item.get("expression")?.as_str()?.to_string();
                        Some((output_field, expression))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let namespaces: HashMap<String, String> = config
            .get("namespaces")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            source_field,
            expressions,
            namespaces,
        }
    }

    fn extract_values(&self, xml: &str) -> HashMap<String, serde_json::Value> {
        let mut results = HashMap::new();

        // Parse XML document using sxd-document
        let package = match parser::parse(xml) {
            Ok(pkg) => pkg,
            Err(_) => return results,
        };
        let document = package.as_document();

        // Create XPath factory and context with namespaces
        let factory = Factory::new();
        let mut context = Context::new();
        for (prefix, uri) in &self.namespaces {
            context.set_namespace(prefix, uri);
        }

        for (output_field, xpath_expr) in &self.expressions {
            // Build the XPath expression
            let xpath = match factory.build(xpath_expr) {
                Ok(Some(xp)) => xp,
                Ok(None) => continue,
                Err(_) => continue,
            };

            // Evaluate the XPath expression
            let value = match xpath.evaluate(&context, document.root()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract string value(s) from the result
            let value_str = value.string();
            if value_str.is_empty() {
                continue;
            }

            // For nodesets, we need to handle differently - each matched node
            if let Value::Nodeset(nodeset) = value {
                let mut values: Vec<String> = Vec::new();
                for node in nodeset {
                    let s = node.string_value();
                    if !s.is_empty() {
                        values.push(s);
                    }
                }

                if values.is_empty() {
                    continue;
                }

                if values.len() == 1 {
                    results.insert(output_field.clone(), serde_json::Value::String(values[0].clone()));
                } else {
                    results.insert(
                        output_field.clone(),
                        serde_json::Value::Array(
                            values.iter().map(|s| serde_json::Value::String(s.clone())).collect(),
                        ),
                    );
                }
            } else {
                // Single value (string, number, etc.)
                results.insert(output_field.clone(), serde_json::Value::String(value_str));
            }
        }

        results
    }
}

#[async_trait]
impl Operable for XmlPathOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "xml-path",
            kind: gateway_sdk::PluginKind::Operator,
            description: Some("Extract values from XML using XPath expressions"),
            version: "0.1.0",
            name_zh: Some("XML路径提取"),
            name_en: Some("XML Path"),
            description_zh: Some("使用XPath从XML数据中提取字段值"),
            description_en: Some("Extract values from XML data using XPath expressions"),
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

        // Extract values using XPath
        let extracted = self.extract_values(&xml_str);

        // Convert serde_json::Value to DataValue and insert into payload
        for (key, value) in extracted {
            let data_value = match value {
                serde_json::Value::String(s) => DataValue::String(s),
                serde_json::Value::Bool(b) => DataValue::Bool(b),
                serde_json::Value::Number(n) => {
                    if let Some(f) = n.as_f64() {
                        DataValue::Float64(f)
                    } else if let Some(i) = n.as_i64() {
                        DataValue::Int64(i)
                    } else {
                        DataValue::String(n.to_string())
                    }
                }
                serde_json::Value::Array(arr) => {
                    // For arrays, store as JSON string
                    DataValue::String(serde_json::to_string(&arr).unwrap_or_default())
                }
                _ => DataValue::String(value.to_string()),
            };
            data.payload.insert(key, data_value);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_operator(source_field: &str, expressions: Vec<(&str, &str)>) -> XmlPathOperator {
        let mut config = HashMap::new();
        config.insert(
            "source_field".to_string(),
            serde_json::json!(source_field),
        );
        config.insert(
            "expressions".to_string(),
            serde_json::json!(expressions
                .iter()
                .map(|(out, expr)| {
                    serde_json::json!({
                        "output_field": out,
                        "expression": expr
                    })
                })
                .collect::<Vec<_>>()),
        );
        XmlPathOperator::new(&config)
    }

    #[test]
    fn test_simple_element_extraction() {
        let op = create_operator(
            "xml",
            vec![("name", "//name"), ("value", "//value")],
        );

        let xml = r#"<root><name>Alice</name><value>42</value></root>"#;
        let extracted = op.extract_values(xml);

        assert_eq!(extracted.get("name").and_then(|v| v.as_str()), Some("Alice"));
        assert_eq!(extracted.get("value").and_then(|v| v.as_str()), Some("42"));
    }

    #[test]
    fn test_nested_element_extraction() {
        let op = create_operator(
            "xml",
            vec![("city", "//address/city"), ("street", "//address/street")],
        );

        let xml = r#"<root><address><city>Beijing</city><street>Main St</street></address></root>"#;
        let extracted = op.extract_values(xml);

        assert_eq!(extracted.get("city").and_then(|v| v.as_str()), Some("Beijing"));
        assert_eq!(extracted.get("street").and_then(|v| v.as_str()), Some("Main St"));
    }

    #[test]
    fn test_attribute_extraction() {
        let op = create_operator(
            "xml",
            vec![("id", "//item/@id"), ("status", "//item/@status")],
        );

        let xml = r#"<root><item id="001" status="active">Test</item></root>"#;
        let extracted = op.extract_values(xml);

        assert_eq!(extracted.get("id").and_then(|v| v.as_str()), Some("001"));
        assert_eq!(extracted.get("status").and_then(|v| v.as_str()), Some("active"));
    }

    #[test]
    fn test_multiple_matching_elements() {
        let op = create_operator("xml", vec![("items", "//item")]);

        let xml = r#"<root><item>One</item><item>Two</item><item>Three</item></root>"#;
        let extracted = op.extract_values(xml);

        let items = extracted.get("items");
        assert!(items.is_some());
        let arr = items.unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 3);
        // Note: HashSet iteration order is not guaranteed, so check contains
        assert!(arr.iter().any(|v| v.as_str() == Some("One")));
        assert!(arr.iter().any(|v| v.as_str() == Some("Two")));
        assert!(arr.iter().any(|v| v.as_str() == Some("Three")));
    }

    #[test]
    fn test_namespace_support() {
        let mut config = HashMap::new();
        config.insert("source_field".to_string(), serde_json::json!("xml"));
        config.insert(
            "expressions".to_string(),
            serde_json::json!([
                {"output_field": "ns_name", "expression": "//ns:data/ns:name"}
            ]),
        );
        config.insert(
            "namespaces".to_string(),
            serde_json::json!({"ns": "http://example.com/ns"}),
        );

        let op = XmlPathOperator::new(&config);

        // XML with properly declared namespace prefix
        let xml = r#"<root xmlns:ns="http://example.com/ns"><ns:data><ns:name>Test</ns:name></ns:data></root>"#;
        let extracted = op.extract_values(xml);

        assert_eq!(extracted.get("ns_name").and_then(|v| v.as_str()), Some("Test"));
    }

    #[test]
    fn test_text_content_extraction() {
        let op = create_operator(
            "xml",
            vec![("content", "//description/text()")],
        );

        let xml = r#"<root><description>Hello World</description></root>"#;
        let extracted = op.extract_values(xml);

        assert_eq!(
            extracted.get("content").and_then(|v| v.as_str()),
            Some("Hello World")
        );
    }

    #[test]
    fn test_invalid_xml_returns_empty() {
        let op = create_operator("xml", vec![("name", "//name")]);

        let xml = "not valid xml at all";
        let extracted = op.extract_values(xml);

        assert!(extracted.is_empty());
    }

    #[test]
    fn test_default_source_field() {
        let config = HashMap::new();
        let op = XmlPathOperator::new(&config);

        assert_eq!(op.source_field, "xml");
    }

    #[test]
    fn test_empty_expressions() {
        let op = create_operator("xml", vec![]);

        let xml = r#"<root><name>Test</name></root>"#;
        let extracted = op.extract_values(xml);

        assert!(extracted.is_empty());
    }
}