# LUT 检查、插值与烘焙

`lut-inspect`、`lut-bake` 与 JSONL/MCP 的 `lut_inspect`、`lut_bake` 共用原生实现。烘焙导出独立 `.cube`，同时返回明确的颜色编码、依赖快照身份和采样误差。

## 读取和应用

```powershell
$vc = '.\target\release\vibecolor.exe'
& $vc lut-inspect looks/warm.cube
```

节点参数示例：

```json
{"id":"look","op":{"type":"lut","path":"looks/warm.cube","domain":"srgb","interpolation":"tetrahedral"}}
```

`domain` 选择节点内部的 sRGB 编码或 canonical linear sRGB；`.cube` 自身的 DOMAIN/INPUT_RANGE 仍决定查表坐标。未指定 interpolation 的旧配方保持 trilinear，序列化不新增默认字段，既有修订 hash 不变。1D 表始终线性插值，3D 可选 trilinear/tetrahedral。输入越界 clamp 到表边界，输出不裁剪；alpha 原样传递。

支持 Iridas 纯 1D/3D 的逐通道 DOMAIN_MIN/MAX，Resolve 的标量 LUT_1D_INPUT_RANGE / LUT_3D_INPUT_RANGE，以及 Resolve 1D shaper 后接 3D 的组合文件；两级范围各自生效。1D 尺寸 2..65536，3D 尺寸 2..129，红通道变化最快。拒绝重复/迟到头部、非有限数、错误行数、错序范围和含糊的混合方言；组合/Resolve INPUT_RANGE 不带 TITLE。纯 Iridas 文件可带 TITLE。不支持 .3dl 等其他格式。

格式语义对照官方 OCIO 2.5.2 的 [Iridas Cube reader](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/src/OpenColorIO/fileformats/FileFormatIridasCube.cpp) 与 [Resolve Cube reader](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/src/OpenColorIO/fileformats/FileFormatResolveCube.cpp)。官方 Python wheel 独立生成 8 组、1112 个向量，覆盖范围、组合 shaper、非线性交叉通道、两种 3D 插值和域外边界；RGB 容差 3e-6 × max(1,abs(expected))。脚本 `scripts/lut-reference.py` 仅为开发工具。

## 烘焙配方或项目修订

```powershell
& $vc lut-bake --recipe examples/lut-grade.json --output artifacts/grade.cube
& $vc lut-bake --project artifacts/node-ocio-demo.vcolor --revision 1 `
  --options examples/lut-display-options.json --output artifacts/node-ocio-baked.cube
```

配方可另传 `--color-pipeline`；所有资源按各自 JSON 所在目录解析。项目使用冻结的资产和修订色彩链，项目 JSON 不变。JSON/MCP 示例：

```json
{
  "command":"lut_bake",
  "source":{"type":"recipe","recipe":{"nodes":[{"id":"balance","op":{"type":"white_balance","gains":[1.04,1,0.96]}}],"output":"balance"}},
  "output":"grade.cube",
  "options":{"size":33,"validation_samples":2048,"validation_interpolation":"tetrahedral"}
}
```

默认输入/输出均为编码 sRGB，输入域 [0,1]。支持以下编码，未启用的 OCIO 链不能使用后两项：

| encoding | 参数与行为 |
| --- | --- |
| standard | `{"type":"standard","space":{"primaries":"srgb","transfer":"linear"}}`；使用支持的原色/传递函数 |
| ocio | `{"type":"ocio","color_space":"ACEScct"}`；由真实 OCIO 转入/转出声明的线性锚点，再与 canonical Frame 交换 |
| display | `{"type":"display"}`；仅输出，应用项目选定的实际 display/view |

`options` 另接受 domain_min/domain_max（三通道，有限且严格递增）、title、size（2..129）、validation_samples（1..65536）、validation_interpolation。默认 size=33、samples=2048、tetrahedral；插值设置用于误差测量，.cube 不记录消费者插值，导入时应明确选择。手动 standard PQ 输入/输出必须给正数 linear_unit_nits，表示 canonical 线性 1 对应的 nits；其他情形不得传此参数。标准 HLG 当前只有 scene OETF，不表示显示 OOTF。

项目 input 声明用于图片解码；烘焙输入由 input_encoding 独立指定。默认输出是场景调色结果经标准 sRGB 编码，选择 display 才包含显示变换。Log 到 Log 的 LUT 应在对应 Log 处理域使用，不能当作普通 sRGB LUT 节点直接套图。导出的纯 3D Iridas 文件保留负值/HDR 输出，并用可往返 f32 的十进制写入；不声称消费者均保留超范围值。

## 可表达范围与误差

烘焙支持逐像素颜色算子、原生 ocio_grade、分支合成、节点 mix、亮度/HSL qualifier，以及这些范围蒙版的合并/反选。仅检查输出的有效祖先是否为纯颜色计算；禁用/不参与输出的空间节点不影响烘焙，但整个配方仍须合法，声明的 OCIO 节点仍编译验证。

Blur、sharpen、clarity、denoise、vignette、grain、glow、crop、resize、rotate、lens distortion、clone，以及几何/渐变/画笔/bitmap 蒙版依赖位置或邻域，烘焙明确报错。采样 alpha=1；独立 RGB LUT 不表达透明合成的所有行为。

先把 LUT/蒙版和 OCIO 配置依赖冻结到临时内容寻址快照，再生成 RGB 网格和检验点。采样结束后重读将要输出的 .cube，与同一冻结配方的 Halton 检验点结果比较。输出仅在全部成功后原子写入；默认不覆盖，指定 overwrite 仍保护直接输入、原配置、原生处理器实际引用的外部 LUT 和项目不可变资产。

报告给出各通道 max_absolute_rgb、mean_absolute_rgb、rms_rgb、worst_input_rgb、snapshot_recipe_hash、节点及输入/输出 processor 身份和超范围输出通道数量。误差单位是所选输出编码的 RGB 数值；点集位于声明输入域，**采样误差不是全域上界**。增加网格密度可能改善非线性误差，阶跃选区和极端 HDR 域仍需针对用途检查。Frame 采用 f32；原色换算内部采用 f64 矩阵后返回 f32，PQ 近黑和饱和色的往返仍受浮点消减影响，空配方也不能假定严格零误差。

其他 LUT 格式、1D/shaper 输出烘焙、自动逆 LUT、完整软件实机交换验收仍待实现。完整 Resolve/Lightroom 目标见 [覆盖表](coverage.md)。

发布验收可运行 `scripts/lut-bake-reference.py`（开发机需 Pillow 和官方 OCIO 2.5.2 Python wheel）。它独立读取实际导出的 ACEScct CDL LUT，比较原生输入/调色/输出链的 4096 个颜色点；另把项目的 Looks/CDL/mix/display 烘焙后重新导入成 LUT 项目，比较工程图片。报告为 `artifacts/lut-bake-reference.json`，对比图左为原节点图、右为烘焙 LUT。实际误差与特定编码/配方关联，不代表所有 LUT 或 Resolve 实机对等。
