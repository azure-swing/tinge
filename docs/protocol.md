# Agent 协议

0.2.1 的 preview/compare/edit_preview 可省略 output，复用项目内工作文件。render 的 temporary 默认为 false，正式导出不会因定稿被回收。用户选定最终修订后调用 cleanup_plan/finalize；定稿保留源图和完整历史，返回逐文件回收结果。参数、重试和平台限制见 [storage-workflow.md](storage-workflow.md)。

## 请求与结果

CLI `run`、JSONL `serve` 继续使用带 `command` 的 `Request` 枚举。MCP 改为独立 `vibecolor_<操作>` 工具，参数不含 command；`tools/list` 为每项操作直接给出完整 schema、描述和 readOnlyHint/destructiveHint/openWorldHint。后台操作使用 `vibecolor_submit_<操作>`，参数与同步操作相同并增加可选 idempotency_key。通用 MCP 执行器、batch、job_submit 不再接受。以下带 command 的示例均为 CLI/JSONL 格式；MCP 使用对应工具并去掉 command。`vibecolor_schema` 只用于补充查看配方/算子结构，不是执行前提。

```json
{"command":"capabilities"}
{"command":"schema","target":"edit_preview"}
{"command":"schema","target":"op:primary"}
{"command":"schema","kind":"request"}
{"command":"schema","kind":"recipe"}
```

示例事务（相对资源路径由 `asset_base` 指定，默认当前工作目录）：

```json
{
  "command":"apply",
  "project":"portrait.vcolor",
  "expect_revision":0,
  "label":"Lift the subject",
  "edits":[
    {"type":"set_mask","id":"face","mask":{"type":"ellipse","center":[0.5,0.4],"radius":[0.16,0.2],"rotation":0,"feather":0.3}},
    {"type":"upsert_node","node":{"id":"face-light","inputs":["source"],"op":{"type":"exposure","stops":0.4},"mask":"face"}},
    {"type":"set_output","id":"face-light"}
  ]
}
```

按顺序处理所有 edits，然后验证整个配方并一次提交。事务可以暂时删除依赖后重新连接。失败不产生新 revision，不修改项目 JSON；在失败前已经复制的内容寻址资产可能保留为未引用文件，后续 GC 待实现。

冲突：

```json
{"ok":false,"error":{"code":"revision_conflict","message":"revision conflict: expected 0, actual 1","expected_revision":0,"actual_revision":1}}
```

调用方需要重新 `project_info`（`include_recipe:true`），确认当前配方再提交，不应盲目替换 expected_revision 重试。

## 版本、分支和保存

`raw_plan {input,options?,sensor_points?}` 解析实际 RAW 开发参数和最多 256 个未裁切/未旋转传感器点。`init/grade/analyze` 支持 `raw_develop`，`set_raw_develop {options}` 事务将完整设置写入 revision，`options:null` 恢复旧解码路径。修改先执行实际开发验证有限值，再原子提交；设置进入 revision hash/decode cache，restore/branch/tag 随设置复制。RAW 与 input_space/OCIO encoded input 冲突。报告区分 as-shot 和矩阵 D65 回退；xy/相机增益不是 Lightroom 色温/tint，见 [RAW 工作流](raw-workflow.md)。

`cdl_inspect {input}` 返回 .cc/.ccc/.cdl 文档和来源 hash。`cdl_import {input,selector?,color_space,style,inverse?}` 要求明确的 color_space 和 asc/no_clamp style，多条文档须显式 `{type:"id",id}` 或 `{type:"index",index}` 选择；返回 grade 与可用于 apply 的 op，不自动修改项目。处理空间是否存在由 apply/validate 的实际 color_pipeline 编译确认。导入的 grade.exchange 保存历史来源 hash、格式、条目身份和标准描述，不依赖原文件。`cdl_export {source,output,overwrite?}` 接受 `{type:"document",document}` 或 `{type:"project",project,revision?,nodes}`；项目导出只读所选修订，报告所选节点/色彩链，XML 仅包含存储的正向 SOP/饱和度和描述，不表达 style/inverse/mask/mix，不等于烘焙节点图。未知 XML/引用明确拒绝；导出用原生读取器重读并核对参数与元数据后原子写入。见 [CDL 工作流](cdl-workflow.md)。

`lut_inspect {input}` 返回 .cube 的内容 hash、1D/3D 表尺寸、独立定义域和组合状态。`lut_bake {source,output,options?,overwrite?,asset_base?}` 的 source 严格区分 `{type:"recipe",recipe,color_pipeline?}` 和 `{type:"project",project,revision?}`。项目烘焙只读所选修订，不创建新 revision，不接受 asset_base；配方使用 asset_base/CWD。默认 33³、sRGB 编码输入/输出、[0,1] 域、2048 个 Halton 检验点及 tetrahedral 验证插值。结果包括输出 hash、snapshot_recipe_hash、处理器身份与 RGB 最大/平均/RMS 采样误差；它不是全域误差上界。输出编码不自动使用项目 display，必须明确选择 `{type:"display"}`。空间效果/空间蒙版拒绝烘焙，颜色范围选区可参与。详见 [LUT 工作流](lut-workflow.md)。

`show` 返回完整 history、branch/tag 和 frozen source。修改总是产生单调递增 revision。`restore` 将目标历史配方复制为新 revision，历史不删除。branch/tag 创建也产生元数据提交，避免其他 agent 对旧版本成功写入。

`branch` 不带 checkout：创建分支引用，然后在当前分支产生元数据提交；已有分支则保留其原位置。带 checkout：切到目标分支，并将其配方复制为新的提交，其 parent 是操作前的 revision。当前实现 branch 是配方版本管理，不是 Git merge 图；不支持合并冲突解决。

项目修改使用 OS 排他文件锁，JSON 通过同目录 tempfile + persist 原子替换。修改前检查版本。源图、LUT、bitmap matte 使用内容 hash 命名，渲染重新验证。`.assets` 必须跟项目一起搬迁。

## 预览与分析

本地 Web 界面由 `view <project> [--no-open] [--port N]` 启动；它只提供看图/圈划/对比/版本保存。CLI `selections <project>` 或请求 `selections {project}` 返回独立批注记录。`selection_save {project,expect_revision,revision?,mask,note?}` 执行原生渲染获取输出基准，再原子追加 `.selections.json`，不会改调色图或 revision。记录包含源/配方 hash、输出节点、全尺寸和归一化 Mask；不接受外部 bitmap/颜色 qualifier。agent 必须核对坐标基准再创建局部节点，不能盲目套用旧修订选区。详见 [Web 查看器工作流](viewer-workflow.md)。

```json
{"command":"preview","project":"portrait.vcolor","output":"preview.png","max_edge":1600}
{"command":"compare","project":"portrait.vcolor","output":"compare.png"}
{"command":"stats","project":"portrait.vcolor","scopes":true}
```

预览先按全分辨率执行配方，再缩小；pixel radius 和 crop 参数不因 max_edge 改变。启用 color_pipeline 时执行固定 OCIO sRGB display/view，其他情况为普通 sRGB 编码，最终整数输出裁剪越界并报告。compare 左为同一修订的源图开发结果（旁路节点图）、右为节点图结果，两边使用相同 RAW 设置、输入/显示链，独立适配 max_edge，几何改变后可能尺寸不同。init/grade/analyze 支持 color_pipeline，set_color_pipeline 事务支持修订/关闭；详见 color-pipeline.md。

render/grade 支持 `output_space`（primaries/transfer）和手动 PQ 的 `linear_unit_nits`。整数 OCIO 输出必须与 pipeline display 的色彩标签一致；浮点输出为选定线性原色，不烘焙显示 view。非 sRGB 项目必须声明 `preview_view`；preview/compare 返回实际 `preview_pipeline`，始终为 SDR sRGB。inspect 和 export 报告包含 `color_metadata`（实际 cICP、ICC hash、EXR colorInteropID/chromaticities/alpha mode），便于 agent 核对文件链。详细支持边界见 color-pipeline.md。

pipeline.config 保持旧内置字符串兼容，也接受 file/frozen 对象。项目导入将可归档自定义配置冻结为 hash 校验的 OCIOZ；context 默认使用配置作者声明值，显式字符串映射参与修订，不加载进程环境。working_space/working_encoding 为 OCIO 名称与线性原色契约；display_name/preview_display_name 对应自定义配置名称。ocio_inspect/ocio_transform 也接受 context。转换报告增加实际 context 和文件引用；项目缓存命中前仍校验 frozen 包，详细目录和容量限制见 color-pipeline.md。

`op.type=ocio_grade` 的 grade 接受 cdl/matrix/look，使用当前修订 color_pipeline 的配置和 context。CDL/矩阵的 color_space 明确处理域，Look 使用配置自身的 process_space；节点间始终返回 canonical linear Frame，随后混合 mask/mix。validate 可传 color_pipeline，并返回 ocio_nodes 的真实 processor 身份与依赖；只有参数检查通过不足以验证外部 LUT。apply 和渲染会编译整个配方中的 OCIO 节点，再提交/查缓存。带这种节点的配方不能单独关闭 pipeline，需同事务移除或替换节点。数值 ocio_transform 增加 `{type:"grade",source:"...",grade:{...}}`，输入输出均为 source 编码；不同于图像节点，不自动转换 canonical 原色。转换报告包含 OCIO 原生 files/looks 元数据。

stats 分位/均值在 scene-linear sRGB 中测量，直方图在编码 sRGB 中分箱。waveform/parade 为 256×256 密度表，vectorscope 为 Rec.709 Cb/Cr 密度表。当前统计包括透明像素 RGB，非前景 alpha-weighted 分析。gray-world WB 仅为统计建议，不代表自动找到了中性物体。

## MCP STDIO

`cutout {input,output,options,matte?,input_space?,color_pipeline?,raw_develop?,bit_depth?,output_space?,linear_unit_nits?,overwrite?,asset_base?}` 执行无模型颜色/GrabCut/trimap 抠像。options 与 `op.type=cutout` 共用 schema；结构化调用返回 alpha 求解、工作尺寸和警告。matte 是不做颜色转换的独立 16 位灰度 PNG，透明输出支持 PNG、8/16 位 TIFF、EXR。参数、标记坐标、精修限制和项目冻结规则见 [抠像工作流](cutout-workflow.md)。抠像输出不会自动注册 MCP 预览资源，项目 preview 仍使用既有资源协议。

JSON-RPC 2.0，一行一个消息，无 Content-Length 帧。stdout 仅协议，诊断/可选进度在 stderr。支持协议版本 2025-11-25、2025-06-18、2024-11-05 的共同工具与资源子集。尚未完成官方 SDK 客户端兼容矩阵，仅有真实进程协议测试。

所有命名工具默认在 structuredContent 返回数据，text 给出短状态，图片使用 resource_link；`_inline_image:true` 可直接显示图片，`_response:"full"` 可取完整诊断。preview 的 include_analysis 默认为 false，与返回模式独立。这两个下划线字段仅属于 MCP 外层，不能嵌入 CLI/JSONL Request。同步与后台工具的 schema 都严格拒绝未知字段；同名工具只能执行自己的操作。

调用失败以 isError 返回（包括版本冲突、部分预览/清理失败和 failed 任务状态），协议方法/参数错误使用 JSON-RPC error。未知工具、command 注入、错误业务参数返回工具错误。preview 资源只枚举和读取当前服务会话生成的 URI，读取验证 hash；重启后需要重新生成。原图和任意路径不作为资源开放。MCP 资源 URI 的可用性不等于磁盘项目状态。

CLI、JSONL、MCP、batch 和后台任务使用相同的部分失败判定。编辑已提交但
预览失败时，CLI 的 stdout 保留完整 `{ok:false,data:...}` 回执并以 1 退出；
JSONL 返回 `ok:false` 并继续服务；MCP 返回 `isError:true`，简短文字也标明失败。
必须检查 `committed`、`revision`、`preview_error`，不要因失败标志直接重做编辑。
清理的 `failed` 文件列表或非空 `registry_error` 同样传播失败。
仅有 `workfile_warning`（例如导出到项目目录之外）仍表示导出成功、有登记警告。

## 缓存、进度和批量

Engine 的内容 key 包含源像素、算子、上游 key、mask 参数与 LUT/bitmap 内容；项目源图解码以 source hash+声明色彩空间+完整 OCIO pipeline 缓存。预算默认 512 MiB，超预算清空旧缓存。不是持久 tile cache。

OCIO 节点还包含实际 config/processor cache ID、显式 context、引用元数据和锚点 working_encoding。editable 配置在每次请求重新加载，依赖缺失先报错，LUT 更新使节点缓存失效；frozen 配置仍检查包 hash。`--progress` 的 OCIO 节点记录额外 ocio_transform，即使节点命中缓存也返回实际处理身份；禁用节点不报告已应用变换。

`--progress` 按完成节点输出 completed/total/cached/elapsed_ms。旧命令仍同步执行；CLI/JSONL 的 `job_submit` 或 MCP 的 `vibecolor_submit_<操作>` 将耗时操作交给一个有界后台工作线程，同一 stdin 可继续查询/取消任务。取消在引擎检查点生效，RAW 解码、部分空间算子和导出不能即时打断。任务/幂等 key 属于当前会话，持久任务与重启恢复尚未实现。缓存默认闲置 60 秒后释放；`configure` 调整主引擎和任务引擎各自的预算/闲置期限，`cache_info` 查询实际缓存像素字节。预算不限制整个进程的峰值内存。详见 [Agent 工作流](agent-workflow.md)。

batch 输入是 request 数组，结果按 index 记录。默认继续处理其他项目；stop_on_error 可提前结束。batch 不是跨项目事务，之前成功的文件/提交保留。普通 CLI batch 任意失败退出码 1；JSONL 报告逐项失败；MCP 不暴露通用 batch。JSONL 没有 request ID 多路复用，严格按行顺序返回。
