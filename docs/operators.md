# 配方和算子参数

配方 schema v1 使用 `nodes`、`output`、`masks`。保留 ID `source` 表示源图。每个节点有 `id`、`inputs`、`op`、可选 `mask`、`mix`（0..1，默认 1）和 `enabled`（默认 true）。除 blend 外需要一个输入；blend 需要两个。整个图（包括未连接或禁用节点）都参与参数/依赖/循环检查，渲染只执行 output 的祖先。

运行 `tinge schema recipe` 获取字段类型和可选默认值。参数单位如下，范围以运行时验证为准。

| `op.type` | 字段与语义 | 处理域 |
| --- | --- | --- |
| cutout | options.selection 为 color/grab_cut/mask/trimap；refinement、grow、feather、decontaminate、despill；output 为 cutout/matte；见 [抠像工作流](cutout-workflow.md) | 颜色 key/GMM 使用 clamped encoded sRGB；alpha matting 默认 scene-linear（可选 srgb）；颜色重建线性；乘原图 alpha |
| ocio_grade | grade.type 为 cdl/matrix/look；需 color_pipeline；参数和例子见 color-pipeline.md | 原生 OCIO 处理空间往返；mask/mix 在返回线性 sRGB 后执行，alpha 不变 |
| exposure | stops -24..24；每档乘 2 | 线性 |
| white_balance | gains[RGB] 0.001..100，通常将 green 归一化为 1 | 线性 |
| primary | exposure；lift/offset -4..4；gamma 0.01..10，gain 0..100；contrast 0..4，pivot 0.001..1；saturation 0..4，vibrance -1..1 | LGGO 使用 sRGB 编码域；曝光/contrast/saturation 使用线性域 |
| tone | shadows/highlights/whites/blacks -1..1；固定亮度区域 ±2 EV 简化映射 | 按编码亮度求权重、在线性域曝光 |
| cdl | slope 0..100、offset -4..4、power 0.01..10、saturation 0..4 | sRGB 编码 SOP→sat；SOP 负值归零 |
| log_wheels | shadows/midtones/highlights RGB -4..4；low<high，0..1；falloff 0.001..1 | sRGB 编码、亮度区间 |
| hdr_zone | min_ev<max_ev，-32..32；stops ±24；color RGB ±8（EV）；falloff 0.001..8 EV | 线性，以 18% 灰为 0 EV |
| curves | channel rgb/red/green/blue/luma；points `[x,y]`，2..4096，x 0..1 严格递增，y ±8 | sRGB 编码、分段线性；端点外按斜率 1 延伸 |
| hue_curves | mode hue_vs_hue/hue_vs_sat/hue_vs_lum/lum_vs_sat/sat_vs_sat/sat_vs_lum；points 同上 | sRGB 编码 HSV；hue_vs_hue 的 y 是 hue 偏移，其余是倍率；Lum 当前使用 V |
| hsl | bands；hue 0..1，width 0.001..0.5，hue_shift ±1 周期，saturation/luminance 0..4 | sRGB 编码 HSV；luminance 参数当前作用于 V，非感知亮度 |
| color_warper | points；from/to `[hue,saturation]` 0..1；radius 0.001..2 | sRGB 编码 HSV，径向权重；不是 Resolve mesh 复刻 |
| rgb_mixer | 3×3 matrix ±10；preserve_luminance | 线性；保亮度在输出亮度非零时按比值缩放 |
| split_tone | shadows/highlights RGB ±4；balance ±1；strength 0..4 | sRGB 编码，亮度权重偏移 |
| lut | path 指向 `.cube`；domain srgb（默认）或 linear；interpolation trilinear（默认）或 tetrahedral | Iridas 1D/3D DOMAIN、Resolve INPUT_RANGE 与 1D shaper + 3D；定义域外 clamp，表输出保留负值/HDR；详见 [LUT 工作流](lut-workflow.md) |
| tone_map | method reinhard/aces_fit；white 0.001..10000 | 线性，以 white 除输入；aces_fit 是拟合近似，不是 OCIO/ACES |
| gamut_compress | threshold 0.01..100；softness 0.001..10 | 线性、围绕 Rec.709 luma 压缩 chroma；不保证输出处于显示 gamut |
| blur | radius 0.01..100 像素，是 Gaussian sigma，支持半径 ceil(3σ) | 线性、premultiplied alpha 过滤；边界 clamp |
| sharpen | radius 同 blur；amount 0..10；threshold 0..1 线性差值 | 线性 unsharp，不自动裁剪 |
| clarity | radius 同 blur；amount -2..2 | 线性，中间亮度区间局部对比 |
| denoise | radius 1..12 px；spatial_sigma 0.01..100 px；range_sigma 0.0001..10 | 线性 RGB bilateral；alpha 不改 |
| vignette | amount ±8 EV；center `[x,y]`；radius 0.001..2；feather 0.001..1 | 线性；归一化椭圆度量，非光学 profile |
| grain | amount 0..1 编码幅度；seed u64；monochrome | 编码 sRGB，固定 hash 噪声，无胶片颗粒物理模型 |
| glow | radius 同 blur；threshold 0..100 线性亮度；amount 0..10 | 线性提取亮部后模糊相加 |
| crop | x/y/width/height 整数，单位输入像素；完全位于输入内 | 不改变像素 |
| resize | width/height，正整数，最多 100M pixels | 线性、premultiplied alpha；缩小使用随比例扩展滤波范围的 Lanczos3 抗混叠，放大双线性 |
| rotate | degrees 是 90 的整数倍 | 无插值旋转 |
| lens_distortion | k1/k2 ±2；center `[x,y]` | 径向反向采样；图外 transparent；非相机镜头 profile |
| clone | source/target `[x,y]`；radius 0.001..1；feather 0.001..1 | 原输入取样、羽化替换，非内容修复 |
| blend | mode normal/add/multiply/screen/overlay/difference | 线性、foreground alpha-over；mix/mask 在合成后插值 |

`[x,y]` 默认是当前节点输入图片的归一化坐标，原点在左上，x 向右、y 向下；采样位于像素中心。几何节点 crop/resize/rotate 不允许 mask 或 mix<1。几何之前和之后的 mask 属于不同节点输入坐标空间。

## 蒙版

- ellipse：center、radius `[x,y]`、rotation（degrees）、feather（向内 0..1）。
- rectangle：min/max `[x,y]`、feather（归一化向内距离）。
- polygon：3..4096 points、feather（归一化向内距离）；直线边界。
- linear_gradient：start/end，分别为 0 和 1 权重，不能重合。
- brush：points 轨迹、radius（归一化）、hardness 0..1；轨迹覆盖而非笔压/flow 累积。
- luma_range：min/max（线性工作空间亮度）、softness，min<max。
- hsl_range：hue、hue_width（周期单位）、saturation/luminance 范围 0..1、softness；此 qualifier 使用实际 HSL 的 S/L。
- bitmap：path，灰度 intensity 作为 mask，必须与节点输入尺寸完全一致；不会自动用 alpha 或缩放。
- combine：mode union/intersect/subtract，masks 数组；union=`1-(1-a)(1-b)`，intersect=`ab`，subtract=`a(1-b)`。
- invert：mask 子对象，权重=`1-a`。

所有蒙版都是非破坏性的；alpha 不变时节点输出=`input + mask × mix × (processed-input)`。alpha 改变时，先按 mask×mix 权重插值预乘 RGB 与 alpha，再解除预乘，避免透明端 RGB 造成黑边。合成节点使用前景的 straight alpha 做 alpha-over，再应用节点 mask/mix。蒙版 feather 参数并非统一像素宽度，制作跨几何配方时应显式检查。

`ocio_grade` 的 CDL 由 OCIO 执行原生 ASC v1.2，独立于旧 `cdl` 基础算子。color_space 可为 ACEScct、DaVinci Intermediate、ACEScg 等配置中真实存在的颜色空间；data 空间拒绝。slope/saturation 有限且非负、power 有限且正、offset 有限；inverse 还要求 slope/saturation 正值。style 默认 no_clamp，asc 会按标准裁剪，逆变换不能恢复已裁剪的信息。matrix 为有限 RGB 3×3 + RGB offset，支持 inverse；奇异逆矩阵拒绝，不暴露 alpha 混合。look 使用配置 authored process_space，支持 OCIO 原生顺序、正负前缀、逆向和缺失文件回退。所有声明节点在提交/缓存命中前编译，包括禁用或未连接节点。

## 数值限制

工作图像最大 100M pixels；512 nodes、256 named masks；蒙版深度最多 16，combine 子项最多 64；HSL bands 最多 64，warp points 最多 256。CPU 缓存默认 512 MiB，并不等于总进程内存上限。滤波、原图、并行分支和导出缓冲仍可能同时占用较大内存，tile 执行待实现。

整数输出裁剪是显式导出行为；LUT、CDL SOP、HSV 饱和度、tone_map 等算子也有其明示 clamp/归零语义，不能把“float32”理解为所有算子都无限制保留负值。
