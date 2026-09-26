//! 联能振动采集器协议：与 dataacq-plugin-virb (YE6235D/YE6235D2) 一致。
//! UDP 通信：上位机发控制命令，采集器主动推送波形数据 (0x40)。


/// 动作类型：上位机->采集器
const STOP_GRAB: u8 = 0x00;
const START_GRAB: u8 = 0x01;
const SET_SAMPLAE_INTERVAL: u8 = 0x02;
const SET_CHANNEL_PROPERTY: u8 = 0x0a;
/// 采集器->上位机：波形数据上传
pub const UPLOAD_DATA: u16 = 0x40;

const CARD_ID: u16 = 1;

/// 采样率 id -> 实际 Hz (与 virb.json 一致)
pub fn sample_rate_id_to_hz(id: u8) -> u32 {
    match id {
        0 => 256_000,
        1 => 128_000,
        2 => 51_200,
        3 => 25_600,
        4 => 12_800,
        5 => 6_400,
        6 => 3_200,
        7 => 1_600,
        _ => 25_600,
    }
}

/// 型号：YE6275D=0 (8 通道 2 字节)，YE6275D2=1 (32 通道 4 字节)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VirbModel {
    YE6275D = 0,
    YE6275D2 = 1,
}

impl VirbModel {
    pub fn channels(&self) -> usize {
        match self {
            VirbModel::YE6275D => 8,
            VirbModel::YE6275D2 => 32,
        }
    }
    pub fn channel_byte_count(&self) -> usize {
        match self {
            VirbModel::YE6275D => 2,
            VirbModel::YE6275D2 => 4,
        }
    }
    /// 原始值转物理量系数 (与 C get_virb_factor_by_model 一致)
    pub fn factor(&self) -> f64 {
        match self {
            VirbModel::YE6275D => 1.0 / 3.2768,
            VirbModel::YE6275D2 => 10240.0 / 256.0 / 8388608.0,
        }
    }
}

/// 字节序 (与 defines.h virb_endianess_e 一致)
#[derive(Clone, Copy)]
pub enum VirbEndianess {
    ABCD = 1,
    BADC = 2,
    DCBA = 3,
    CDAB = 4,
    AB = 5,
    BA = 6,
}

impl VirbEndianess {
    pub fn from_id(id: i64) -> Self {
        match id {
            2 => VirbEndianess::BADC,
            3 => VirbEndianess::DCBA,
            4 => VirbEndianess::CDAB,
            5 => VirbEndianess::AB,
            6 => VirbEndianess::BA,
            _ => VirbEndianess::ABCD,
        }
    }
}

/// 解析 2 字节 (AB 或 BA)
fn parse_i16(data: &[u8], e: VirbEndianess) -> i32 {
    if data.len() < 2 {
        return 0;
    }
    
    match e {
        VirbEndianess::AB => i16::from_be_bytes([data[0], data[1]]) as i32,
        VirbEndianess::BA => i16::from_le_bytes([data[0], data[1]]) as i32,
        _ => i16::from_be_bytes([data[0], data[1]]) as i32,
    }
}

/// 解析 4 字节 (ABCD/BADC/DCBA/CDAB)
fn parse_i32(data: &[u8], e: VirbEndianess) -> i32 {
    if data.len() < 4 {
        return 0;
    }
    match e {
        VirbEndianess::ABCD => i32::from_be_bytes([data[0], data[1], data[2], data[3]]),
        VirbEndianess::BADC => i32::from_be_bytes([data[1], data[0], data[3], data[2]]),
        VirbEndianess::DCBA => i32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        VirbEndianess::CDAB => i32::from_be_bytes([data[2], data[3], data[0], data[1]]),
        _ => i32::from_be_bytes([data[0], data[1], data[2], data[3]]),
    }
}

/// 编码命令：10 字节 start/stop/channel_prop，14 字节 set_sample_rate
fn append_crc(buf: &mut [u8], crc_len: usize) {
    if buf.len() < crc_len + 2 {
        return;
    }
    let mut crc: u16 = 0;
    for i in 0..crc_len {
        crc = crc.wrapping_add(buf[i] as u16);
    }
    buf[crc_len] = (crc & 0xff) as u8;
    buf[crc_len + 1] = (crc >> 8) as u8;
}

/// 构建「开始采集」命令 (10 字节)
pub fn encode_start_grab() -> [u8; 10] {
    let mut buf = [0u8; 10];
    buf[0] = START_GRAB;
    buf[1] = 0x00;
    buf[2] = 2;
    buf[3] = 0;
    buf[4] = 0;
    buf[5] = 0;
    buf[6] = (CARD_ID & 0xff) as u8;
    buf[7] = (CARD_ID >> 8) as u8;
    append_crc(&mut buf, 8);
    buf
}

/// 构建「停止采集」命令 (10 字节)
pub fn encode_stop_grab() -> [u8; 10] {
    let mut buf = [0u8; 10];
    buf[0] = STOP_GRAB;
    buf[1] = 0x00;
    buf[2] = 2;
    buf[3] = 0;
    buf[4] = 0;
    buf[5] = 0;
    buf[6] = (CARD_ID & 0xff) as u8;
    buf[7] = (CARD_ID >> 8) as u8;
    append_crc(&mut buf, 8);
    buf
}

/// 构建「设置采样率」命令 (14 字节)，rate 为实际 Hz
pub fn encode_set_sample_rate(rate_hz: u32) -> [u8; 14] {
    let mut buf = [0u8; 14];
    buf[0] = SET_SAMPLAE_INTERVAL;
    buf[1] = 0x00;
    buf[2] = 6;
    buf[3] = 0;
    buf[4] = 0;
    buf[5] = 0;
    buf[6] = (CARD_ID & 0xff) as u8;
    buf[7] = (CARD_ID >> 8) as u8;
    buf[8] = (rate_hz & 0xff) as u8;
    buf[9] = ((rate_hz >> 8) & 0xff) as u8;
    buf[10] = ((rate_hz >> 16) & 0xff) as u8;
    buf[11] = ((rate_hz >> 24) & 0xff) as u8;
    append_crc(&mut buf, 12);
    buf
}

/// 构建「设置通道属性」(YE6275D2，10 字节)，prop: 1=IEPE, 0=VOLT
pub fn encode_set_channel_prop(prop: u8) -> [u8; 10] {
    let mut buf = [0u8; 10];
    buf[0] = SET_CHANNEL_PROPERTY;
    buf[1] = 0x00;
    buf[2] = 2;
    buf[3] = 0;
    buf[4] = 0;
    buf[5] = 0;
    if prop > 0 {
        buf[6] = 0xff;
        buf[7] = 0xff;
    } else {
        buf[6] = 0x00;
        buf[7] = 0x00;
    }
    buf[8] = 0x00;
    buf[9] = 0x00;
    buf
}

/// 解码后的单包数据 (与 virb_packet_t 对应)
pub struct VirbPacket {
    #[allow(dead_code)]
    pub timestamp_ms: i64,
    #[allow(dead_code)]
    pub seconds: u8,
    #[allow(dead_code)]
    pub packet_id: u16,
    pub data_maxs: Vec<f64>,
    #[allow(dead_code)]
    pub data_min: Vec<f64>,
    #[allow(dead_code)]
    pub data_avg: Vec<f64>,
}

/// 解码 UDP 波形包 (type=0x40)。与 decode_data_common 一致。
pub fn decode_upload_data(
    data: &[u8],
    timestamp_ms: i64,
    channel_count: usize,
    channel_byte_count: usize,
    _sample_rate: u32,
    endianess: VirbEndianess,
    factors: &[f64],
) -> Option<VirbPacket> {
    if data.len() < 11 {
        return None;
    }
    let type_ = u16::from_le_bytes([data[0], data[1]]);
    if type_ != UPLOAD_DATA {
        return None;
    }
    let _length = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
    let _magic = u16::from_le_bytes([data[6], data[7]]);
    let seconds = data[8];
    let packet_id = u16::from_le_bytes([data[9], data[10]]);
    let payload = data.get(11..)?;
    let all_channel_bytes = channel_byte_count * channel_count;
    let data_count_per_packet = 1024 / all_channel_bytes;
    if payload.len() < data_count_per_packet * all_channel_bytes {
        return None;
    }
    let mut data_maxs = vec![0.0; channel_count];
    let mut data_min = vec![0.0; channel_count];
    let mut data_avg = vec![0.0; channel_count];

    for ch in 0..channel_count {
        let ch_factor = factors.get(ch).copied().unwrap_or(1.0);
        let mut local_max = 0.0;
        let mut local_min = 0.0;
        let mut local_sum = 0.0f64;
        let ch_offset = ch * channel_byte_count;

        for data_index in 0..data_count_per_packet {
            let off = ch_offset + data_index * all_channel_bytes;
            let raw = payload.get(off..off + channel_byte_count)?;
            let value = if channel_byte_count == 2 {
                parse_i16(raw, endianess) as f64
            } else {
                parse_i32(raw, endianess) as f64
            };
            let real_value = value * ch_factor;
            local_sum += real_value;
            if data_index == 0 {
                local_max = real_value;
                local_min = real_value;
            } else {
                local_max = local_max.max(real_value);
                local_min = local_min.min(real_value);
            }
        }
        data_maxs[ch] = local_max;
        data_min[ch] = local_min;
        data_avg[ch] = local_sum / data_count_per_packet as f64;
    }

    Some(VirbPacket {
        timestamp_ms,
        seconds,
        packet_id,
        data_maxs,
        data_min,
        data_avg,
    })
}

/// 根据组间隔与包总数计算本周期应处理的包数 (与 virb_handle_packet_count 一致)
pub fn handle_packet_count(packet_total: usize, interval_ms: u64) -> usize {
    (interval_ms as usize * packet_total) / 1000
}
