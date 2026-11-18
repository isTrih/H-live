# Tauri v2 机器绑定授权与防破解调研

核验日期：2026-10-21，Asia/Shanghai。范围：H-live 机器绑定授权、运行时云端下发模块、指定主播/直播间白名单，以及 Tauri v2 的安全边界。按项目 `docs/research` 惯例记录一手资料与代码证据；本轮只新增调研文档，没有修改业务代码、安装依赖、执行构建、查询本机硬件标识或访问用户凭据。下文区分已观察事实与架构建议，不能把建议当作已实现或已验收的功能。

## 结论

**Tauri 可以承载机器绑定授权和多层保护，但需要在应用和授权服务中实现；CSP、capabilities、Stronghold、更新签名各自解决不同问题，不能合并称作“防破解加密”。** 官方将 Rust Core 视为可信边界，明确说明 capabilities 不防范恶意 Rust 或 Rust 主动绕过。基于这个边界的架构推论是：拥有客户端修改权的人仍可能替换本地授权分支，离线程序不能承诺绝对不可破解。[Tauri 安全边界](https://v2.tauri.app/security/capabilities/#security-boundaries)

因此，多层方案应该让设备绑定、授权真实性、服务器资源访问、磁盘秘密保护和可信分发分别成立。简单把同一份本地数据重复 AES 加密，或者把同一把解密密钥拆到多个文件，不会自动产生新的信任边界。此为威胁模型推论，不是某种加密算法失效的结论。

结合新增的白名单要求，建议把产品目标定义成“指定设备只能通过我们的系统使用获准主播的数据”。若要求修改客户端也无法越过这一限制，核心采集或消息网关需由我们控制；云端随机下发代码可提高还原成本，但其最终仍在客户设备执行。第 9–10 节给出两种强度的具体设计。

## 1. Capabilities：限制前端调用，不替代授权

官方文档明确：通过 `tauri::Builder::invoke_handler` 注册的应用自定义命令，默认允许应用各窗口和 WebView 调用。不能把插件权限通常需要显式授权的规则，泛化为所有自定义命令都默认禁止。[自定义命令的默认行为](https://v2.tauri.app/security/capabilities/)

若要控制这些命令，应在构建阶段使用 `AppManifest::commands` 生成应用命令权限，再由 capability 分配。`commands` 生成对应的 `allow-*`、`deny-*` 权限，不表示自动完成用户授权或商业许可校验；需要覆盖完整命令清单、采用生成的 schema，并验证缺权调用确实被拒绝。[Tauri AppManifest API](https://docs.rs/tauri-build/2.0.0/tauri_build/struct.AppManifest.html)、[官方构建实现](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-build/src/acl.rs)

官方同时写明，capabilities 主要降低前端被攻陷后的系统接口暴露；它不防恶意 Rust、不当配置、错误 scope 检查或 Rust 主动绕过。建议在真正开始受限操作的 Rust 业务入口检查许可；窗口是否显示、按钮是否禁用、前端传来的 `activated = true` 均不应成为授权依据。后半句是基于上述边界的应用设计建议。[Capabilities Security Boundaries](https://v2.tauri.app/security/capabilities/#security-boundaries)

## 2. CSP 与 Isolation：保护 WebView / IPC

CSP 需要在 Tauri 配置中明确设置才启用。它约束页面可加载和执行的资源，Tauri 在构建时为打包资产补充相关 nonce / hash。建议按真实资源来源设置最小范围；`csp: null` 不能记为已开启 CSP 保护。[官方 CSP 文档](https://v2.tauri.app/security/csp/)

Isolation 在主前端与 Rust Core 之间插入 sandbox iframe，允许检查、拒绝或改写 IPC 输入。官方描述其消息在检查后使用每次启动生成的密钥进行 AES-GCM 加密，设计目标是阻止不可信前端内容滥用 Core。Windows 的 iframe 脚本加载还有构建内联及 ES Modules 限制，接入前须核对项目实际前端和调用负载。[官方 Isolation 机制与限制](https://v2.tauri.app/concept/inter-process-communication/isolation/)

**推论：** Isolation 的 IPC 加密不能解释成 EXE 加密、授权校验不可修改，或本地管理员无法观察运行过程。CSP 与 Isolation 可作为前端注入防护层；商业授权仍应由 Rust 业务入口和服务端共同执行。对直播高频数据是否引入可见开销，本次未做测量。

## 3. Updater 签名与 Windows Authenticode 是两层

Tauri updater 使用应用配置内的公钥验证更新包签名；官方要求签名且不提供关闭选项，私钥用于发布端签名。公钥可以公开，私钥必须留在受控发布环境。它证明更新包来自所信任的发布密钥；并不证明设备获得商业授权。[官方 Updater Signing updates](https://v2.tauri.app/plugin/updater/#signing-updates)

当前上游 updater 源码包含下载产物验签流程；这是更新包完整性检查，不是持续运行中的应用内存防篡改。此处链接是可变化的上游 `v2` 分支，实施时须再以项目锁定插件版本核对，不应直接假定所有新 API 已存在于项目。[官方 updater 实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/updater/src/updater.rs)

Windows 代码签名用于平台的发布者身份与文件信任；官方说明未签名程序仍可运行，且 EV / OV 证书都不保证新发布文件立即没有 SmartScreen 提示。故不能把 Authenticode 当作机器授权或“被修改就绝对不能启动”的保证。[Tauri Windows Code Signing](https://v2.tauri.app/distribute/sign/windows/)

建议将授权凭证签名、updater 签名、Windows 代码签名分成独立用途及密钥生命周期。最终发布物完成所需签名后，不再修改其字节；若随后加壳或改写，需要重新安排并验收签名流程。这是基于签名所保护对象的工程建议；本次仅只读检查现有本地产物的 Authenticode 状态，没有运行发行流程。

## 4. Stronghold：秘密管理，不自动绑定机器

Tauri Stronghold 插件通过 IOTA Stronghold 管理秘密与密钥，使用密码派生的 32 字节密钥保护 snapshot；提供 Argon2 配置路径。它适合评估为本地秘密存储方案，但密码、解锁方式和权限仍由应用决定。[官方 Stronghold 文档](https://v2.tauri.app/plugin/stronghold/)、[官方 snapshot 包装实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/stronghold/src/stronghold.rs)

必须区分普通 store 与 vault procedure：官方插件 `get_store_record` 会向调用方返回字节，`execute_procedure` 支持在 vault 内使用密钥执行操作。给前端 `stronghold:default` 会包含读取 store 及执行 procedure 等权限；不能因为数据放在 Stronghold，就声称它永远不会进入前端。[官方插件命令实现](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/stronghold/src/lib.rs)、[官方默认权限集](https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/stronghold/permissions/default.toml)

**推论与接入要求：** 不在前端示例中硬编码真正的 vault 密码；优先通过 Rust 暴露业务所需的最小操作，避免通用“读出任意秘密”接口。该插件没有因为安装本身就建立 TPM 设备证明、服务器激活、设备数量限制或本地代码防修改。即使密钥采用更强的硬件保护，仍需检查调用者是否能滥用已解锁的签名或解密接口。

## 5. Rust 发布优化：提高分析成本，不能作为授权机制

Cargo 的 `release` 是优化构建配置，默认 `debug = false`、`strip = "none"`、`lto = false`；`strip` 可去符号或调试信息，`lto` 是链接时优化。它们的官方定义是编译和发布设置，并未承诺加密代码或阻止反编译。[Cargo Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)

建议在项目已有发布配置上评估关闭发布调试信息、正确处理符号文件和 source map，并按性能与可诊断性选择 strip / LTO。**推论：** 优化后的原生代码通常比直接暴露前端业务逻辑更难阅读，但仍是可执行客户端；删符号、混淆、资源加密、自校验或反调试只能增加工作量，不能取代授权服务或不可伪造的授权凭证。额外保护还应做启动、直播、更新与杀软兼容性验证，本次未评估具体商业加壳器。

## 6. 供项目方案整合的优先级

以下是基于官方机制边界的建议，并非 H-live 当前实现声明：

| 顺序 | 保护层 | 应验收的结果 | 独立解决不了的问题 |
| --- | --- | --- | --- |
| 1 | 应用授权协议与服务端裁决 | 授权凭证可验证，设备和到期策略明确，服务端拒绝无权请求 | 完全离线的本地代码修改 |
| 2 | Rust 业务入口的权限检查 | 绕过界面直接调用受限操作仍被拒绝 | 攻击者替换整个本地实现 |
| 3 | 本地秘密保护 | 凭据不明文落盘，不经通用 IPC 读回前端 | 已解锁进程或合法调用接口被滥用 |
| 4 | CSP、最小 capability、可选 Isolation | 注入面与暴露接口收敛，自定义命令 ACL 实际生效 | 核心许可策略自身缺陷 |
| 5 | Updater 与 Windows 发布签名 | 更新包、安装包和发布者信任分别验收 | 设备是否付费和运行时本地篡改 |
| 6 | 编译优化与可选混淆 / 自校验 | 降低额外信息泄露，在兼容性验证后提高分析成本 | 本地绝对不可破解 |

## 7. H-live 的实际代码与产物

下列路径相对于项目根目录，行号对应本次观察时的源码；仅为只读核查，不是新增授权机制。

| 观察 | 代码证据 | 对方案的影响 |
| --- | --- | --- |
| 桌面完整核心在本地进程执行 | `src-tauri/src/backend.rs:24`、`:30`、`:50`；`crates/h-live-runtime/src/requests.rs:16` | 当前仅有退出/更新门禁，没有机器码、许可证或主播授权校验 |
| 初始化可能直接恢复监控 | `crates/h-live-runtime/src/runtime.rs:448`、`:449`、`:684` | 前端激活页或 RPC 的 start 分支不能单独承担授权；需在真实调度入口检查 |
| 新建监控和 WSS 连接是两层入口 | `crates/h-live-runtime/src/runtime.rs:512`；`crates/h-live-runtime/src/monitor.rs:173`、`:201` | 应分别检查设备/任务权限与刚解析得到的真实主播身份 |
| 代理凭据已经采用用户范围 DPAPI | `crates/h-live-runtime/src/storage.rs:74`、`:188` | 可复用封装设备私钥和授权缓存；当前并不保护许可证或全部数据库 |
| 发布已做精简与更新验签 | `src-tauri/Cargo.toml:25`；`src-tauri/tauri.conf.json:70`；`scripts/verify_release.ps1:36` | 不必为授权更换现有 updater 密钥；授权使用独立密钥体系 |
| server 是另一种采集部署形态 | `crates/h-live-server/src/lib.rs:1`；`crates/h-live-server/src/auth.rs:16`；`crates/h-live-server/src/http.rs:66` | 当前是单管理员令牌/会话体系，不是账号、设备、白名单授权服务 |
| 已有算法和本地核心公开 | `README.md:70`；`crates/h-live-runtime/src/signature.rs` | 对已经公开的同一算法做云端下发，不能重新建立算法秘密；可从原始实现重新构建 |

只读使用 Get-AuthenticodeSignature 检查以下三个现存文件，均返回 NotSigned、无签署证书：`src-tauri/target/release/DouyinLiveMonitor.exe`、`dist/DouyinLiveMonitor/DouyinLiveMonitor.exe`、`dist/installer/QingkanLive-0.3.2-windows-x64-setup.exe`。这只描述工作区现有文件，不代表所有其他渠道安装包的状态，也不否定 updater 的独立签名。

## 8. 机器码与设备授权建议

1. **设备身份优先使用设备密钥，机器指纹辅助识别。** 首次注册在 Rust 侧生成设备密钥，私钥由 Windows 用户范围 DPAPI 保护；服务端保存公钥及设备记录。SMBIOS UUID、主板序列号等可作为辅助信号，需规范化、排除空值/全零/占位值、给指纹格式加版本。微软明确 UUID 缺失时可能全零，主板类也暴露序列号；因此不应简单把任意一项当作不可伪造的秘密。[系统 UUID](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-computersystemproduct)、[主板信息](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-baseboard)
2. **哈希解决不了身份真实性。** 可按产品命名空间对组件做摘要，减少直接传输原始硬件序列号，但摘要仍可被复制和上报；不能只凭客户端提交相同机器码就认定同一台物理机。不要把 MAC、IP、主播显示名称或一个本地随机 UUID 当作唯一强绑定依据。此为身份模型建议。
3. **授权用私钥签名，客户端只留公钥。** 可用成熟实现的 Ed25519 签发许可证/短期租约，签名覆盖产品、协议版本、授权 ID、设备公钥摘要、权限范围、有效期、策略版本和 key_id。验证确定的原始消息字节或标准化封装，避免 JSON 重新序列化歧义。授权签名私钥不下发，也不与 updater 或代理加密密钥复用。[EdDSA 签名与验证规范](https://www.rfc-editor.org/rfc/rfc8032)
4. **设备证明与重放保护单独实现。** 激活/续租使用服务端挑战，绑定设备密钥和请求上下文，并对挑战有效期及重复使用作服务器检查。设备数量、解绑冷却、并发租约在持久化事务里裁决。DPoP 提供了访问凭据绑定客户端密钥及 nonce 的标准参考；不应把自定义协议自动称为已符合 DPoP。[RFC 9449](https://www.rfc-editor.org/rfc/rfc9449)
5. **DPAPI 是磁盘凭据保护层。** 通常与同一用户/计算机相关，存在漫游等例外；同用户进程仍可能调用解密接口。DPAPI 不是硬件远程证明，也不保护已解密的执行逻辑。项目已有用户范围实现，首版可复用，无需仅为叠加层数再引入 Stronghold。[CryptProtectData](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)
6. **TPM 可作为后续增强。** CNG Platform Crypto Provider 可使用 TPM 保护的设备密钥；若要让服务端确认确为 TPM 密钥，需要评估密钥证明，不能信任客户端一个 tpm=true。它主要提高密钥复制成本，不能证明整个客户端未修改，也不能阻止合法设备被用作请求代理。设备密钥算法应按硬件支持另行选择，不假设 TPM 支持授权票据使用的 Ed25519。[微软 TPM Key Attestation](https://learn.microsoft.com/en-us/windows-server/identity/ad-ds/manage/component-updates/tpm-key-attestation)

需明确“一个设备”是否跨 Windows 用户计数，并提供重装、主板更换、TPM 清除和正常换机的恢复流程。组件变化与私钥丢失不能简单等同盗用；指纹最小化收集，日志不记录原始硬件标识。以上均为待实现策略。

## 9. 随机摘取代码并运行时云端下发

**技术上可以动态交付模块，但不能任意剪几行 Rust 就直接在已编译程序里执行。** 片段常依赖类型、状态、泛型、生命周期和其他函数；普通 Tauri Rust 宿主没有自动解释源码的机制。应先定义稳定模块边界，再决定交付字节码、原生库还是远程结果。

| 方式 | 适用性 | 对防破解的实际作用 |
| --- | --- | --- |
| 核心始终在自有服务器执行，只返回结果/允许的数据 | 对必须保密且必须控制访问范围的能力最有价值；要承担服务成本与可用性 | 客户端拿不到这部分实现，服务端可独立拒绝越权访问；不能阻止别人另写一个独立客户端 |
| Rust 宿主加载签名 Wasm 模块 | 适合明确输入输出、依赖较少的纯计算或规则模块，需验证额外运行时体积与性能 | 安装包可不带完整模块，便于按设备/版本交付；被接收的模块仍能被保留和分析 |
| 验签后加载原生 DLL | 可行，但架构、ABI、依赖、路径、更新与回滚成本较高 | 与普通本地原生代码相同，加载后仍处于用户掌控的进程中 |

Wasmtime 官方支持把内存中的 Wasm 字节编译成模块；其沙箱主要限制模块访问宿主和系统，不能让模块对宿主拥有者保密。宿主导入、内存和执行预算须显式收敛；Wasm 格式验证也不等于发布者签名。优先接收签名的标准 Wasm 字节并验证清单，预编译产物反序列化有额外信任要求，不能把任意下载字节交给 unsafe deserialize。[Wasmtime 安全模型](https://docs.wasmtime.dev/security.html)、[Module API](https://docs.wasmtime.dev/api/wasmtime/struct.Module.html)

DLL 应限定完整可信路径和依赖搜索范围，验证发布者/模块签名并处理检查与实际加载之间的文件替换风险。加载成功本身不是授权或可信来源证明。[Windows DLL 安全](https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-security)

若保留“每次随机”，建议限定为经过测试的等价模块变体、每次不同的会话密钥/密文、模块编号与签名清单；输入输出契约及业务结果保持确定。随机拆块顺序、加密 nonce 或变体不能冒充新的授权边界。一旦合法设备完整接收过足够的代码和解密材料，仍可能保存并重组；声称“只在内存所以抓不到”没有依据。更大规模的动态拆分还会放大兼容、重试、回滚和定位故障的成本。

本项目可考虑的模块边界：`crates/qingkan-core/src/protocol.rs:504` 的 decode_frame 是字节到 ACK/events 的本地解码接口，可评估完整协议插件；`crates/qingkan-core/src/quality.rs:100` 的 normalize 与后续判定核可在脱离 SQLite 状态后评估规则插件。逐帧请求云端会把网络延迟带入 `monitor.rs:228` 的解码/ACK 链路；把聊天内容送到云分析还会改变数据流向和服务成本。网络连接、SQLite 事务、任务取消/恢复不宜随意拆成随机片段。`h-live-runtime/src/signature.rs` 的平台请求签名虽易单独调用，但已存在公开实现，不适合作为独占授权能力。

建议的交付清单至少签名覆盖：产品、模块 ID、内容哈希、ABI/宿主兼容范围、设备公钥摘要、授权范围、策略版本、租约及到期时间。TLS、设备证明、模块来源验签、拒绝旧策略与短期租约分别处理不同问题。客户端本地到期与白名单检查仍可能被修改；云端下发本身不能消除这一点。

## 10. 新增核心约束：只能监控指定白名单主播/直播间

### 标识与输入

当前 `crates/h-live-runtime/src/storage.rs:37` 接收数字 web_rid、live.douyin.com 的数字直播间路径以及 douyin.com/follow/live 的数字路径；未实现 v.douyin.com 短链展开。不能把尚不支持的短链/任意主页链接写成已支持功能。

`crates/h-live-runtime/src/net.rs:97` 定义 RoomInfo；`:138` 从上游 owner/user 字段解析 anchor_id；`:174` 解析当前 room_id。`crates/h-live-runtime/src/monitor.rs:242` 将 room_id 保存为 session_id。**主播级白名单应以平台返回的主播身份为主键，并维护允许的链接别名；不要用可改昵称或某一场的 room_id 代替主播身份。** 当前字段的跨场稳定性仍需用合成契约及有界实测验证，不能把项目解析结果宣称为平台保证永久不变的 ID。

应区分两种可配置策略：允许同一主播未来所有场次，或仅允许明确的一次场次。全局白名单与账号授权集合取交集；客户端只能选择被授权主播，不能通过添加任意 URL 扩大集合。名称只作展示，未知身份、别名解析不一致或账号与房间对应不上时拒绝进入采集。

### 本地执行的具体门禁

1. `Runtime::start`（`crates/h-live-runtime/src/runtime.rs:512`）检查设备授权、允许的 web_rid 和任务数，覆盖普通启动与更新恢复。入口可以避免明显越权请求；持久化 config 中的 anchor_id 只能作提示，不能作授权证据。
2. `monitor::run` 每轮在 `network.room` 得到新 RoomInfo 后、`patch_info` 和场次登记前核验解析到的 anchor_id；关键位置为 `crates/h-live-runtime/src/monitor.rs:173`。只有匹配才允许登记、统计和 `:201` 的 WSS connect，每次断线重连重新核验。
3. `refresh_one`（同文件 `:112`）独立解析房间并写人数，也需使用同一策略；不能只有首次 WSS 连接受限制。尚未确认身份的离线房间只能执行策略允许的最小身份/开播查询，不能默认为授权采集。
4. 为白名单拒绝定义明确的授权错误并结束任务；当前 Invalid 通用错误会进入重试，不能对被拒主播无限重试。长连接不重新解析身份，必须用统一租约监督和策略变更通知取消被撤销任务，完成已经接受的归档。
5. 更新恢复（`runtime.rs:449`、`:684`）、cancel_update、配置导入、缓存、RPC 和后台任务都走相同策略。停止、退出、保存已收数据和必要的数据恢复不应依赖激活成功。

### 云端授权与更强控制的边界

按设备申请某个主播/场次的短期许可时，**服务器自己解析并核对身份和白名单**，不直接相信请求里上报的 anchor_id。签名许可可绑定 license_id、设备公钥摘要、anchor_id、当前 room_id、功能、policy_version、有效期和 lease_id；主播换场后需要按主播规则重新签发场次许可。授权服务由我们控制，用户自托管的现有 h-live-server 不应作为我们许可证的信任根。

但项目的 WSS connect 直接连接平台，不经过我们的服务器；平台不会因我们本地增加一张自签许可就替我们执行白名单。**本地采集 + 云端发票/发代码，只能约束遵循协议的官方客户端；修改客户端仍可能移除检查。** 这是由现有调用链得出的架构结论，不是签名算法可伪造。

若目标是“即使改了客户端，也不能借用我们的系统采集其他主播”，应采用自有服务器/受控采集网关：客户端仅提交授权主播的任务，服务器执行白名单检查、自己采集或代理消息流，并只向该账号/设备发送获准的数据。所有 HTTP、WSS、导出和结果查询均验证数据归属；禁止任意上游地址或客户端自报身份。现有 server 的采集核心可以作为改造基础，但仍需新增账号/设备权限、流式交付、隔离、容量与运营能力，不能把当前单管理员服务直接当成完成方案。

网关必须理解并控制上游主播/场次及消息交付；普通 CONNECT 隧道只转发加密流量，不能可靠执行主播白名单。server 自身的任务恢复也要复用相同策略：`crates/h-live-server/src/control.rs:243` 恢复 desired tasks，`:255` 调 runtime start，不能通过服务重启恢复已撤销目标。

随机下发计算模块可以是附加层；白名单的最终裁决不应交给可修改的下发模块。即便采用受控网关，也只能控制我们提供的服务，不能阻止别人使用已有公开代码或自写程序独立访问平台。

### 撤销和离线行为

严格白名单首版建议新任务/新场次必须联网获取许可，授权服务不可达时不放行新的目标。已连接任务可按短期租约继续，失效后停止新采集并完成写入。任何离线宽限都意味着白名单撤销的相应延迟；要明确最大延迟，不能承诺即时撤销。客户端时间回拨、缓存回滚和虚拟机快照也不能仅靠本地时钟彻底解决。

## 11. 建议的实施顺序和验收

先确定控制强度：若接受提高官方客户端的滥用/破解成本，先做独立授权服务、设备密钥、主播白名单和 Rust 全链路门禁；若要求修改客户端也无法越过我们系统的白名单，先设计受控采集网关。两条路径都不应把随机摘代码作为第一步。

随后补齐签名票据与模块清单、策略撤销、换机恢复、DPAPI 缓存和 Windows 发布签名。动态 Wasm 仅在确有模块独立交付需求时再做有界原型，并量测包体、冷启动、运行延迟和故障恢复；本次未选择依赖版本或实现原型。

后续至少验收：复制授权文件到另一设备、篡改签名/设备/主播字段、重放和过期许可、修改配置与直接 RPC 启动非白名单主播、白名单链接解析为不同身份、跨场次、重连、更新恢复、人数刷新、运行中撤销、服务断网、时钟/缓存回滚，以及写库中的安全停止。网关方案还需验收越权读取任务/消息/导出、绕过 HTTP 从 WSS 订阅，以及多设备并发租约事务。

本轮仅完成代码审阅、官方资料核查、现存文件签名状态检查与方案记录；没有运行上述新增机制的测试，也没有改变任何监控行为或现有更新密钥。
