# ASC CDL 文件交换

`cdl-inspect`、`cdl-import`、`cdl-export` 与 JSONL/MCP 请求共用原生实现。读取由真实 OCIO 2.5.2 执行；输入结构先严格检查，避免拼错参数、重复节点或未解析引用被忽略。文件交换保存正向 SOP/饱和度参数及标准描述，处理空间和应用方式由调用方明确指定。

## 检查与导入

```powershell
$vc = '.\target\release\vibecolor.exe'
& $vc cdl-inspect examples/cdl/looks.ccc
& $vc cdl-import examples/cdl/looks.ccc --id 1 --color-space ACEScct --style no-clamp
```

JSON/MCP：

```json
{"command":"cdl_import","input":"examples/cdl/looks.ccc","selector":{"type":"id","id":"1"},"color_space":"ACEScct","style":"no_clamp","inverse":false}
```

`.cc` 为单条 ColorCorrection，`.ccc` 为 ColorCorrectionCollection，`.cdl` 为带内联 ColorCorrection 的 ColorDecisionList。多条文档必须显式选择；ID 区分大小写，index 从 0 开始。数字 ID 与 index 分开，`id:"1"` 不会回退成索引 1。CLI 的 `--id` 和 `--index` 互斥。

`color_space` 与 `style` 必填；CLI style 为 `asc` / `no-clamp`，JSON 为 `asc` / `no_clamp`。文件中的 SOP/SAT 不编码 OCIO 风格或处理空间，不能从扩展名推断 Log 域。inverse 默认 false；逆向使用要求 slope/saturation 严格大于零，ASC 被裁剪的信息不可恢复。

响应包含原文件 BLAKE3 hash、所选 index、`grade` 和 `op`。可把 op 放入普通 apply 事务：

```powershell
$entry = (& $vc cdl-import examples/cdl/looks.ccc --id 1 --color-space ACEScct --style no-clamp) | ConvertFrom-Json
$edits = @(
  @{type='upsert_node'; node=@{id='imported_cdl'; op=$entry.data.op}},
  @{type='set_output'; id='imported_cdl'}
) | ConvertTo-Json -Depth 100
$edits | & $vc apply photo.vcolor --expect-revision 0 --edits - --label 'Import CDL'
```

项目须有有效的 color_pipeline，比如 `examples/aces2-srgb.json`。import 只检查参数/名称，不假定配置；apply 会实际编译具名处理空间，失败不提交。节点间仍交换 canonical linear sRGB；OCIO 在节点内部转入 ACEScct 等处理域再返回，mask/mix 随后作用。

`grade.exchange` 把历史来源 hash、格式、条目 ID/index、集合与条目描述保存到项目。数值 SOP/饱和度是内联值，删除/修改原 CDL 文件不影响项目。来源 hash 记录导入历史，不是当前参数的 hash；后续编辑可改变参数。没有 exchange 的旧节点序列化保持不变，旧修订 hash 兼容。

## 导出

```powershell
& $vc cdl-export --project photo.vcolor --revision 1 --nodes imported_cdl --output saved.ccc
& $vc cdl-export --document examples/cdl/document.json --output saved.cc
```

多节点使用 `--nodes node1,node2`；按给定顺序输出。文档 JSON 也可直接作为 MCP source：

```json
{
  "command":"cdl_export",
  "source":{"type":"document","document":{"format":"cc","corrections":[{"id":"balance","slope":[1.05,0.94,1.1],"offset":[-0.01,0.02,0],"power":[0.95,1.1,1.02],"saturation":0.8}]}},
  "output":"saved.cc"
}
```

项目导出只读所选修订，不创建 revision。必须选择原生 ocio_grade CDL 节点；不把旧 sRGB 算子或整个节点图自动拟合成 CDL。节点 source_context 随报告返回，包含实际 grade/style/inverse、输入连接、mask、mix、enabled 与修订色彩链。

XML 输出的是存储的正向参数，**不编码处理空间、风格、inverse、图连接、蒙版和 mix**。把它当作参数交换使用；要重现完整纯颜色图，应使用 [LUT 烘焙](lut-workflow.md)，明确输入/输出编码。重新导入 CDL 时仍须指定解释方式。

文档 format 必须匹配输出扩展名。.cc 只接受一条 correction 且没有集合描述；需要保留集合描述时用 .ccc/.cdl。集合中的非空 ID 必须唯一，节点无导入 ID 时使用节点 ID。不同来源的集合描述冲突时拒绝合并，需分开导出。XML root 的标准 namespace 为 urn:ASC:CDL:v1.01；输入也支持 v1.2 或无 namespace。

Rust 写入 f64 可往返精度，XML 特殊字符只转义一次；输出先由 OCIO 重读并核对所有参数/描述，成功后原子写入。默认不覆盖；指定 overwrite 仍保护项目、项目源图和不可变资产，CLI 也保护 document JSON 输入文件。

## 元数据、限制与验收

保留 collection/correction 的 Description、InputDescription、ViewingDescription，以及 SOPNode/SatNode 的重复 Description，分别出现在 metadata 的 descriptions/input_descriptions/viewing_descriptions/sop_descriptions/sat_descriptions 数组。Unicode 和 XML 实体按解码文本保存，数字按 f64 保存。未知属性/元素、ColorCorrectionRef、决定级额外描述、嵌套描述、重复参数、DTD/声明实体、非 UTF-8、错误 root/扩展名明确拒绝。UTF-8 XML 可含内置实体和字符引用。输入/输出上限 16 MiB、4096 条 correction，树解析限制 262144 个节点。

SOPNode 若出现须含一组 Slope/Offset/Power，SatNode 若出现须含一项 Saturation；省略整组时沿用 OCIO 的 identity 默认。参数必须有限，slope/saturation 非负、power 正数。源文件先复制至唯一临时快照再读取，避免同名文件的原生缓存返回旧值；临时文件的全局 OCIO cache 随即释放，已有 processor 仍拥有自己的数据。

开发用 `scripts/cdl-reference.py` 从官方 OCIO 2.5.2 Python wheel 生成 .cc/.ccc/.cdl 共 20 组、120 个 RGBA 向量，覆盖 ASC/no-clamp、正反向、负值/HDR/alpha、数字与 Unicode ID。Rust 测试对照数值、实际 CLI/MCP、来源删除后的浮点复现、项目导出、完整 f64/XML 描述往返和事务失败原子性。Python 不参与软件运行。

本实现没有使用 OCIO 的 CDL XML writer：2.5.2 的 [CDLWriter](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/src/OpenColorIO/fileformats/cdl/CDLWriter.cpp) 预先转义描述，随后 [XmlFormatter](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/src/OpenColorIO/fileformats/xmlutils/XMLWriterUtils.cpp) 再转义；[数值 writer](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.5.2/src/OpenColorIO/ParseUtils.cpp) 使用 16 位有效数字。实际往返测试暴露了描述重复转义，因此用 Rust 单次转义和完整精度，再以原生 reader 验证文件。读取/风格语义基于 [OCIO CDLTransform](https://opencolorio.readthedocs.io/en/v2.5.2/api/transforms.html)。

不代表 Resolve/Lightroom 实机交换认证。引用式决定、扩展 XML、全部决定级元数据与目标软件验收仍需补齐；完整目标保持在 [覆盖表](coverage.md)。
