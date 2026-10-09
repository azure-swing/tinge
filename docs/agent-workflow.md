# Agent 工作流

MCP 使用独立命名工具，每项工具直接提供完整静态 schema 与显式安全标注。
不传 command，不通过通用执行器调用隐藏操作。CLI/JSONL 继续接受原有 Request。
本地插件运行仅需要原生二进制，不依赖 Python、Node、模型下载或额外守护进程。

## 一次调色迭代

常规流程：读取项目 → 编辑并看预览 → 需要时导出。

- 新图片可直接 init；inspect/raw_plan 只在需要检查格式、色彩解释或 RAW 参数时调用。
- 已有项目首次用 `tinge_project_info` 读取修订，基础调色省略配方。
  历史默认不返回（history_limit=0）；查看历史时显式指定 1..100。
- 普通调色用 `tinge_adjust`，一次合并曝光、对比度、饱和度、自然饱和度、白平衡和明暗。
  看图时加 `_inline_image:true`，自动使用托管路径，无需另外 preview 或 resources/read。
- 高级节点图、蒙版才加载 `tinge_edit_preview`，并用 `include_recipe:true` 读取配方。
- 成功后沿用回执的 revision；发生冲突、重连或已知外部修改时再读项目。
  compare、stats、scopes、capabilities 和完整 schema 均按需调用。

例如对初始空配方的项目：

```json
{"name":"tinge_project_info","arguments":{"project":"portrait.tinge"}}
{"name":"tinge_adjust","arguments":{"project":"portrait.tinge","expect_revision":0,"exposure":0.2,"contrast":1.1,"saturation":1.05,"highlights":-0.1,"_inline_image":true}}
```

以上为 tools/call 的 params。adjust 的值是绝对值，省略/null 保留原值，显式中性值可重置。
默认 adjustment_id 为 basic，复用 ID 更新同一组，不叠加；新 ID 才在当前输出后追加。
组由 `__tinge_<ID>_wb/primary/tone` 三节点组成，保留下游节点和输出。
白平衡为场景线性 RGB 增益 [0.001..100]，不是色温 K；[1,1,1] 中性。
组被高级编辑改坏或旁路时拒绝自动更新，需协调配方或有意使用新 ID。
每个工具已有完整静态 schema，不必先查询 `tinge_schema`；只在需要单个算子、
蒙版或 edit 的说明时使用它。

重型工作在同一工具上设 `background:true`，以实际返回的 job ID 查询 job_status，
不紧密轮询。每次新编辑使用新 idempotency_key；同会话相同请求重试复用 key。
取消不回滚已提交编辑；预览失败也可能带 committed/revision/preview_error。
先核对回执，不能因为没看到图片就再次编辑。冲突后核对新配方，不盲目替换版本号。

支持后台模式的工具为 adjust/edit_preview/preview/compare/render/grade/stats/analyze/finalize。
省略 background 或 false 同步执行；idempotency_key 只用于 background:true，任务仍属当前会话。
这些工具统一标为非只读，因为后台模式会创建任务；stats/analyze 同步调用仍只读取图片。
源图统计用 analyze（不接收 recipe），项目配方统计用 stats；LUT 烘焙用 lut_bake 的
project/revision。直接配方处理保留在高级 grade/validate/节点编辑中，CLI/JSONL 不变。

## 结果、文件与授权范围

结果只在 structuredContent 中返回一次；compact/full 的 text 均为短状态，不复制 JSON。
图片默认返回 resource_link。
需要像素时使用 resources/read 或 `_inline_image:true`；完整诊断使用 `_response:"full"`。
这两个字段仅属于 MCP 外层。preview 的 include_analysis 默认为 false，
完整模式也不会自动启用分析；统计可单独调用 tinge_stats。
`_response:"full"` 也不会自动内联图片；传输像素须显式 `_inline_image:true`。
compact 模式对直接 analyze 和后台 analyze 结果同样省略 histogram。
相同源/配方的统计最多缓存 8 项。
统计缓存同时区分源图的 input_space，避免同一文件的不同色彩解释串用结果。

预览先按全分辨率执行配方再缩小，空间算子参数不因预览尺寸改变。
图片 URI 读取会验证文件 hash，传输上限为 32 MiB；不会把旧 URI 悄悄指向新像素。
同步与后台预览共享的会话资源索引最多 64 项；旧 URI 可能淘汰，重新生成预览
可恢复引用。索引淘汰不会删除文件，也不是磁盘清理操作。
adjust 自动使用托管路径；preview/compare/edit_preview 可省略 output，使用已登记的项目工作路径。
preview/compare 的稳定缓存文件可能被替换，所以不是只读操作。
显式输出默认拒绝覆盖，只有 overwrite:true 才允许，并继续保护源图、项目和资产。

render 的同步/后台模式均不隐式定稿或清理。temporary 默认为 false，正式输出保留；
temporary:true 登记为可清理草稿。仅当用户已选定最终 revision 并授权清理时，
先用 tinge_cleanup_plan 查看范围，再调用 tinge_finalize（可设 background:true）。
回收是破坏性操作；可恢复不等于非破坏性。平台和部分失败行为见
[临时文件工作流](storage-workflow.md)。

## 会话、重试与容量

后台惰性创建一个工作线程，队列最多 8 项，保留 64 个任务的完整结果，
请求限 1 MiB、结果限 4 MiB。队列满返回 retryable 的 queue_full。
同步耗时工具仍会占用协议线程；后台工具允许主协议继续处理轻量控制请求。

同一 key 与相同规范化请求在同一会话返回同一任务，不同请求返回 idempotency_conflict。
结果淘汰后保留小型回执，旧重试返回 expired/commit，不重复执行。
每会话最多保留 4096 个 key。key 限 128 UTF-8 字节。
重启丢失任务和 key；重连后必须读取项目/输出状态，再判断是否重试。
取消在引擎检查点生效，RAW 解码、部分空间算子、事务和导出无法任意时刻打断。

## 缓存与可选界面

主协议与后台引擎各自默认最多缓存 512 MiB 像素，闲置 60 秒释放。
tinge_configure 可设 0..4096 MiB、1..3600 秒；0 禁用像素缓存。
tinge_clear_cache 清理主引擎并请求后台在当前操作结束后释放缓存。
tinge_cache_info 的字节数不包含活动渲染、PNG 缓冲、OCIO、查看器和分配器。
这是缓存预算，不是进程内存限制；完整解码的 24MP float32 RGBA 帧约 366 MiB。

tinge_viewer_open 在同一会话复用规范项目路径对应的查看服务，返回 URL 和 PID。
端口复用时沿用原值；只管理本会话子进程，viewer_close 和会话结束会关闭它们。
独立 CLI 仍使用 view。查看器预算独立，configure 不改变它。
查看器是可选的；编辑、预览资源与导出可以完全脱离 UI。

## 验证

Rust 回归覆盖静态 schema 的传递引用、安全标注、隐藏执行器拒绝、操作注入拒绝、
分页、资源 hash、精简返回、版本冲突、提交后取消、幂等淘汰和缓存回收。
scripts/agent-acceptance.py 验证真实 MCP 进程、幂等重试、资源与查看器生命周期；
提供 --raw-project 可进一步验证真实 RAW 任务和闲置缓存。
scripts/plugin-acceptance.py 检查工具契约和本地插件包；tests/plugin-prompts.json
包含直接、间接、误触发、缺失输入和失败重试样例，需要在目标宿主回放后导入 traces。
结构检查通过不代表真实模型工具选择已通过。见 [插件设计与验收](plugin-design.md)。
