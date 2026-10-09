# 当前交接状态

2026-10-09 / 全面统一为 Tinge：正式原生程序为 `target/release/tinge.exe`，七个 Rust crate 使用 `tinge-*` 名称，MCP 服务为 `tinge`，49 个工具使用 `tinge_*` 前缀，图片资源为 `tinge://preview/…`。查看器标题、HTTP 请求头、EXR 软件标识和 alpha 属性、插件连接、脚本、示例与文档同步改名。项目示例统一使用 `.tinge` 和 `.tinge.assets/`。不提供旧名称别名或兼容层。

打包脚本直接构建正式 release 程序，插件包固定输出至 `target/plugin-package/tinge`，不再生成带随机编号的独立可执行版本。改名前已按用户授权结束占用旧正式程序的三个进程。

已实现七个 Rust crates、31 类 CPU 算子、DAG/蒙版、ICC、基础 RAW、非破坏性项目和历史、真实 OpenColorIO 2.5.2/ACES、CDL/LUT 交换及烘焙、无模型抠像、CLI/JSONL/MCP、后台任务、原生本地查看器与登记临时文件回收。

PR #1 已合并：统一部分失败回执、修复输入色彩解释的统计缓存、独立控制诊断/分析/内联图片、compact analyze 省略直方图、预览资源索引限制 64 项。合并前 Windows/Linux 四项 CI 均通过。架构审计见 [architecture-audit.md](architecture-audit.md)。

完整 Resolve/Lightroom 功能对等仍未完成。专业校准/HDR 显示、完整 RAW 摄影显影、更多相机验证、GPU、AI 语义蒙版与持久任务恢复等限制见 [coverage.md](coverage.md)。结构验收不等于真实模型工具选择或宿主兼容验收。

本次改名验证：Rustfmt、Clippy `-D warnings`、完整 workspace 测试、正式 release 构建、49 项 MCP 工具契约与固定插件包验收、Agent 后台任务/资源/幂等验收、查看器脚本测试及真实 HTTP 流程全部通过。schema 已重新导出，变更仅为 LUT 默认标题中的产品名称。正式程序与包内程序 SHA-256 一致。生成目录中的 105 个旧名称可执行文件已清理；未再生成独立验收程序。

报告：`artifacts/tinge-plugin-acceptance.json`、`artifacts/tinge-agent-acceptance.json`、`artifacts/viewer-acceptance.json`。改名后的真实模型工具选择回放和浏览器视觉验收未执行。
