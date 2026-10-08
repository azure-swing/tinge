# Tinge

面向 agent 的本地图片调色插件，基于 VibeColor 原生 Rust CLI 引擎。目标是覆盖 DaVinci Resolve Studio 中适用于静态图片的调色、蒙版和图像处理，并补足 Lightroom 的摄影显影工作流。

**当前为 0.2 开发版，尚未达到完整功能对等。** 已实现可运行的浮点调色引擎、节点图、蒙版、非破坏性项目和 MCP；完整需求没有缩减。具体差距见 [功能覆盖表](docs/coverage.md)。原有语言约定和架构计划保存在 [原始设计](docs/original-design.md)。

## 构建与运行

已在 Windows x64 / MSVC / Rust 1.95 上构建验证。业务代码使用 Rust，RAW 使用 rawler，ICC 使用 moxcms，真实 OCIO 2.5.2 使用静态链接 C++ 库；程序运行不依赖 Python 或 Node。

```powershell
cargo build --release --locked
$vc = '.\target\release\vibecolor.exe'
& $vc capabilities
& $vc schema recipe
& $vc --help
```

第一次构建需要 CMake、C++17 编译器和 Git，并联网下载 crates.io / OCIO 外部依赖。Windows 使用 MSVC Build Tools 与 Windows SDK；Linux 使用 GCC/Clang、CMake 和 Git。Cargo.lock 固定 Rust 依赖解析，ocio-rs/ocio-sys 0.2.1 与 OCIO 2.5.2 源码保存在 vendor；二进制归档绑定修正见 [补丁说明](vendor/PATCHES.md)。CMake 可能使用系统依赖，尚不是完全隔离构建。

## 调一张图片

本地看图界面：`vibecolor view portrait.vcolor` 启动后自动打开网页。支持缩放、圈划、原图/历史版本对比和版本保存；agent 从 CLI/MCP 调色后页面自动更新。`--no-open` 返回可在 Codex 侧栏浏览器打开的本地 URL；`vibecolor selections portrait.vcolor` 读取圈选批注。预览采用带 ICC 标签的 sRGB SDR，专业显示校准/HDR 仍待实现。详见 [Web 查看器](docs/viewer-workflow.md)。

```powershell
# 直接执行配方，默认导出 16 位 PNG
& $vc grade photo.jpg --recipe examples/cinematic.json --output graded.png

# 创建非破坏性项目，冻结源图副本
& $vc init photo.jpg --project portrait.vcolor

# 一次事务提交蒙版、节点和输出，要求项目版本为 0
& $vc apply portrait.vcolor --expect-revision 0 --edits examples/local-edits.json

# agent 看图、读取统计和示波器，再迭代
& $vc preview portrait.vcolor --output preview.png
& $vc compare portrait.vcolor --output comparison.png
& $vc stats portrait.vcolor --scopes

# 输出 16 位 PNG / TIFF，或线性浮点 EXR / TIFF
& $vc render portrait.vcolor --output final.png
& $vc render portrait.vcolor --output final.exr
& $vc render portrait.vcolor --output final.tiff --bit-depth 32

# 恢复原始配方，产生新版本；已有历史保留
& $vc restore portrait.vcolor 0 --expect-revision 1
```

默认拒绝覆盖已有输出；需要覆盖时显式使用 `--overwrite`。输出禁止与源图、项目文件或项目资产冲突。

## Agent 协议

MCP 已按 [OpenAI Plugin guidelines](https://developers.openai.com/plugins/plugin-guidelines) 改为独立命名工具：`vibecolor_project_info`、`vibecolor_edit_preview`、`vibecolor_submit_edit_preview`、`vibecolor_render`、`vibecolor_cleanup_plan`、`vibecolor_finalize` 等。每个工具包含完整静态参数契约和显式安全标注。旧通用 MCP 工具 `vibecolor_agent` / `vibecolor_run`、通用 job_submit / batch 不再公开或接受；CLI/JSONL 的 command 请求格式保持兼容。这是 MCP 接口的破坏性迁移，需更新调用端。图片默认返回资源引用，统计显式请求；后台任务支持会话内幂等重试。见 [Agent 工作流](docs/agent-workflow.md)。

Tinge 的本地 Windows 插件模板在 `plugin/`，包含 manifest、STDIO 连接和图片工作流 Skill。运行 `pwsh -File scripts/package-plugin.ps1` 构建包含原生二进制、Skill 参考资料、许可证与本地 marketplace 的独立包；脚本输出包根目录。没有提交或发布动作。设计依据、迁移和验收边界见 [插件规范改造](docs/plugin-design.md)。

0.2.1 新增“定稿并清理”：选定最终版本后回收已登记的临时预览和其他版本临时导出，保留源图/资产/全部历史/最终导出。查看器不再写临时 PNG，Agent 预览可使用稳定缓存路径，无损 PNG/项目 JSON 进一步减少体积。详见 [临时文件工作流](docs/storage-workflow.md)。

无模型抠像支持颜色取样、GrabCut 框选/前景背景标记、trimap、Closed-Form alpha 精修和去溢色，可导出透明图片与独立 16 位灰度蒙版：

```powershell
& $vc cutout photo.png --options examples/cutout/grabcut.json --output subject.png --matte alpha.png
```

无需模型下载或额外运行库；复杂背景需少量标记。参数和项目节点见 [抠像工作流](docs/cutout-workflow.md)。

普通命令的成功结果写 stdout：`{"ok":true,"data":...}`。错误写 stderr，退出码 1；参数错误退出码 2。`--progress` 将节点进度作为 JSONL 写 stderr。`--help` 和 `--version` 输出普通文本。

- `schema request` / `schema recipe` / `schema edits`：与 Rust 类型一起生成 JSON Schema。
- `run request.json` / `run -`：结构化请求，支持文件或 stdin。
- `serve`：常驻 JSONL 服务，一行一个请求，一行一个响应，共享解码和节点缓存。
- `mcp`：MCP STDIO 服务，每个操作由独立工具暴露，不传 command；预览返回资源引用，也可请求内联 PNG。
- `batch manifest.json`：批量请求，逐项返回结果；失败时整体退出码为 1。JSONL 会报告批次中的逐项失败；MCP 不提供通用 batch。
- 项目修改必须携带 `expect_revision`。冲突返回 `revision_conflict`、`expected_revision` 和 `actual_revision`，不会覆盖其他修改。

MCP 客户端可使用以下通用 STDIO 配置，将 command 替换为实际绝对路径：

```json
{"mcpServers":{"tinge":{"command":"C:/path/to/vibecolor.exe","args":["mcp"]}}}
```

服务器支持初始化、工具枚举/调用、预览资源枚举/读取和 ping。同步工具与独立后台提交工具共存；任务可查询/协作取消，但重启恢复和完整宿主兼容验收仍待完成。详见 [协议说明](docs/protocol.md)。

## 引擎和文件

RAW 可使用版本化的传感器电平、相机 WB/xy、曝光、裁切方向和相机标定选择：`raw-plan photo.CR2` 检查实际解析参数，`init photo.CR2 --project raw.vcolor --raw-develop examples/raw/as-shot.json` 创建项目。使用方法和相机验收范围见 [RAW 工作流](docs/raw-workflow.md)。

真实 OCIO / ACES 使用方法见 [专业色彩链](docs/color-pipeline.md)。例如：

```powershell
& $vc ocio-configs
& $vc ocio-inspect --builtin studio-config-v4.0.0_aces-v2.0_ocio-v2.5
& $vc init photo.exr --project aces.vcolor --color-pipeline examples/aces2-srgb.json
& $vc preview aces.vcolor --output aces-preview.png
& $vc render aces.vcolor --output scene.exr

# ACES 2.0 HDR / P3 输出，agent 预览使用独立 SDR view
& $vc init photo.exr --project hdr.vcolor --color-pipeline examples/aces2-hdr1000.json
& $vc render hdr.vcolor --output hdr-pq.png
& $vc preview hdr.vcolor --output hdr-sdr.png
& $vc render hdr.vcolor --output acescg.exr --output-space '{"primaries":"aces_cg","transfer":"linear"}'

# 自定义配置及目录内 LUT 冻结为项目 OCIOZ 资产
& $vc init photo.jpg --project custom.vcolor --color-pipeline examples/custom-ocio/pipeline.json
& $vc preview custom.vcolor --output custom-preview.png

# 原生 Look 和指定 OCIO 空间中的 CDL 节点
& $vc validate examples/node-ocio/recipe.json --color-pipeline examples/node-ocio/pipeline.json
& $vc grade photo.jpg --recipe examples/node-ocio/recipe.json --color-pipeline examples/node-ocio/pipeline.json --output look.png
```

| 模块 | 当前实现 |
| --- | --- |
| `vibecolor-core` | 配方验证、DAG、蒙版、31 类算子（含无模型抠像）、统计和示波器 |
| `vibecolor-color` | RGB 原色矩阵、sRGB/gamma/PQ/HLG 传递函数 |
| `vibecolor-ocio` | 真实 OCIO、配置发现、色彩转换、ACES 输入/显示链 |
| `vibecolor-io` | 图片解码、RAW 适配、ICC、8/16/32 位原子导出 |
| `vibecolor-project` | 冻结源图与资产、版本、事务、分支、标签、恢复 |
| `vibecolor-engine` | CPU / Rayon 调度、内容缓存、节点进度、引擎取消标记 |
| `vibecolor-cli` | 原生 CLI、JSON/JSONL 和 MCP STDIO |

节点之间在 scene-linear sRGB/D65 中以 32 位浮点交换图像，使用 straight alpha。曝光、HDR EV 分区、原有矩阵、滤波和合成使用线性域；主色轮、曲线、HSL/warper、原有 CDL 和分离色调使用明确的 sRGB 编码域。新增 ocio_grade 在节点内部进入指定 OCIO 处理空间执行原生 CDL/矩阵，或按配置应用 Look，再返回交换域。操作的名称表示控制概念，**不表示与 Resolve 私有算法逐像素相同**。具体参数、单位和限制见 [算子说明](docs/operators.md)。

- 输入：PNG/JPEG/TIFF/EXR/HDR/WebP/BMP；RAW 相机与格式受 rawler 支持范围限制。
- RAW：传感器缩放、PPG Bayer / bilinear X-Trans 与四通道去马赛克、相机矩阵、拍摄白平衡、方向与裁剪；基础实现，不等于 Lightroom 完整显影。
- RGB ICC 自动转入线性工作空间；显式 `--input-space` 可覆盖文件元数据。支持部分标准 PNG cICP，优先于 ICC；CMYK/Gray ICC 和完整 cICP/自定义 PNG gamma/chromaticities 仍待实现。
- 未标记整数图假定 sRGB；未标记浮点图假定线性 sRGB。EXR 解释标准 colorInteropID 和 RGB chromaticities，冲突/未知 ID 需显式解决。
- 输出：PNG 8/16、JPEG 8、TIFF 8/16/32、EXR 32。SDR RGB 可选择 sRGB/P3/Rec.2020/ACEScg 原色和正确 ICC；线性 EXR 写入色彩标签与 chromaticities。Rec.2020 PQ 输出限 16 位 PNG，写 cICP。
- 整数导出最后裁剪 RGB 并报告。OCIO 支持 sRGB、P3、Rec.2100 PQ 显示输出；P3/PQ 项目为 agent 指定独立 SDR sRGB 预览 view。未启用时默认普通 sRGB，可添加 tone_map 或声明 output_space。手动 PQ 编码须声明 linear_unit_nits，完整 HLG/OOTF、HDR 静态元数据和校准显示仍待实现。
- EXIF/XMP/IPTC 尚不复制；JPEG 和 32 位 TIFF 当前不能输出透明图片。RGBA EXR 在文件中预乘、内部为 straight alpha；零 alpha 隐藏 RGB 导出时丢弃并报告，外部 additive EXR 暂不支持。
- 项目为 `.vcolor` JSON，旁边的 `.vcolor.assets/` 保存按内容命名的资产。搬迁时一起移动。`.lock` 文件只承载 OS 文件锁，不依赖删除文件解锁。
- 所有配方资源相对于配方文件目录解析；JSONL/MCP 的资源相对于当前目录或显式 `asset_base`。

## 验证

ASC CDL 文件可通过 `cdl-inspect`、`cdl-import`、`cdl-export` 交换。导入要求指定处理空间和风格，返回可直接用于 apply 的 ocio_grade op；SOP/饱和度与来源描述存进项目。导出所选节点的参数，并报告文件未表达的处理链上下文。详见 [CDL 工作流](docs/cdl-workflow.md)。

LUT 可检查纯 1D/3D 或组合 shaper，选择 trilinear/tetrahedral。纯颜色配方与项目修订可烘焙为独立 3D `.cube`，返回编码与采样误差；空间效果明确拒绝。用法见 [LUT 工作流](docs/lut-workflow.md)。

```powershell
& $vc lut-bake --recipe examples/lut-grade.json --output artifacts/grade.cube
& $vc lut-inspect artifacts/grade.cube
```

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p vibecolor-cli --example test_chart -- artifacts/test-chart.png
& $vc grade artifacts/test-chart.png --recipe examples/cinematic.json --output artifacts/graded.png

# 开发用独立 ICC 检查，需要 Pillow/ImageCms；原生程序不依赖它
python scripts/icc-reference.py
```

测试包括传递函数参考向量、色彩矩阵、ICC、高位深往返、RAW DNG、LUT 顺序/插值、局部蒙版、HDR/负值、透明滤波、事务失败/冲突、分支恢复，以及真实进程 CLI/MCP 工作流。OCIO 与官方 Python 绑定生成的固定向量对照，另验证编码输入、显示输出和项目色彩链历史。RAW 相机样本、校准显示、GPU 和 Resolve/LR 对照验收需要继续扩展。

独立 LittleCMS 检验四类 ICC 输出的 8 位 SDR 交换，最大差异为 1 个量化步长；OCIO 与固定官方 Python wheel 的 24 组转换/调色/Look、8 组 LUT 插值、20 组 CDL 文件/风格/方向参考用例对照。实际烘焙 LUT 和 CDL 文件也由官方 OCIO 独立读取核验。最新构建、测试和边界说明见 [当前状态](docs/status.md)。

## 后续完整功能目标

按覆盖表继续扩展 OCIO 外部依赖搬迁/节点工作域、完整 HDR 元数据与专业显示、完整 RAW 与摄影工作流、GPU、持久任务与重启恢复、AI 语义蒙版/深度/重光、人像工具、完整特效/插件宿主。未完成项不会在 capabilities 中报告为可用。

资料基线：[Resolve Color](https://www.blackmagicdesign.com/products/davinciresolve/color)、[Lightroom Develop](https://helpx.adobe.com/lightroom-classic/desktop/help/applying-adjustments-develop-module-basic.html)、[Lightroom Masking](https://helpx.adobe.com/be_en/lightroom-classic/desktop/process-and-develop-photos/masking.html)。这是功能规划来源，覆盖表是本项目的实现评估。

