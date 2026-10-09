# RAW 开发与版本

RAW 使用 rawler 0.8.0，程序运行不依赖 Python/LibRaw。显式 `raw_develop` 选择版本 1 的传感器开发链，输出仍为 scene-linear sRGB D65、float32、straight alpha。`init` / `grade` / `analyze` 均接受 `--raw-develop options.json`，JSONL/MCP 对应可选字段 `raw_develop`；项目保存于每个 revision，渲染、分析、预览、对比读取所选修订。RAW 与 `input_space` 或 OCIO encoded input 互斥，OCIO managed input/display 可继续使用。

```powershell
target/release/tinge.exe raw-plan photo.CR2 --raw-develop examples/raw/as-shot.json
target/release/tinge.exe init photo.CR2 --project photo.tinge --raw-develop examples/raw/as-shot.json --color-pipeline examples/aces2-srgb.json
target/release/tinge.exe apply photo.tinge --expect-revision 0 --edits examples/raw/edits.json
target/release/tinge.exe preview photo.tinge --output preview.png
```

`raw_plan` 先解码并检查原始数据，解析选项和相机标定，不执行去马赛克，不修改项目。报告传感器尺寸、CFA/平面顺序、实际黑白电平、实际 WB 增益与来源、可选 illuminant tags、选定的 D65 适配矩阵、active/default crop 和方向。可请求最多 256 个 `sensor_points`，坐标为 `[x,y]`、原始传感器原点、未裁切/未旋转；每个点返回 native_values 与 normalized_before_wb，便于 agent 核对传感器值。

```json
{"command":"raw_plan","input":"photo.CR2","options":{},"sensor_points":[[100,100],[101,100]]}
```

| 参数 | 意义 |
| --- | --- |
| algorithm_version | 当前仅 1。显式空对象也选择 v1，不等于旧解码路径 |
| white_balance | `as_shot`（默认）、`unity`、`camera_gains {gains}`、`chromaticity {xy}`、`temperature {kelvin,tint_duv}` |
| exposure_ev | -20..20；在去马赛克和相机转换后乘 2^EV，不改变传感器饱和判断 |
| black_levels / white_levels | `{repeat:[columns,rows],values:[...]}`，原始 sensor units；行优先、单元内按 cpp 交错，元素数必须是 columns × rows × cpp，原点为未裁切 sensor origin |
| below_black | `clip` 默认将低于 black 的传感器归零；`preserve` 保留有符号归一化传感器样本。去马赛克自身的边缘/插值约束仍适用 |
| crop | `sensor` 全传感器、`active` 有效区域、`default` 默认摄影裁切（缺失时回退 active/full） |
| apply_orientation | 默认 true；EXIF 方向以浮点坐标变换应用 |
| calibration_illuminant | 可选 DNG/EXIF 数值 tag，选择解码器提供的相机矩阵（来自 RAW 或相机数据库）；`raw_plan` 列出可用 tags，缺失即失败，不自动换另一个；只支持有定义参考白点的 tags 1/4/17/18/19/20/21/22/23，其他 tags 明确拒绝 |

WB camera_gains 必须与相机颜色平面数量一致（三色为 3、四色为 4），按 `plane_order` 顺序，并归一化到平面 1（RGB 相机通常为绿色）。第四平面名称由 rawler 的 CFA enum 给出，Sony RGBE 的 E 在该库称 CYAN；不能从名称推断光谱响应。单色 RAW 只允许 as_shot/unity。as_shot 缺失或无效时，用选定 D65 适配矩阵与 xy=[0.3127,0.3290] 计算回退，并在报告标明 `matrix_d65_fallback` 和警告。xy 白平衡使用同一矩阵，并不是 Lightroom Temperature/Tint 滑杆或完整色度适应模型。

相机矩阵按 [DNG ColorMatrix 定义](https://developer.apple.com/documentation/imageio/kcgimagepropertydngcolormatrix1) 消耗标定光源下的 XYZ。v1 将 D65 XYZ 先按 Bradford 适配到标定光源，再进入 XYZ→camera 矩阵；A 光源的固定 f64 参考矩阵测试验证方向。Daylight/Flash 是 D65/D55 近似并报告警告，不是测得的光谱标定。旧未设置参数路径保留原有非 D65 适配算法，专业开发应显式选择 v1。

v1 在完整 sensor repeat pattern 上执行 `(sample-black)/(white-black)` 和相机 WB，再交给原生 PPG Bayer、X-Trans bilinear 或四色 bilinear。每个位置 white 必须大于 black；高于 white 不截断，RGB 转换后负值/HDR 继续保留，整数输出按选定显示链量化。裁切后 CFA 相位仍以原传感器为基准；项目初始开发校验使用已冻结源。不提供假装的高光恢复：曝光降低不能恢复已经传感器饱和的数据。

```json
{"command":"apply","project":"photo.tinge","expect_revision":1,"edits":[{"type":"set_raw_develop","options":{"exposure_ev":-0.5,"white_balance":{"type":"camera_gains","gains":[2.4,1,1.36]}}}]}
```

新 RAW 参数参与 revision hash 和 decode cache key；restore/branch/tag 会复制完整设置，失败事务不改变项目 JSON。`set_raw_develop {options:null}` 回到旧解码路径，旧项目不添加字段且保持已有 hash。旧路径大部分颜色 RAW 使用去马赛克后 WB；v1 将 WB 放在之前，因此 PPG 可能产生不同像素，这个差别是版本语义。缺失拍摄 WB 的旧路径也修复为选定矩阵 D65 回退。compare 的左侧是**同一修订的 RAW 开发结果、旁路节点图**，右侧执行节点图，不是前后两个 RAW 设置；比较不同 RAW 修订可分别 preview。LUT 烘焙采样独立 RGB，不能把 RAW 解码/开发烘焙进 cube；CDL 文件同样不包含 RAW 设置。

验收用四个 [Raw.pixls.us](https://raw.pixls.us/) 文件，逐条核对 catalogue CC0 链接与 SHA-256，见 `artifacts/raw-corpus-manifest.json`。真实样本是 Canon EOS 400D Bayer、Fuji X-E1 X-Trans、Leica M Monochrom 单色、Sony DSC-F828 四色。开发脚本 `scripts/raw-corpus-reference.py` 用独立 [rawpy 0.27.1 / LibRaw](https://letmaik.github.io/rawpy/api/rawpy.RawPy.html) 检查完整传感器尺寸和采样值；报告保留两解码器对 black/white/default crop 的差异。相同 sensor samples 不代表开发色彩与 Lightroom/Resolve 等价。相机 corpus 是四文件范围，不是所有机型/所有压缩模式验收。

尚未完成：完整光谱/其他 CCT 方法和目标软件色温/tint 对等、去马赛克算法选择、完整传感器类型及压缩模式、DNG CFAPlaneColor 四色完整解码、rotated Fuji v1 开发、非 D65 四色矩阵、dual illuminant/forward matrix/DCP/相机 ICC look、高光重建、传感器降噪与坏点校正、镜头配置与摄影元数据导出。当前 rawler DNG decoder 对四平面 CFAPlaneColor 的解释不完整，v1 检测不一致的 plane count 后拒绝；真实 Sony 四色 SRF 的独立解码路径可用。

当前实际报告：78 个 Rust 测试、四文件有限开发、1024 个 native sensor sample 和三个 CFA pattern 与 LibRaw 精确一致。项目恢复的 43776 个 float32 RGBA 像素由 [官方 OpenEXR 3.4.12](https://openexr.com/en/latest/python.html) 独立读取验证完全一致；并行 EXR chunk 布局使文件字节可不同。视觉检查发现 Sony 四色 bilinear 有紫色边缘伪色，去马赛克质量与外观对等验收仍未完成。

Kelvin/Tint：`white_balance:{type:"temperature",kelvin:6504,tint_duv:0.003}` 使用 [Kang 2002 Planckian-locus xy 拟合](https://colour.readthedocs.io/en/develop/generated/colour.temperature.CCT_to_xy_Kang2002.html)，有效 Kelvin 1667..25000；tint_duv 为 CIE **1960** uv 平面中相对拟合轨迹的法线位移，范围 -0.05..0.05，最终须为正 XYZ 且相机矩阵能表示该白点。切线使用所选多项式分支的解析导数。正 Duv 表示更绿的光源白点，相机 WB 补偿偏向品红。它不是 Adobe Tint 数值；6504K/Duv=0 是近似黑体白，**不是 D65**，D65 必须用 chromaticity xy=[0.3127,0.3290] 精确选择。`raw_plan` 报告最终 f32 white_balance_white_xy 和 camera gains。单色 RAW 拒绝色温。既有 RAW revision 的字段/默认值/hash 不变。33 组固定参考由 Colour Science 0.4.7 的 CCT 与 xy/uv 转换独立生成，tint 切线使用有限差分，对比 Rust 解析导数，绝对 xy 容差 3e-8；fixture 和脚本可重现。用例为 `examples/raw/temperature.json`。
