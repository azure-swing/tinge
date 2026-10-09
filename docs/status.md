# 当前交接状态

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
