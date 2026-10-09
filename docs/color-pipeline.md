# OCIO / ACES 色彩链

tinge 静态链接真实 OpenColorIO 2.5.2，通过 Rust 调用 C++ CPU processor。默认绑定的 stub 模式没有启用，运行时仍明确拒绝 stub。内置官方配置包含 ACES 1.3 和 ACES 2.0，配置名、引擎版本和 view 写入项目历史。

## 使用

```powershell
$vc = '.\target\release\tinge.exe'
& $vc ocio-configs
& $vc ocio-inspect --builtin studio-config-v4.0.0_aces-v2.0_ocio-v2.5
& $vc init scene.exr --project scene.tinge --color-pipeline examples/aces2-srgb.json
& $vc preview scene.tinge --output preview.png
& $vc render scene.tinge --output final.png
& $vc render scene.tinge --output scene-linear.exr

# 真正的 LogC4 编码图片；不是相机 RAW 文件
& $vc grade logc4.tif --recipe examples/cinematic.json --color-pipeline examples/logc4-aces2.json --output aces.png
```

`color_pipeline` 可用于 init、grade、analyze、validate。CLI 从 JSON 文件读取并以文件目录解析外部配置路径；run/serve/MCP 直接传入对象，init/validate 默认从当前目录解析，grade/analyze/apply 可使用 asset_base。内置配置必须从 ocio-configs 选择完整版本名称，不接受 default 等随版本改变的别名。自定义配置可用对象 `{ "type":"file", "path":"config.ocio" }`。

```json
{
  "config": "studio-config-v4.0.0_aces-v2.0_ocio-v2.5",
  "engine_version": "2.5.2",
  "input": { "type": "managed" },
  "view": "ACES 2.0 - SDR 100 nits (Rec.709)"
}
```

## 输入、工作与输出

- managed：现有 RAW 显影、RGB ICC 或 input_space 解码到线性 sRGB/D65；适用于普通照片和 RAW。RAW 目前仍使用基础显影后端。
- encoded：按图片文件解码后的 RGB 数值读取，跳过 ICC、gamma 和原色转换，由指定 OCIO color_space 转入声明的线性 working_space，再转换到内部 Frame。默认 working_space 为 `Linear Rec.709 (sRGB)`。可用 ARRI LogC4、S-Log3、ACEScg、DaVinci Intermediate 等；实际名称由 ocio-inspect 返回。此模式拒绝相机 RAW 和同时传入 input_space。
- 调色算子的工作域仍由算子定义：Frame 为线性 sRGB/D65；主色轮、部分曲线等按已有明确编码域执行。启用 ACES 不会把这些算子改成 Resolve 或 ACEScct 私有算法。
- 8/16 位输出：工作 RGB 经指定 display/view，写入已经编码的数值，再量化/裁剪，不会再次加 gamma。display 可为 `srgb`、`display_p3`、`rec2100_pq`，分别标记对应 ICC 或 cICP。PQ 限制为 16 位 Rec.2020 PNG。view `Raw` 被拒绝。
- 32 位 EXR/TIFF：保留 HDR 和负值，不烘焙显示 view；`output_space` 可选择支持的线性原色，默认线性 sRGB。
- preview 与 compare 始终输出 8 位 SDR sRGB。P3/PQ 项目必须显式指定适用于 sRGB 的 `preview_view`；这是单独的场景显示变换，不从已量化 HDR 文件反推预览。compare 两边使用相同解码和预览链。stats/scopes 仍测量工作 RGB，不是显示输出波形。

不启用 pipeline 时，保持原有 ICC/RAW 输入和普通 sRGB 输出语义。项目历史中的旧版无 pipeline 配方保持原有哈希，可直接读取。

## P3、HDR 和文件交换

```powershell
& $vc init scene.exr --project hdr.tinge --color-pipeline examples/aces2-hdr1000.json
& $vc render hdr.tinge --output hdr-pq.png
& $vc preview hdr.tinge --output hdr-sdr-preview.png
& $vc init scene.exr --project p3.tinge --color-pipeline examples/aces2-display-p3.json
& $vc render p3.tinge --output p3.png
& $vc render hdr.tinge --output acescg.exr --output-space '{"primaries":"aces_cg","transfer":"linear"}'
& $vc inspect hdr-pq.png
```

HDR 示例使用 ACES 2.0 的 1000 nits Rec.2020 view，以及独立的 100 nits Rec.709 预览 view。config、display、view、preview_view 全部参与项目修订哈希；旧 pipeline 省略 display 时仍为 sRGB，序列化保持兼容。整数 `output_space` 必须匹配 OCIO display，否则拒绝写文件。

PNG/JPEG/TIFF 的 SDR 输出支持 sRGB、Display P3、Rec.2020、ACEScg 原色，以及 linear/sRGB/gamma22/gamma24 传递函数。生成的 RGB ICC 使用标准 xy 白点、Bradford 到 ICC D50 PCS 的适配、对应 TRC；创建日期固定以保证复现。浮点 TIFF 保留负值和大于 1 的值，目前仅 RGB、不支持浮点 alpha。小尺寸 TIFF 的 ICC 独立读取，避免解码器的像素预算静默遗漏标签。

Rec.2020 PQ PNG 写实际 cICP `[9,16,0,1]`，不附带会造成误导的 SDR ICC。自动读取目前识别 primaries 1/9/12、transfer 8/13/16、RGB matrix 0、full range；优先级为显式 input_space > cICP > ICC。其他 cICP 报错并要求显式解释。PQ 解码的线性 1 对应 10000 nits；它是绝对显示亮度信号，自动读取不会逆解 ACES view。未启用 OCIO 的手动 PQ 导出必须传入正值 `linear_unit_nits`，声明工作 RGB 的 1 对应多少 nits；该选项不适用于已拥有亮度映射的 OCIO HDR 输出，也不实现 tone mapping。HLG 自动显示 OOTF 仍待实现。

EXR 写 `colorInteropID` 和一致的 chromaticities，以兼容旧应用。支持的标准 ID 为 `lin_rec709_scene`、`lin_p3d65_scene`、`lin_rec2020_scene`、`lin_ap1_scene`。读取标准 ID、检查与 chromaticities 的一致性，并可用矩阵转换未具名的自定义 RGB chromaticities；冲突、未知 ID、data 通道要求显式声明或 OCIO 输入。缺少色彩标签的旧 EXR 默认线性 sRGB。当前读取首个可用 RGBA 层，不是完整多层/deep EXR 编辑器。whiteLuminance 可检查但不自动改变场景曝光尺度。

EXR 按文件惯例预乘 alpha，读取时转换到内部 straight alpha。零 alpha 的隐藏 RGB 在导出时丢弃并报告；外部 EXR 的零 alpha additive RGB 当前拒绝，不能用 straight alpha 无损表示。显式 `tingeAlphaMode=straight` 的文件可读取，inspect 返回该模式。

文件格式依据：[PNG 3 cICP](https://www.w3.org/TR/png-3/)、[OpenEXR 技术说明](https://openexr.com/en/latest/TechnicalIntroduction.html)、[ASWF OpenEXR 色彩互操作建议](https://github.com/AcademySoftwareFoundation/ColorInterop/blob/main/Recommendations/04_OpenEXRFiles/OpenEXRFiles.md)。尚未输出 mDCV/cLLI 等 HDR 静态元数据，也未完成 HDR 校准显示验收。

## 事务和复现

修改显示 view 或输入编码需要新的 revision：

```json
{
  "command": "apply",
  "project": "scene.tinge",
  "expect_revision": 0,
  "edits": [
    {
      "type": "set_color_pipeline",
      "pipeline": {
        "config": "studio-config-v4.0.0_aces-v2.0_ocio-v2.5",
        "engine_version": "2.5.2",
        "input": { "type": "managed" },
        "view": "Un-tone-mapped"
      }
    }
  ]
}
```

`pipeline: null` 关闭链；已有 ocio_grade 节点时必须在同一事务移除或替换这些节点。配置参与 revision 的 recipe_hash；restore、branch、tag 保留或恢复目标修订的配置。解码缓存也包含完整配置。引擎版本不匹配时渲染失败，不能悄悄换成另一版 ACES。

## 自定义配置、上下文与冻结

```powershell
& $vc ocio-inspect --config-file examples/custom-ocio/config.ocio --context '{"GRADE":"warm"}'
& $vc init photo.jpg --project custom.tinge --color-pipeline examples/custom-ocio/pipeline.json
& $vc preview custom.tinge --output custom-preview.png
```

init 和 set_color_pipeline 会用 OCIO 原生归档器将配置及其工作目录内、具有受支持 LUT 扩展名的文件保存为完整二进制 OCIOZ。目录结构和未选中的 LUT 也保留，以支持后续视图/上下文选择。`.tinge.assets/<hash>.ocioz` 的路径和 BLAKE3 hash 写入 pipeline，随修订参与哈希；渲染和缓存命中前验证包内容。既有 OCIOZ 也可导入；从另一项目导入 frozen 配置时复制包到本项目资产目录。

归档使用 native isArchivable 约束：搜索路径和 FileTransform src 必须是配置工作目录内的相对路径；变量路径应有 `./` 等相对前缀，例如 `./$GRADE/look.cube`。绝对路径、`../` 和开头为变量的路径目前不自动重写，项目导入明确失败；目录外依赖搬迁仍待实现。归档扫描拒绝 link/junction，限制 100000 个目录项和 512 MiB LUT 数据。OCIOZ 不包含任意未知扩展名文件，也不表示所有自定义配置依赖已获得完整覆盖。

`context` 为字符串映射，显式值覆盖配置作者声明的默认值。使用新建的 OCIO Context，不加载进程环境变量，因此 GRADE/SHOT 等环境值不会悄悄改变项目。默认值保留在包中的配置里；显式值写入项目修订。inspect 返回 context_defaults、resolved_context 和 looks；转换报告返回实际 context、文件引用及 processor/cache ID。每次读取可编辑外部配置刷新 OCIO 文件缓存，以观察 LUT 修改/删除；one-shot grade 不冻结资产。

更换上下文应复用 show 返回的 frozen pipeline，以保持包内容不变：

```powershell
$p = (& $vc show custom.tinge | ConvertFrom-Json).data
$head = $p.history | Where-Object id -eq $p.revision
$pipeline = $head.color_pipeline
$pipeline.context.GRADE = 'neutral'
@{command='apply'; project='custom.tinge'; expect_revision=$p.revision;
  edits=@(@{type='set_color_pipeline'; pipeline=$pipeline})} |
  ConvertTo-Json -Depth 100 | & $vc run -
```

selected input、display 和 preview processors 在提交前编译，缺失 LUT/无效上下文拒绝提交。失败可能留下未引用的内容资产，但项目 JSON/revision 不变。restore、branch、tag 保留包和上下文。

`working_space` 是配置中的 OCIO 名称，`working_encoding` 声明它的实际线性 RGB 原色。默认 linear sRGB；可选线性 P3/Rec.2020/ACEScg。输入/显示链在该空间和 canonical Frame 之间转换，alpha 精确保留；节点算子仍使用现有 Frame/算子域，不等于任意节点域处理。必须正确声明配置空间的原色，非线性 working_encoding 被拒绝。`display_name` 和 `preview_display_name` 可指定自定义配置中的实际显示名称；`display` 仍声明文件输出编码，预览必须为 SDR sRGB。

归档与上下文语义依据：[OCIO Config API](https://opencolorio.readthedocs.io/en/v2.5.1/api/config.html)、[固定 OCIO 2.5.2 源码](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/include/OpenColorIO/OpenColorIO.h)。tinge vendored Rust 绑定增加显式字节长度 API，避免把 ZIP 当作 C 字符串截断，详见 vendor/PATCHES.md。

## 指定处理空间的调色节点与 Looks

`ocio_grade` 使用项目修订的 OCIO 配置/context，在节点内部执行原生 processor。CDL/矩阵的链是 canonical 线性 sRGB → 声明的线性 working_space → grade.color_space → 原生 grade → working_space → canonical。mask 和 mix 在返回 canonical 后执行；直通/禁用、后续滤波、分支合成和显示链仍使用统一交换域。节点改变场景图像，因此 float EXR/TIFF 也包含 grade，只省略显示 view。旧基础算子的默认域不变，尚未支持所有算子任意工作空间。

例如内置 ACES 2.0 配置中的 ACEScct CDL：

```json
{
  "nodes": [{
    "id": "log-cdl",
    "op": {
      "type": "ocio_grade",
      "grade": {
        "type": "cdl", "color_space": "ACEScct",
        "slope": [1.05, 0.94, 1.1], "offset": [-0.01, 0.02, 0],
        "power": [0.95, 1.1, 1.02], "saturation": 0.8,
        "style": "no_clamp", "inverse": false
      }
    },
    "mix": 0.75
  }],
  "output": "log-cdl"
}
```

CDL 使用 OCIO 的 ASC v1.2 SOP/saturation。style 为 `asc` 或默认 `no_clamp`；asc 裁剪 [0,1]，no_clamp 的负值行为由原生 OCIO 定义。inverse 要求 slope/saturation 为正；裁剪过的值无法通过 inverse 恢复。参数用 f64 保存、CPU 像素为 f32。RGB matrix 使用同样的 color_space 往返，字段为 3×3 `matrix`、RGB `offset` 与可选 `inverse`，禁止 alpha 混合，奇异逆矩阵在编译时拒绝。处理空间必须实际存在且不是 data。

Look 由配置作者定义 process_space，无须调用方猜测编码。节点 grade 可写 `{ "type":"look", "looks":"+Warm,+SoftContrast", "inverse":false }`。顺序、正负前缀和 `|` 缺失文件回退交给真实 OCIO；示例配置的 `Unavailable | +Warm` 会在缺失 LUT 时选择 Warm。未知 Look 名称不会自动当作可用效果，失败不会提交项目修订。native files/looks 元数据写入处理报告。这些语义依据 [OCIO transforms](https://opencolorio.readthedocs.io/en/stable/api/transforms.html)。

```powershell
& $vc validate examples/node-ocio/recipe.json --color-pipeline examples/node-ocio/pipeline.json
& $vc --progress grade photo.jpg --recipe examples/node-ocio/recipe.json --color-pipeline examples/node-ocio/pipeline.json --output look.png
```

validate 的 ocio_nodes 返回每个节点的实际 processor ID、context、files、looks。项目提交和渲染会编译所有 OCIO 节点，包括未连接/禁用节点；缓存命中仍先检查依赖/包 hash。节点缓存身份包含实际 config/processor ID 与 working_encoding，editable LUT 更新会使缓存失效，依赖缺失先报错。`--progress` 的已启用 OCIO 节点额外返回 ocio_transform，缓存命中也可审计 processor。

## 数值工具与验证

```json
{
  "command": "ocio_transform",
  "config": { "type": "builtin", "name": "studio-config-v4.0.0_aces-v2.0_ocio-v2.5" },
  "transform": { "type": "color_space", "source": "ACEScg", "destination": "Linear Rec.709 (sRGB)" },
  "pixels": [[0.18, 0.18, 0.18, 1.0]]
}
```

transform 支持 color_space、display_view 和 grade。display_view 包含 source、display、view、inverse；grade 使用 `{ "type":"grade", "source":"ACEScg", "grade":{...} }`，输入/输出均为 source 编码，内部原生 grade 与节点字段相同。这类数值工具不假设输入/输出是 Frame，可处理 Log、广色域及 HDR 编码值；不会替调用者添加输出文件标签。返回实际 config/processor cache ID、引擎版本、context、files/looks 和转换后的 RGBA，straight alpha 精确保留。外部配置可用 `{ "type": "file", "path": "config.ocio" }`。

参考向量由固定官方 OpenColorIO 2.5.2 Python wheel 生成，独立于 Rust FFI；9 组内置转换和 2 组自定义上下文转换覆盖 ACEScg、sRGB、LogC4、S-Log3、DaVinci Intermediate、ACES 2.0 SDR/P3/PQ HDR 与目录 LUT。新增 13 组 grade 参考覆盖 ACEScct/DaVinci Intermediate/linear CDL、ASC/no-clamp、正反向、ACEScg 矩阵、Look 顺序/逆向/context/缺失文件回退，总计 24 组。包含灰阶、颜色、HDR、负值、透明度。包测试删除原配置/LUT 后对照固定向量，项目测试包括搬迁、上下文切换、恢复、环境变量干扰、缓存后篡改拒绝；节点测试验证 canonical mask/mix、float 输出包含 grade、editable LUT 缓存失效和无效事务不改项目。Python 仅为开发 oracle，运行时不需要；重新生成脚本见 scripts/ocio-reference.py。

ASC .cc/.ccc/.cdl 参数交换与来源描述固化已接入，见 [CDL 工作流](cdl-workflow.md)。另有 20 组文件/风格/方向参考用例，实际导出文件由官方 OCIO 独立读取核验。LUT 的组合 shaper、tetrahedral 和纯颜色图烘焙见 [LUT 工作流](lut-workflow.md)。

仍待完成：所有基础算子的任意节点域、目录外/非标准依赖完整搬迁、CDL 引用/扩展 XML/决定级元数据与实机验收、HLG/完整 HDR 元数据与格式、校准 viewer、soft proofing、完整 RCM/Resolve 对照验收。这一阶段不表示整个专业色彩管理或全部 Resolve 功能完成。

RAW managed input 可带修订化 `raw_develop`，先执行版本化 sensor normalization/WB/去马赛克/相机矩阵，再进入 canonical Frame；不能与 encoded input 或 input_space 同用。v1 不包含 DCP/双光源插值，见 [RAW 工作流](raw-workflow.md)。
