# IoT 设备数据采集网关调研报告

> 目标：可扩展通讯协议的数据采集网关  
> 调研时间：2025-01

---

## 一、市场与产品概览

### 1.1 网关角色与能力

IoT 网关在体系中通常承担：

- **连接**：异构网络、多种通讯协议接入
- **转换**：协议 → 统一格式（常见为 MQTT/HTTP/OPC UA）
- **边缘处理**：过滤、聚合、计算、缓存
- **上行**：与云平台、SCADA、MES、时序库等对接

现代网关多支持 **插件/驱动** 扩展协议，而非写死在主程序里。

### 1.2 常见可扩展方案对比（简要）

| 类型 | 代表产品 | 协议扩展方式 | 北向输出 | 适用场景 |
|------|----------|--------------|----------|----------|
| 工业协议网关 | **EMQX Neuron** | 南/北向插件 + SDK（C） | MQTT、HTTP、Sparkplug 等 | 工业 PLC、仪表、OT 设备 |
| 平台配套网关 | **ThingsBoard Gateway** | Connector + Converter | 仅 MQTT → ThingsBoard | 与 TB 平台深度绑定 |
| 边缘框架 | **EdgeX Foundry** | Device Service（Go 等） | REST、MQTT 等 | 通用边缘、微服务架构 |
| 数据采集框架 | **Fledge** | South/North 插件（Python/C++） | 可配置多种输出 | 传感器、楼宇、能源 |
| 消息网关 | **EMQX 5.0 网关** | 监听器 + 协议适配 | MQTT | 多物联网协议统一接入 MQTT |

---

## 二、重点产品与可扩展架构

### 2.1 EMQX Neuron（推荐重点参考）

- **定位**：工业物联网连接网关，轻量、边缘优先。
- **开源**：GitHub `emqx/neuron`，LGPL。
- **协议**：100+ 工业协议（Modbus、OPC UA、S7、BACnet、EtherNet/IP、三菱、欧姆龙等），北向 MQTT、HTTP、Sparkplug 等。

**可扩展架构要点：**

- **Core + 插件**：Core 提供 NNG 消息总线、数据管理、适配器；协议与北向均为**插件**。
- **南向驱动 / 北向应用**：
  - **南向**：实现具体工业协议，访问设备。
  - **北向**：连接云、MQTT Broker、HTTP 等。
- **节点 (Node)**：每个「适配器 + 插件」形成一个 Node；多 Node 通过总线交换数据。
- **Tag / Group**：点位（Tag）与分组（Group），对应采集与轮询间隔。
- **扩展方式**：用 **C SDK** 开发动态库（.so），实现驱动/应用插件，可热插拔。

**资源与性能（参考）：**

- 内存：启动 < 7MB，运行 < 200MB。
- 采集周期：可达 ≤ 100ms。
- 支持多设备、多驱动并发；单实例可管理万级 Tag。

**适合你「可扩展协议」需求的点：**

- 插件化清晰，南/北向分离。
- 有官方 SDK 与文档，便于自研协议驱动。
- 北向以 MQTT 为主，易与自建平台或现有系统集成。

---

### 2.2 ThingsBoard IoT Gateway

- **定位**：与 ThingsBoard 平台配套的软网关（Python 3.7+）。
- **协议**：MQTT、OPC-UA、Modbus、BLE、HTTP、CAN、BACnet、ODBC、REST、SNMP、FTP、Socket、XMPP、OCPP 等，且支持**自定义 Connector**。

**可扩展架构要点：**

- **Connector**：对接具体协议或设备，负责轮询/订阅、上下行。
- **Converter**：将协议数据 ↔ ThingsBoard 遥测/属性格式；分上行、下行。
- **Event Store**：内存队列或持久化队列，断网缓冲。
- **Gateway Service**：拉拢 Connector、存储、TB 客户端，上报统计。

与平台通信**仅通过 MQTT**，遵循 ThingsBoard Gateway MQTT API。

**扩展方式**：实现自定义 Connector + Converter，按模板接入；本质上仍是「连接器 + 转换器」模式。

**局限**：强依赖 ThingsBoard；若你的目标是**独立网关 + 自建后端**，需要接受「网关→TB」或自行改造成「网关→你的 MQTT/HTTP」。

---

### 2.3 EdgeX Foundry

- **定位**：Linux 基金会下的边缘计算框架，微服务架构。
- **协议**：通过 **Device Service** 扩展；已有很多 Modbus、OPC UA、BACnet 等实现。

**可扩展架构要点：**

- **Device Service**：独立微服务，实现 `ProtocolDriver`（及可选的 `ExtendedProtocolDriver`）。
- **核心接口**：`Initialize`、`HandleReadCommands` / `HandleWriteCommands`、`Discover`、`ValidateDevice`、设备增删改等。
- **数据模型**：Device、DeviceProfile、Event、Reading；REST 暴露，内部消息总线通信。
- **扩展方式**：一般为 **Go** 实现一 DS，可 CGo 调用 C 库；独立进程，便于水平扩展。

**适合场景**：要**微服务、标准化 API、与云/后端解耦**时；协议扩展以「一个协议一个 Service」为主。

---

### 2.4 Fledge

- **定位**：面向工业 IoT、楼宇、传感器等的数据采集与边缘处理。
- **协议**：通过 **South 插件** 接入，如 Modbus、OPC UA、DHT11、S7 等；**North 插件** 负责上报。

**可扩展架构要点：**

- **South 插件**：  
  - **轮询**：`plugin_poll()` 按间隔采集，返回 asset、timestamp、readings 等。  
  - **异步 I/O**：`plugin_start()` 接入事件循环，适合阻塞/长连协议。
- **North 插件**：配置化输出（如 MQTT、HTTP、云平台）。
- **实现语言**：Python 或 C/C++；插件放在固定目录，按规范实现接口。

**适合场景**：传感器、楼宇、能源等采集；偏**数据管道 + 插件化采集**，与 Neuron 的「工业协议中枢」略有不同。

---

### 2.5 EMQX 5.0 网关框架

- **定位**：多物联网协议**统一接入 MQTT**，偏消息网关而非传统「采集网关」。
- **能力**：TCP/SSL/UDP/DTLS 监听 → 报文解析 → 统一为 MQTT 消息；MQTT、LwM2M、CoAP、JT/T 808 等。
- **扩展**：通过**协议接入模块**扩展；生态和文档以 EMQX 为主。

**适合场景**：设备**本来就用或能转换成 MQTT**；你要的是「多协议 → MQTT」的接入层，可关注。若你要做的是「轮询 PLC/仪表、聚合并上报」的采集网关，则 Neuron / EdgeX / Fledge 更贴切。

---

## 三、协议扩展的常见模式（做自研网关可复用）

| 模式 | 含义 | 典型代表 |
|------|------|----------|
| **南/北向插件** | 南向协议驱动 + 北向上报/应用，解耦 | Neuron、Fledge |
| **Connector + Converter** | 连接器管通讯，转换器管格式 | ThingsBoard Gateway |
| **Device Service** | 每个协议一个微服务，统一 API | EdgeX |
| **SDK + 动态库** | 核心提供 SDK，协议以 .so 等加载 | Neuron |
| **配置化 + 通用轮询** | 用配置描述寄存器/点位，通用引擎轮询 | 很多 Modbus/OPC UA 网关 |

做「可扩展通讯协议」时，建议至少明确：

1. **插件边界**：单一协议一个插件，还是多协议共用一个插件。
2. **数据模型**：Tag/Group、Asset/Reading、Device/Event 等，选一种统一的内部模型。
3. **扩展接口**：轮询 vs 异步、发现、读写、生命周期（增删改设备）。
4. **北向**：MQTT / HTTP / 消息队列 / 直写时序库 等，要不要可插拔。

---

## 四、选型与自研建议

### 4.1 直接采用现有网关

- **以工业设备、PLC、仪表为主** → **Neuron**：协议多、插件化成熟、可 C 自研驱动，北向 MQTT 友好。
- **已用 ThingsBoard** → **ThingsBoard Gateway**：用现成 Connector，或做自定义 Connector。
- **想要微服务、标准化 REST、多组件协作** → **EdgeX**：用现有 Device Service 或自研 DS。
- **偏传感器、楼宇、能源，喜 Python** → **Fledge**：South/North 插件简单清晰。

### 4.2 自研「可扩展协议」网关时

可借鉴的共性点：

1. **明确南向抽象**：轮询间隔、点位/寄存器映射、读写接口、可选发现。
2. **统一内部数据模型**：如「设备 + 点位 + 时间戳 + 值」，再转成 MQTT/HTTP 等。
3. **插件/SDK 契约**：加载方式（动态库 / 独立进程）、配置注入、日志与遥测。
4. **北向可插拔**：MQTT、HTTP、Kafka 等可选，便于对接不同后端。
5. **缓存与离线**：断网时本地队列/持久化，避免丢数。

若你已有技术栈偏好（如 Go/Rust/Python），可以再按语言筛选（例如 EdgeX→Go，Fledge→Python，Neuron→C）。

---

## 五、参考链接

- [EMQX Neuron 产品页](https://www.emqx.com/zh/products/emqx-neuron)
- [Neuron 文档（含架构、插件列表、SDK）](https://neugates.io/docs/en/latest/)（或 `docs.emqx.com/zh/neuronex`）
- [Neuron 插件列表](https://docs.emqx.com/zh/neuronex/latest/introduction/plugin-list/plugin-list.html)
- [ThingsBoard IoT Gateway 说明](https://thingsboard.io/docs/iot-gateway/)
- [ThingsBoard 自定义串口 Connector](https://thingsboard.io/docs/iot-gateway/custom/serial-connector/)
- [EdgeX Foundry 官方](https://www.edgexfoundry.org/)
- [Fledge 文档（South 插件等）](https://fledge-iot.readthedocs.io/)
- [EMQX 5.0 网关框架](https://www.emqx.com/zh/blog/emqx-5-0-gateway-framework)

---

## 六、总结

- **工业协议、PLC、仪表采集，且要强扩展**：优先看 **Neuron**（插件 + SDK + MQTT）。
- **协议扩展模式**：南/北向插件、Connector+Converter、Device Service 三种都可参考；自研时建议**南向抽象 + 统一数据模型 + 北向可插拔**。
- 你当前项目 `iot-gateway` 仍为空，后续可考虑：  
  - 用 Neuron 做采集与协议扩展，自建 MQTT/HTTP 后端；或  
  - 以 Neuron/EdgeX 的架构为范本，用 Go/Rust/Python 做一个最小可运行网关，再逐步加协议插件。

如需，我可以基于本报告再整理一版「自研网关最小架构设计」或「Neuron 插件开发速查表」提纲，方便你直接落地到 `iot-gateway` 项目里。
