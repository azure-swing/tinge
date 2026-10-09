# 当前交接状态

2026-10-10 / 0.2.6：新增可选调色参考手册，主 Skill 仅在风格设计、偏色排查或用户要求调色指导时按需引用。包含基础校正/局部调色判断、晚霞案例、Tinge 算子与蒙版单位及官方学习入口；不内置第三方整本手册，不将晚霞参数变成通用预设。网页默认流程保留，引擎、工具 schema 与目录不变。

Skill 校验、Rustfmt、Clippy `-D warnings`、完整 workspace 139 项测试、release 构建、插件包/Agent 验收和前端 5 项测试通过。已安装 0.2.6，真实 Codex 宿主发现 42 工具、toolsError 为空；正式二进制、Skill 与参考手册在源码/生成包/安装缓存分别 SHA-256 一致。真实 CR3 分区调色并在网页检查，旧修订保留。未运行真实模型工具选择或手册采用行为回放，不能由这些检查推断自动调色质量。报告见 `artifacts/tinge-*-0.2.6.json`。

2026-10-10 / 0.2.5：将网页预览写入插件默认交互工作流：项目明确后启动/复用查看器，在首次调整前通过宿主打开页面并确认项目/修订；静态图片不能替代网页展示。显式无网页/批量导出和只读元数据检查可跳过；仅启用插件而没有照片不会启动网页。引擎与 headless 命令不变。

Skill 格式校验、Rustfmt、Clippy `-D warnings`、完整 workspace 139 项测试、正式 release 构建、插件/Agent 验收及前端 5 项测试通过。安装 0.2.5 后真实 Codex 宿主发现 42 工具、toolsError 为空；源码/包内/安装缓存的 Skill 与二进制分别通过 SHA-256 一致性检查。提示回放用例更新为 22 项，未执行真实模型工具选择回放，不能据结构验收宣称自动开页行为已经由模型验收。

2026-10-10 / 0.2.4：修复桌面宿主无法启动 Tinge 的插件路径问题。旧兼容布局的 command 使用未展开的 ${PLUGIN_ROOT}，宿主报 os error 3；改用标准 Agent Plugins 1.0 根 manifest/MCP 配置及包内相对 stdio 路径。包验收不再自行替换该变量，新增包内二进制 SHA-256/版本/握手检查及真实 Codex 宿主工具发现脚本。宿主验收只检查启动及目录，不代表模型工具选择或全部宿主兼容性。

Rustfmt、Clippy `-D warnings`、完整 workspace 139 项测试、正式 release 构建、MCP/Agent 与包验收、前端 5 项测试通过。使用桌面版自带 Codex app-server 复现旧包 0 工具/os error 3；安装 0.2.4 后返回 serverInfo.version=0.2.4、42 工具及空 toolsError。正式程序、生成包与安装缓存程序的 SHA-256 完全一致；真实 CR3 导入、调色、预览及全尺寸导出另行验证。报告见 `artifacts/tinge-host-before.json`、`artifacts/tinge-host-after.json` 及 `artifacts/tinge-*-acceptance-0.2.4.json`。

2026-10-10 / 0.2.3：撤销本地对查看器选区修复的回退，恢复安全的选区还原和复杂选区提示，并重新构建发布程序及插件包，确保本地交付采用修复后的代码。

2026-10-10 / 评审问题修复：查看器只将简单几何和左侧按绘制顺序组合的加减选区恢复为可编辑笔画；复杂右侧分组、羽化/旋转及超出 12 区域的选区保留原始记录，并显示不可编辑说明，避免 A − (B − C) 被错误展开。新增前端回归覆盖区域往返一致性、复杂结构拒绝恢复和持续提示。

MCP 仅协商支持 structuredContent/resource_link 的 2025-11-25、2025-06-18；旧版/未知版本提议返回 2025-11-25，客户端须核对支持情况。新增真实进程回归覆盖两个受支持版本的项目摘要/预览结果，以及旧版/未知提议的握手响应。README、协议、查看器和覆盖表同步修正。请求/配方/编辑序列化类型未变，工具目录仍为 42 项。

Rustfmt、Clippy `-D warnings`、完整 workspace 139 项测试、正式 release 构建、MCP/Agent 验收、前端 5 项测试和真实 HTTP 查看器验收通过。记录见 `artifacts/design-review-fixes-2026-10-10/`。真实模型选择、旧版 SDK 和浏览器视觉验收未执行；这次修复不代表完整 Resolve/Lightroom 功能对等。

2026-10-09 / 工具架构收窄：MCP 同步/后台共用工具，以 background:true 执行后台，移除九项 submit_*，目录 51→42。analyze 只分析源图，项目配方用 stats；lut_bake 直接接受 project/revision，完整配方保留在高级编辑/grade/validate。CLI/JSONL 原生请求不变。公共参数由原生类型派生，收窄后删除不可达定义；后台仍复用原队列、幂等与部分失败回执。支持后台的工具静态 readOnlyHint=false，因为排队会创建会话任务。

目录 269,768→159,421 字节、66,816→38,889 参考 tokens（-41.80%）；adjust 748，基础三工具 1,519。所有 42 项完整 schema、后台/key 条件校验通过；按新接口边界归一化后，公共参数 7,435 次合法与 58,090 次非法样例新旧结果一致。完整 Rust 回归、Clippy、正式 release、MCP 后台编辑幂等/源图分析/项目 LUT、Agent 及查看器验收通过。记录见 `artifacts/architecture-context-measurement.json`、`artifacts/architecture-contract-validation.json`。

2026-10-09 / 全量定义审计：检查 51 个工具及嵌套配方、31 类算子、10 类递归蒙版、RAW、OCIO、CDL、LUT 和抠像参数。合并联合分支公共 required、共享重复字符串数组，并精简嵌套兼容历史、哈希实现和算法出处说明。保持字段、默认值、各分支 additionalProperties、类型和递归引用；完整目录从 68,591 降至 66,816 参考 tokens，字节从 277,744 降至 269,768。51 项 schema 的 13,680 次合法输入及 103,821 次非法输入新旧校验结果一致。完整 Rust 回归、Clippy、正式 release、MCP/Agent 验收通过。覆盖与结论见 [工具定义审计](tool-schema-audit.md)，样例比较记录见 `artifacts/full-schema-audit-validation.json`。

2026-10-09 / 工具说明去冗余：精简只读工具反复“不修改文件”、输出覆盖规则、后台会话/排队提醒、基础工具内部实现细节及与 schema 重复的默认值/范围。保留参数单位、用途区别、提交后预览失败、取消不回滚及清理范围等影响调用的信息。51 个工具及参数/安全标注不变；对真实目录去除 description 后逐项比较完全一致。完整定义由 69,765 降至 68,591 参考 tokens，adjust 638，基础三工具 1,299、五工具 2,994。记录见 `artifacts/tool-wording-audit.json`。

2026-10-09 / 基础调色按需上下文：新增 adjust/submit_adjust，目录共 51 个工具。基础工具一次接收曝光、对比度、饱和度、自然饱和度、白平衡及明暗，使用一个事务并返回一次预览。复用 adjustment_id 更新三节点组，省略/null 保留值，保留下游节点；不完整、冲突、旁路组拒绝更新。复用原有原子提交、修订锁、部分失败回执和后台幂等机制。普通工作流仅读取修订，高级图/蒙版才读取配方并使用 edit_preview。

按 o200k_base 对紧凑 JSON 测量：adjust 684 tokens，edit_preview 6,151；同样五工具改用 adjust 从 8,601 降至 3,134（-63.56%），基础三工具 1,393。完整目录增加至 284,807 字节 / 69,765 tokens；此优化依赖客户端按需加载，服务端不控制宿主注入行为，也不代表单图账单。记录见 `artifacts/basic-context-measurement.json`。

Rustfmt、Clippy、完整 workspace、正式 release、真实 MCP 基础调色/修订冲突与 Agent 验收通过；新增回归覆盖参数保留、下游图、锁内二次修订检查、提交后取消及后台幂等。基础同步/后台 schema 经 Draft 2020-12 合法/非法样例检查。提示用例更新为 19 项；真实模型选择验收未执行。

2026-10-09 / 工具参数结构进一步精简：49 个工具合计从 331,551 降到 278,924 UTF-8 字节（-15.87%）；按 `o200k_base` 对紧凑 JSON 分词，从 83,309 降到 68,329 tokens（-17.98%）。基础流程的 init/project_info/preview/edit_preview/render 从 10,486 降到 8,601；edit_preview 从 7,703 降到 6,151。合并的是 schema 内部重复结构，工具、参数和运行行为保留；所有引用仍可在单个工具内解析。参考分词不等于实际模型 tokenizer 或宿主上下文占用。

新增参数压缩回归，覆盖递归引用、引用旁的额外约束、重叠 oneOf 分支、数组长度和默认值/描述保护，并给完整目录设置体积回归上限。Rustfmt、Clippy、完整 workspace、正式 release、MCP/Agent 及查看器验收通过。额外使用 JSON Schema Draft 2020-12 校验器对比旧/新定义，13,662 次合法输入及 103,637 次非法输入检查结果一致，覆盖全部 49 个工具；这是样例验证，不代表真实宿主工具选择验收。记录见 `artifacts/schema-compaction-validation.json`。

2026-10-09 / Agent 上下文精简：49 个工具和参数约束保留，省略 MCP number 的非标准 float/double format 注解并缩短重复说明；工具定义数组从 356,998 降至 331,551 UTF-8 字节，减少 7.13%。Skill 正文及元数据按统一换行计减少 35.46%。project_info 默认不返回历史（history_limit=0），显式分页保持可用；full 结果只在 structuredContent 返回一次，text 仅短状态。普通迭代使用 edit_preview 并按需内联图片，复用成功回执的修订，减少重复读取和轮询。

Rustfmt、Clippy、完整 workspace 回归、正式 release 与 Skill 格式检查通过；真实进程回归覆盖完整统计不重复及默认历史为空。约束/安全标注等价检查和单次编辑带图验收见 `artifacts/context-acceptance.json`。这仅量化协议/文件字节和实际调用结果，不代表 token、宿主上下文注入量或真实模型工具选择已测定。

2026-10-09 / 全面统一为 Tinge：正式原生程序为 `target/release/tinge.exe`，七个 Rust crate 使用 `tinge-*` 名称，MCP 服务为 `tinge`，49 个工具使用 `tinge_*` 前缀，图片资源为 `tinge://preview/…`。查看器标题、HTTP 请求头、EXR 软件标识和 alpha 属性、插件连接、脚本、示例与文档同步改名。项目示例统一使用 `.tinge` 和 `.tinge.assets/`。不提供旧名称别名或兼容层。

打包脚本直接构建正式 release 程序，插件包固定输出至 `target/plugin-package/tinge`，不再生成带随机编号的独立可执行版本。改名前已按用户授权结束占用旧正式程序的三个进程。

已实现七个 Rust crates、31 类 CPU 算子、DAG/蒙版、ICC、基础 RAW、非破坏性项目和历史、真实 OpenColorIO 2.5.2/ACES、CDL/LUT 交换及烘焙、无模型抠像、CLI/JSONL/MCP、后台任务、原生本地查看器与登记临时文件回收。

PR #1 已合并：统一部分失败回执、修复输入色彩解释的统计缓存、独立控制诊断/分析/内联图片、compact analyze 省略直方图、预览资源索引限制 64 项。合并前 Windows/Linux 四项 CI 均通过。架构审计见 [architecture-audit.md](architecture-audit.md)。

完整 Resolve/Lightroom 功能对等仍未完成。专业校准/HDR 显示、完整 RAW 摄影显影、更多相机验证、GPU、AI 语义蒙版与持久任务恢复等限制见 [coverage.md](coverage.md)。结构验收不等于真实模型工具选择或宿主兼容验收。

本次改名验证：Rustfmt、Clippy `-D warnings`、完整 workspace 测试、正式 release 构建、49 项 MCP 工具契约与固定插件包验收、Agent 后台任务/资源/幂等验收、查看器脚本测试及真实 HTTP 流程全部通过。schema 已重新导出，变更仅为 LUT 默认标题中的产品名称。正式程序与包内程序 SHA-256 一致。生成目录中的 105 个旧名称可执行文件已清理；未再生成独立验收程序。

报告：`artifacts/tinge-plugin-acceptance.json`、`artifacts/tinge-agent-acceptance.json`、`artifacts/viewer-acceptance.json`。改名后的真实模型工具选择回放和浏览器视觉验收未执行。
