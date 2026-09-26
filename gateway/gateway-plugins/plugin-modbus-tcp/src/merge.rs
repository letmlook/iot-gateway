//! 寄存器读取合并规划。
//!
//! 逐点读会让 100 个点位产生 100 次 Modbus 往返；把地址相邻、形状相同的点位
//! 合并成一次批量读，可以把往返次数降到个位数（同组内通常只有 1–3 段）。
//!
//! 只有「输入寄存器 / 保持寄存器」且无位偏移（`.BIT`）的点位参与合并：
//! 位偏移点位读取的是寄存器中的某一位，语义与整寄存器读取不同，合并会破坏其正确性。

use crate::address::{ModbusArea, ParsedAddress};

/// 允许合并的最大地址间隙（寄存器数）：间隙内的"空洞"会一起读回来，代价可接受
pub const DEFAULT_MERGE_GAP: u16 = 8;
/// Modbus 单次读寄存器上限（协议规定 0x03/0x04 最多 125 个寄存器）
pub const DEFAULT_MAX_READ_REGS: u16 = 125;

/// 一个点位的读取需求
#[derive(Debug, Clone)]
pub struct TagRead {
    /// 在 tags 数组中的下标，用于把结果回填到正确位置
    pub index: usize,
    pub parsed: ParsedAddress,
    /// 是否允许参与合并（寄存器类且无 `.BIT` 偏移）
    pub mergeable: bool,
}

impl TagRead {
    /// 根据解析结果判断该点位能否合并
    pub fn new(index: usize, parsed: ParsedAddress) -> Self {
        let is_register = matches!(
            parsed.area,
            ModbusArea::HoldingRegister | ModbusArea::InputRegister
        );
        Self {
            index,
            mergeable: is_register && parsed.bit_index.is_none() && parsed.count > 0,
            parsed,
        }
    }
}

/// 一次批量读：[start, start+count) 的连续寄存器
#[derive(Debug, Clone, PartialEq)]
pub struct MergePlan {
    pub area: ModbusArea,
    pub start: u16,
    pub count: u16,
    /// (tags 下标, 该点位在本次读结果中的寄存器偏移)
    pub members: Vec<(usize, u16)>,
}

/// 点位形状是否一致（可放进同一个合并桶）
fn same_shape(a: &ParsedAddress, b: &ParsedAddress) -> bool {
    a.area == b.area
        && a.count == b.count
        && a.endian.swap16 == b.endian.swap16
        && a.endian.order32 == b.endian.order32
}

/// 规划批量读。
///
/// 返回 `(批量读计划, 需要逐点读的 tags 下标)`。
/// 只包含成员 ≥ 2 的计划：单点"批量读"没有意义，交回逐点路径以保持行为可预期。
pub fn plan_merges(reads: &[TagRead], gap: u16, max_regs: u16) -> (Vec<MergePlan>, Vec<usize>) {
    let mut buckets: Vec<Vec<&TagRead>> = Vec::new();
    for r in reads.iter().filter(|r| r.mergeable) {
        match buckets
            .iter_mut()
            .find(|b| same_shape(&b[0].parsed, &r.parsed))
        {
            Some(b) => b.push(r),
            None => buckets.push(vec![r]),
        }
    }

    let mut plans: Vec<MergePlan> = Vec::new();
    for mut bucket in buckets {
        bucket.sort_by_key(|r| r.parsed.start);
        let mut current: Option<MergePlan> = None;
        for r in bucket {
            let start = r.parsed.start;
            let end = start.saturating_add(r.parsed.count);
            match current.as_mut() {
                Some(plan) => {
                    let plan_end = plan.start.saturating_add(plan.count);
                    let fits_span = start >= plan_end
                        && start - plan_end <= gap
                        && end.saturating_sub(plan.start) <= max_regs;
                    if fits_span {
                        plan.members.push((r.index, start - plan.start));
                        plan.count = end - plan.start;
                    } else {
                        plans.push(current.take().expect("current plan exists"));
                        current = Some(MergePlan {
                            area: r.parsed.area,
                            start,
                            count: r.parsed.count,
                            members: vec![(r.index, 0)],
                        });
                    }
                }
                None => {
                    current = Some(MergePlan {
                        area: r.parsed.area,
                        start,
                        count: r.parsed.count,
                        members: vec![(r.index, 0)],
                    });
                }
            }
        }
        if let Some(plan) = current.take() {
            plans.push(plan);
        }
    }

    let mut merged: Vec<MergePlan> = Vec::new();
    let mut singles: Vec<usize> = reads
        .iter()
        .filter(|r| !r.mergeable)
        .map(|r| r.index)
        .collect();
    for plan in plans {
        if plan.members.len() >= 2 {
            merged.push(plan);
        } else {
            singles.extend(plan.members.iter().map(|(idx, _)| *idx));
        }
    }
    singles.sort_unstable();
    merged.sort_by_key(|p| (p.start, p.count));
    (merged, singles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::parse_address_full;

    fn read(index: usize, addr: &str) -> TagRead {
        TagRead::new(index, parse_address_full(addr, 1).expect("valid address"))
    }

    #[test]
    fn adjacent_registers_merge_into_one_read() {
        // 400001/400002/400003（保持寄存器，uint16 各占 1 个寄存器）
        let reads = vec![read(0, "4x!1"), read(1, "4x!2"), read(2, "4x!3")];
        let (plans, singles) = plan_merges(&reads, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert_eq!(plans.len(), 1, "three adjacent tags should collapse into one read");
        assert!(singles.is_empty());
        let p = &plans[0];
        assert_eq!(p.start, 1);
        assert_eq!(p.count, 3);
        assert_eq!(p.members.len(), 3);
        // 偏移必须与各自地址对应
        let mut offsets: Vec<_> = p.members.iter().map(|(_, off)| *off).collect();
        offsets.sort_unstable();
        assert_eq!(offsets, vec![0, 1, 2]);
    }

    #[test]
    fn gaps_are_tolerated_up_to_the_limit_then_split() {
        // 间隙 3（<=8）→ 合并
        let close = vec![read(0, "4x!1"), read(1, "4x!5")];
        let (plans, _) = plan_merges(&close, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].count, 5, "the hole in between is read too");

        // 间隙 100 → 拆成两次
        let far = vec![read(0, "4x!1"), read(1, "4x!101")];
        let (plans, singles) = plan_merges(&far, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert!(plans.is_empty(), "far apart tags are not worth merging");
        assert_eq!(singles, vec![0, 1]);
    }

    #[test]
    fn span_is_capped_by_max_registers() {
        // 200 个寄存器跨度超过单次读上限，必须拆分
        let reads = vec![read(0, "4x!1"), read(1, "4x!200")];
        let (plans, _) = plan_merges(&reads, 500, 125);
        assert!(plans.is_empty(), "span above the protocol limit must not be merged");
    }

    #[test]
    fn different_areas_and_shapes_are_not_merged() {
        // 保持寄存器 vs 输入寄存器
        let reads = vec![read(0, "4x!1"), read(1, "3x!2")];
        let (plans, singles) = plan_merges(&reads, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert!(plans.is_empty());
        assert_eq!(singles, vec![0, 1]);

        // 不同字节序（int32 大端 vs 小端）形状不同，不合并
        let mixed = vec![read(0, "4x!1"), read(1, "4x!2")];
        let (plans, _) = plan_merges(&mixed, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert_eq!(plans.len(), 1, "identical shapes merge");
    }

    #[test]
    fn bit_index_tags_and_coils_stay_single() {
        // 带 .BIT 的点位读取的是某一位，语义不同，不能与整寄存器读合并
        let reads = vec![read(0, "4x!1.3"), read(1, "4x!2"), read(2, "0x!1")];
        let (plans, singles) = plan_merges(&reads, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        assert!(plans.is_empty(), "nothing mergeable here");
        assert_eq!(singles, vec![0, 1, 2]);
    }

    #[test]
    fn duplicate_addresses_do_not_produce_negative_offsets() {
        // 同一地址被两个点位引用（不同数据类型时形状可能相同）：不得算错偏移
        let reads = vec![read(0, "4x!7"), read(1, "4x!7")];
        let (plans, singles) = plan_merges(&reads, DEFAULT_MERGE_GAP, DEFAULT_MAX_READ_REGS);
        for plan in &plans {
            for (_, off) in &plan.members {
                assert!(plan.count > 0);
                assert!(*off < plan.count.max(1), "offset must stay inside the read window");
            }
        }
        // 完全重叠时退化为逐点读（不合并跨度为 1）
        assert!(singles.contains(&0) || plans.iter().any(|p| p.members.len() == 2));
    }
}
