//! OPC UA 地址解析：NS!NODEID 格式。

/// 解析后的 OPC UA 地址：NS!NODEID（NS=命名空间索引，NODEID=数字或字符串）
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ParsedOpcAddress {
    pub namespace: u16,
    /// 数字节点 ID 或字符串节点 ID
    pub node_id: OpcNodeId,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum OpcNodeId {
    Numeric(u32),
    String(String),
}

/// 解析地址格式：NS!NODEID。例如 0!2258、2!Device1.Module1.Tag1
pub fn parse_address(addr: &str) -> Option<ParsedOpcAddress> {
    let addr = addr.trim();
    let mut it = addr.split('!');
    let ns_str = it.next()?;
    let node_part = it.next()?;
    if it.next().is_some() {
        return None;
    }
    let namespace: u16 = ns_str.parse().ok()?;
    let node_id = if let Ok(n) = node_part.parse::<u32>() {
        OpcNodeId::Numeric(n)
    } else {
        OpcNodeId::String(node_part.to_string())
    };
    Some(ParsedOpcAddress { namespace, node_id })
}
