//! IEC 60870-5-104 ASDU → DataValue 缓存映射。
//!
//! 映射规则（来自 docs/design/南向驱动.md §3.6）：
//! - M_ME_NA / M_ME_NB（规一化/标度化值）→ Int16
//! - M_ME_NC（短浮点）→ Float32
//! - M_SP（单点信息）→ Bool
//! - M_DP（双点信息）→ Bool（中间态记 0 并 warn）

use gateway_sdk::types::DataValue;
use iec104::types::information_elements::{Dpi, Siq, Spi};
use iec104::types::quality_descriptors::Qds;
use iec104::types::InformationObjects;

/// 检查 Siq 的 quality 标记是否为"中间态"。
fn siq_is_intermediate(siq: &Siq) -> bool {
    siq.iv || siq.nt
}

/// 检查 Diq 的 quality 标记是否为"中间态"。
fn diq_is_intermediate(diq: &iec104::types::information_elements::Diq) -> bool {
    diq.iv || diq.nt
}

/// 检查 Qds quality 标记是否为"中间态"。
fn qds_is_intermediate(q: &Qds) -> bool {
    q.iv || q.nt
}

/// 从 Siq 提取 Spi 值。
fn spi_from_siq(siq: &Siq) -> bool {
    matches!(siq.spi, Spi::On)
}

/// 从 Diq 提取 Dpi 值（返回 bool，intermediate → false + warn）。
fn dpi_to_bool(diq: &iec104::types::information_elements::Diq) -> (bool, bool) {
    match diq.dpi {
        Dpi::Off => (false, false),
        Dpi::On => (true, false),
        Dpi::Indeterminate => (false, true),
        Dpi::Invalid => (false, true),
    }
}

/// 将 InformationObjects 解析为 DataValue。
/// 返回 None 如果类型不在支持范围内。
pub fn info_objects_to_value(io: &InformationObjects) -> Option<DataValue> {
    match io {
        InformationObjects::MSpNa1(v) => {
            let obj = v.first()?;
            if siq_is_intermediate(&obj.object.siq) {
                tracing::warn!("M_SP_NA_1 intermediate state (iv/nt), setting 0");
                Some(DataValue::Bool(false))
            } else {
                Some(DataValue::Bool(spi_from_siq(&obj.object.siq)))
            }
        }
        InformationObjects::MDpNa1(v) => {
            let obj = v.first()?;
            let diq = &obj.object.diq;
            if diq_is_intermediate(diq) {
                tracing::warn!("M_DP_NA_1 intermediate state, setting 0");
                Some(DataValue::Bool(false))
            } else {
                let (val, intermediate) = dpi_to_bool(diq);
                if intermediate {
                    tracing::warn!("M_DP_NA_1 intermediate DPI state, setting 0");
                    Some(DataValue::Bool(false))
                } else {
                    Some(DataValue::Bool(val))
                }
            }
        }
        InformationObjects::MMeNa1(v) => {
            // Normalized Value → Int16
            let obj = v.first()?;
            if qds_is_intermediate(&obj.object.qds) || obj.object.qds.ov {
                tracing::warn!("M_ME_NA_1 intermediate/overflow state, setting 0");
                Some(DataValue::Int16(0))
            } else {
                Some(DataValue::Int16(obj.object.nva))
            }
        }
        InformationObjects::MMeNb1(v) => {
            // Scaled Value → Int16
            let obj = v.first()?;
            if qds_is_intermediate(&obj.object.qds) || obj.object.qds.ov {
                tracing::warn!("M_ME_NB_1 intermediate/overflow state, setting 0");
                Some(DataValue::Int16(0))
            } else {
                Some(DataValue::Int16(obj.object.sva))
            }
        }
        InformationObjects::MMeNc1(v) => {
            // Short Float → Float32
            let obj = v.first()?;
            if qds_is_intermediate(&obj.object.qds) {
                tracing::warn!("M_ME_NC_1 intermediate state, setting 0.0");
                Some(DataValue::Float32(0.0))
            } else {
                Some(DataValue::Float32(obj.object.value))
            }
        }
        InformationObjects::MStNa1(v) => {
            // Step Position → Int8
            let obj = v.first()?;
            let vti = &obj.object.vti;
            if qds_is_intermediate(&vti.qds) || vti.transient {
                tracing::warn!("M_ST_NA_1 intermediate/transient state, setting 0");
                Some(DataValue::Int8(0))
            } else {
                Some(DataValue::Int8(vti.value))
            }
        }
        _ => None,
    }
}

/// 将 ASDU 解析并写入缓存。
/// 返回 Some((IOA, DataValue, ms_timestamp))。
pub fn asdu_to_cache_entry(asdu: &iec104::asdu::Asdu) -> Option<(u32, DataValue, u64)> {
    use std::time::SystemTime;
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // IOA 位于 GenericObject 内
    let io = &asdu.information_objects;
    let first_ioa = match io {
        InformationObjects::MSpNa1(v) => v.first()?.address,
        InformationObjects::MDpNa1(v) => v.first()?.address,
        InformationObjects::MMeNa1(v) => v.first()?.address,
        InformationObjects::MMeNb1(v) => v.first()?.address,
        InformationObjects::MMeNc1(v) => v.first()?.address,
        InformationObjects::MStNa1(v) => v.first()?.address,
        _ => return None,
    };

    info_objects_to_value(io).map(|v| (first_ioa, v, ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iec104::types::information_elements::{Diq, Siq, Spi};
    use iec104::types::measurements::{MDpNa1, MMeNa1, MMeNb1, MMeNc1, MSpNa1};
    use iec104::types::quality_descriptors::Qds;
    use iec104::types::GenericObject;

    fn make_siq(iv: bool, nt: bool, spi: Spi) -> Siq {
        // Siq is { iv, nt, sb, bl, spi }
        Siq {
            iv,
            nt,
            sb: false,
            bl: false,
            spi,
        }
    }

    fn make_diq(iv: bool, nt: bool, dpi: Dpi) -> Diq {
        Diq {
            iv,
            nt,
            sb: false,
            bl: false,
            dpi,
        }
    }

    fn make_qds(iv: bool, nt: bool, ov: bool) -> Qds {
        Qds {
            iv,
            nt,
            sb: false,
            bl: false,
            ov,
        }
    }

    #[test]
    fn test_single_point_on() {
        let io = InformationObjects::MSpNa1(vec![GenericObject {
            address: 5,
            object: MSpNa1 {
                siq: make_siq(false, false, Spi::On),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(true))));
    }

    #[test]
    fn test_single_point_off() {
        let io = InformationObjects::MSpNa1(vec![GenericObject {
            address: 5,
            object: MSpNa1 {
                siq: make_siq(false, false, Spi::Off),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(false))));
    }

    #[test]
    fn test_single_point_invalid() {
        let io = InformationObjects::MSpNa1(vec![GenericObject {
            address: 5,
            object: MSpNa1 {
                siq: make_siq(true, false, Spi::Off),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(false)))); // intermediate → false
    }

    #[test]
    fn test_double_point_on() {
        let io = InformationObjects::MDpNa1(vec![GenericObject {
            address: 10,
            object: MDpNa1 {
                diq: make_diq(false, false, Dpi::On),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(true))));
    }

    #[test]
    fn test_double_point_off() {
        let io = InformationObjects::MDpNa1(vec![GenericObject {
            address: 10,
            object: MDpNa1 {
                diq: make_diq(false, false, Dpi::Off),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(false))));
    }

    #[test]
    fn test_double_point_indeterminate() {
        let io = InformationObjects::MDpNa1(vec![GenericObject {
            address: 10,
            object: MDpNa1 {
                diq: make_diq(false, false, Dpi::Indeterminate),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Bool(false)))); // intermediate → false
    }

    #[test]
    fn test_normalized_value() {
        let io = InformationObjects::MMeNa1(vec![GenericObject {
            address: 20,
            object: MMeNa1 {
                nva: 1234i16,
                qds: make_qds(false, false, false),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Int16(1234))));
    }

    #[test]
    fn test_normalized_overflow() {
        let io = InformationObjects::MMeNa1(vec![GenericObject {
            address: 20,
            object: MMeNa1 {
                nva: 1234i16,
                qds: make_qds(false, false, true),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Int16(0)))); // overflow → 0
    }

    #[test]
    fn test_scaled_value() {
        let io = InformationObjects::MMeNb1(vec![GenericObject {
            address: 30,
            object: MMeNb1 {
                sva: -500i16,
                qds: make_qds(false, false, false),
            },
        }]);
        let dv = info_objects_to_value(&io);
        assert!(matches!(dv, Some(DataValue::Int16(-500))));
    }

    #[test]
    fn test_short_float() {
        let io = InformationObjects::MMeNc1(vec![GenericObject {
            address: 40,
            object: MMeNc1 {
                value: std::f32::consts::PI,
                qds: make_qds(false, false, false),
            },
        }]);
        let dv = info_objects_to_value(&io);
        match dv {
            Some(DataValue::Float32(v)) => assert!((v - std::f32::consts::PI).abs() < 0.001),
            _ => panic!("expected Float32"),
        }
    }

    #[test]
    fn test_unsupported_type_returns_none() {
        // M_IT_NA_1 (IntegratedTotals) is not supported
        use iec104::types::measurements::MItNa1;
        use iec104::types::quality_descriptors::SeqQd;
        let io = InformationObjects::MItNa1(vec![GenericObject {
            address: 99,
            object: MItNa1 {
                bcr: 42,
                sqd: SeqQd::default(),
            },
        }]);
        assert!(info_objects_to_value(&io).is_none());
    }
}
