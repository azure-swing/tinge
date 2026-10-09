# Tinge 插件规范改造

依据 2026-10-09 阅读的 OpenAI 官方文档：

- [Plugin guidelines](https://developers.openai.com/plugins/plugin-guidelines)：操作独立暴露、准确描述、三项显式布尔安全标注、最小化输入与可预测行为。
- [Define tools](https://developers.openai.com/plugins/plan/tools)：按用户目标设计工具，区分查询和产生副作用的操作。
- [Build skills](https://developers.openai.com/plugins/build/skills)：触发条件、输入、步骤、输出、不能推断的事实和按需参考资料。
- [Optimize Metadata](https://developers.openai.com/plugins/guides/optimize-metadata)：直接、间接和不应触发的 golden prompts，记录实际宿主选择结果。
- [Package your plugin](https://developers.openai.com/plugins/build/plugins)：本地兼容 manifest、skills 与 MCP 配置。

## 工具契约

MCP 不再枚举或接受 tinge_agent/tinge_run/job_submit/batch。
每个业务操作由 tinge_<操作> 独立公开，同步/后台共用契约，通过 background:true 选择后台。
Rust Request 枚举仍是引擎与 CLI/JSONL 的共同实现；工具契约从对应单项生成，
去掉 command，保留严格字段检查和所有传递类型定义。
新增请求类型必须登记安全元数据或明确保持 CLI-only，否则目录完整性测试失败。
recipe/edit/mask 中的 type 表示图像数据结构，不是绕过独立工具的通用执行器。

公开 schema 不再递归携带全部 Request，后台也不接受 request/command/jobs。
_response/_inline_image 是静态声明的 MCP 输出偏好，不属于底层业务请求。
所有工具都可独立完成其描述的操作，不依赖另一个插件/连接器。

只读检查/计算标为 readOnlyHint:true、destructiveHint:false。
保存预览、导出、编辑、排队、服务控制和配置变更标为非只读。
可覆盖导出、替换预览缓存、修改当前配方、取消、回收与删除缓存标为 destructiveHint:true。
init、追加 selection_save、新增 tag 和启动 viewer_open 为新增操作；计算任务的排队为新增会话状态，destructiveHint:false。
当前服务未强制约束文件根目录，参数可引用任意本地路径，故这些操作保守标为
openWorldHint:true；仅内置目录、schema 和会话内任务/缓存查询等标为 false。
这不表示服务上传文件或联网，亦不能把标注当作文件系统授权或确认机制。

服务保留原有版本检查、路径保护、显式 overwrite、原子保存与回收时文件 hash 检查。
finalize 要求 Agent 在用户选定版本并授权清理后使用，不会因为 render 而隐式触发。
服务没有独立验证聊天授权的能力；本地宿主的工具许可和 Skill 工作流承担交互授权边界。
MCP 版本协商仍为现有 STDIO 子集，没有声称实现官方 Tasks 或 SDK 全兼容。

## 本地包与迁移

插件标识为 `tinge`，显示名为 **Tinge**；本地 marketplace 标识为 `tinge-local`，
MCP 连接名及 Skill 依赖同步为 `tinge`。原生二进制统一为 `tinge.exe`，
工具名统一为 `tinge_*`，项目示例统一为 `.tinge`；不提供旧名称兼容入口。

运行 `pwsh -File scripts/package-plugin.ps1` 直接构建 `target/release/tinge.exe`，
并更新固定包目录 `target/plugin-package/tinge`。构建前须结束占用正式程序的进程；
脚本不会自动终止进程或另建可执行版本。已有最新构建可加 -SkipBuild，
非默认二进制路径可用 -Executable 指定。包内包括 tinge/ 插件、原生 exe、许可证、
工作流 Skill/参考资料和 .agents/plugins/marketplace.json。
manifest 采用根目录 plugin.json / mcp.json 的 Agent Plugins 1.0 标准布局。
stdio 使用 type:stdio 和包内相对路径 ./bin/tinge.exe，由宿主解析成实际安装路径；
不在旧版 .mcp.json 的 command 中使用未展开的 ${PLUGIN_ROOT}，不绑定开发机器路径，
打包脚本只移除固定生成目录内的旧配置文件，不自动修改宿主个人配置。
plugin/ 是构建模板，二进制与参考资料由脚本装入最终包；安装应使用生成的包。

在项目目录执行 `codex plugin marketplace add ./target/plugin-package`，再执行
`codex plugin add tinge@tinge-local` 安装到 Codex。已安装的开发包可用相同 add 命令刷新，
无需单独配置 MCP。此前只验证 installed/enabled 状态未能发现启动路径错误；0.2.4 增加实际宿主工具发现验收，
并验证安装缓存程序与正式 release 的 SHA-256 一致；这不代表其他宿主已验收。

界面图标由 manifest 的 logo/logoDark/composerIcon 引用 plugin/assets 下的 SVG。
彩色图标使用连续 T 形和统一渐变；小图标保留同一轮廓，去除底板、收紧留白并使用纯紫色。
tinge.png 是彩色 SVG 的 512px 预览，修改轮廓时同步更新小图标和预览。

通用 STDIO 客户端仍可直接配置 exe + args:["mcp"]。
已有 MCP 调用需要把 `name:tinge_agent, arguments:{command:render,...}` 改为
`name:tinge_render, arguments:{...}`。
CLI job_submit(request=edit_preview) 在 MCP 中使用 tinge_edit_preview，参数摊平并设
background:true，idempotency_key 保留；submit_* 工具已移除。完整结果设置 _response:full；内联图设置 _inline_image:true。
preview 分析另需 include_analysis:true。
CLI run/serve/batch/schema 均保留原格式和行为。

没有上传、提交、发布或远程托管。当前包是本地开发版本，仍有功能/平台限制；
不能宣称已通过目录审核或满足公开发布的完整条件。
公开远程发布的 HTTPS、域名验证、身份、隐私/支持 URL、审查资料和宿主兼容验收
属于后续发布路线，不在本次本地工具改造中虚构或自动实施。
查看器为普通 loopback 网页，不宣称已实现 ChatGPT 内嵌 MCP UI 资源协议。

## 验收

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python scripts/agent-acceptance.py --executable target/release/tinge.exe
python scripts/plugin-acceptance.py --executable target/release/tinge.exe --package '<生成根目录>/tinge'
# 安装新版后，直接通过桌面版自带 Codex 的 app-server 验证宿主启动及工具目录，无模型调用
python scripts/plugin-host-acceptance.py --codex '<桌面版 codex.exe 路径>' --version 0.2.4
```

工具选择样例在 tests/plugin-prompts.json。目标宿主回放每个 prompt，保存
`[{"id":"案例 ID","calls":[{"name":"实际工具名","arguments":{...}}]}]`，然后运行
plugin-acceptance.py --traces <文件>。脚本检查工具、禁止操作、只读限制和关键参数；
仍需人工核对提问、提交回执、视觉输出和部分失败说明。无 traces 时明确报告 not_run，
不会将测试样例存在或元数据结构检查冒充真实模型选择准确率。
