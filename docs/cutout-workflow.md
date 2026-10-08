# 无模型抠像

VibeColor 在原生 Rust CPU 中执行颜色抠像、交互式 GrabCut 和 Closed-Form Matting。无需网络、模型权重、Python 或 OpenCV 运行库。Python/OpenCV/PyMatting 仅用于开发对照，不随 CLI 分发。复杂背景需要主体框或少量前景/背景标记；没有按语义自动选择人、天空、物体的能力。

## CLI 与 agent

```powershell
.\target\release\vibecolor.exe cutout input.png --options examples/cutout/color.json --output cutout.png --matte alpha.png
.\target\release\vibecolor.exe cutout input.jpg --options examples/cutout/grabcut.json --output subject.png --matte alpha.png
```

`--options` 是严格 JSON；也支持 `-` 从 stdin 读取。相对蒙版路径以 options 文件目录为基准。`--input-space`、`--color-pipeline`、`--raw-develop` 与 grade 一致，RAW 也可先显影再抠像。透明输出允许 PNG、8/16 位 TIFF、EXR；JPEG 与 32 位 TIFF 会拒绝。默认 PNG/TIFF 16 位、EXR 32 位。`--matte` 是独立的 **16 位灰度 PNG 数据**，数值直接等于最终透明度，不经过 gamma/ICC/显示转换；不能把有 gamma 的蒙版预览当作数据。所有文件单独原子写入，多个输出不是一个事务；已有文件默认不覆盖，使用 `--overwrite` 明确允许。

JSON、JSONL、MCP 都使用同一个请求：

```json
{
  "command": "cutout",
  "input": "input.png",
  "output": "subject.png",
  "matte": "alpha.png",
  "options": {
    "selection": {
      "type": "grab_cut",
      "rect": [0.1, 0.05, 0.9, 0.95],
      "foreground": [{ "points": [[0.5, 0.5]], "radius": 0.015 }]
    },
    "refinement": { "method": "closed_form", "radius": 3 },
    "decontaminate": 0.7
  }
}
```

`asset_base` 可在结构化请求指定相对外部资产的目录。直接命令读取资产但不冻结；需要恢复历史时使用项目节点。输出包含工作尺寸、最终透明/不透明/半透明像素数、求解迭代/残差/收敛状态及警告。达到迭代上限仍输出有限解并报告未收敛，agent 应检查蒙版或提高迭代数。

agent 推荐循环：取得原图/预览尺寸 → 给出主体框或标记 → 生成透明 PNG/蒙版 → 检查误选 → 增加前景/背景标记 → 重新运行。现阶段没有交互 GUI，标记以 JSON 坐标提交。

## 选择方式

所有坐标都是当前节点输入图像的归一化坐标 `[0,1]`：像素中心 `(x+0.5)/width,(y+0.5)/height`。RAW/EXIF 方向和上游裁切已经生效。端点 1 的取样会落到最后一个像素。

| selection.type | 输入与行为 |
|---|---|
| `color` | `color` 是背景的 encoded sRGB `[R,G,B]`，或 `samples` 提供背景取样点；两者可以同时给出。对最近取样颜色计算 RGB 距离除以 √3，小于 `tolerance` 为背景，在其后 `softness` 宽度内平滑转为前景。默认 0.08/0.12。适合绿幕/白底等。 |
| `grab_cut` | `rect=[min_x,min_y,max_x,max_y]` 之外是确定背景，框内初始为可能前景；或省略框，同时提供 foreground/background strokes。使用 5 分量全协方差颜色 GMM、8 邻域图割迭代。`iterations` 1..20，默认 5；`smoothness` 0..100，默认 50。 |
| `mask` | `mask` 接受现有几何、画笔、亮度/HSL、bitmap、组合蒙版，包括反相。蒙版 1 保留、0 去除。外部 bitmap 必须与输入同尺寸。 |
| `trimap` | `path` 指向与输入同尺寸的灰度图。≤0.01 确定背景，≥0.99 确定前景，其余未知；未知值的大小不是预设 alpha。此模式自动执行 Closed-Form 求解。图像按标量数据读取，不做颜色管理；RGBA 的 alpha 不作为灰度 trimap。 |

Stroke 的 `points` 是连线画笔，`radius` 相对图像短边，默认 0.01；0 仍会标记邻近像素。前景/背景重叠、前景落在框外会报错，禁止默默覆盖。所有确定标记在最终缩放、精修、grow、feather 后重新应用，因此标记保持确定值。框外仍是背景。分辨率降低后相邻标记可能冲突，应提高 `max_edge` 或移动标记。

GrabCut 默认最长边 512，范围 16..1024；这限制图割 CPU/内存，并不限制输出尺寸。微小毛发在该尺度可能丢失，不能声称自动恢复未被选出的细节。

## 精修与颜色

`refinement.method` 默认为 `none`；`closed_form` 根据初始蒙版自动生成未知边缘带，`radius` 默认 3、范围 0..32，以 **精修工作尺寸像素**计。已有部分透明像素也作为未知区域。显式 trimap 不使用自动边缘带。`refinement.space` 默认 `linear`，在 scene-linear RGB 中估计物理混合比例，负值/HDR 不裁切，只用全图最大绝对 RGB（至少 1）归一化求解幅值；`srgb` 可兼容在 encoded/clamped sRGB 上求解的工作流。两者所得 alpha 可能不同。求解使用 Levin/Lischinski/Weiss 的 3×3 matting Laplacian、epsilon=1e-7，消去确定像素并使用 Jacobi 预条件共轭梯度；未知变量的 1e-8 正则化先验为 0.5。确定像素保持 0/1，需要同时有确定前景和背景。

精修 `max_edge` 默认 1024，范围 16..2048；未知像素上限 500000，超限需要降低尺寸或缩窄 trimap。`iterations` 默认 120、范围 1..1000，`tolerance` 为 RHS 范数归一化残差，默认 1e-6、范围 1e-10..0.01。降采样时 trimap 最近邻保持类别，精修后用颜色引导的联合双边上采样返回原尺寸；很薄的结构仍受工作分辨率限制。显式 trimap 原尺寸确定像素也会恢复。

`grow` 是原尺寸方形核膨胀/腐蚀半径，正数扩大、负数收缩，范围 -32..32。`feather` 是原尺寸高斯 sigma，范围 0..32。它们在 alpha 求解后执行；显式 trimap 的确定值也会受到这些主动形态调整影响，GrabCut 确定标记最后恢复。

`decontaminate` 0..1，默认 0，以附近不透明前景/背景作为先验，在 scene-linear RGB 中正则化求解 `I = alpha F + (1-alpha) B`，减轻旧背景白边/绿边。它是局部近似，找不到两类样本时不改 RGB；复杂纹理、全透明结构不能保证恢复真实前景。重建可能产生负值/HDR，浮点输出保留，整数输出按既有规则裁切并报告。`despill={"color":[0,1,0],"amount":0.7}` 去除绿色等优势通道溢色，作用于保留前景而不仅是边缘；会影响主体本来同色的部分，按需启用。

最终 alpha 为 `原图 alpha × 选择 alpha`，不会让原图透明区域变得不透明。内部 RGB 是 straight alpha，完全透明 RGB 清零。EXR 按既有预乘 alpha 交换规则导出。

## 项目节点与独立蒙版

```json
[
  {"type":"upsert_node","node":{"id":"key","op":{"type":"cutout","options":{"selection":{"type":"color","color":[0,1,0]}}}}},
  {"type":"set_output","id":"key"}
]
```

`apply` 使用既有 `--expect-revision`。节点支持任意上游输入，`mix`/节点 mask 在预乘空间混合改变 alpha 的结果，避免部分混合导致黑边。外部 trimap/bitmap 自动冻结、参与修订 hash/缓存标识，并在缓存命中前校验不可变资产；分支/标签/恢复会保留设置与资产。

节点 `options.output="matte"` 将 **最终 alpha** 放入 RGB，图像本身 alpha=1，可作为图中后续处理输入。该 RGB 仍受普通 render 的颜色管理，数据交换应选择 linear sRGB 输出；单独 CLI `--matte` 更直接。`cutout` 命令要求 options.output 为默认 cutout。空间分割/alpha 变化不能烘焙为 RGB `.cube` LUT，LUT bake 会拒绝 cutout 节点。

算法参考：[GrabCut 原论文](https://www.microsoft.com/en-us/research/wp-content/uploads/2004/08/siggraph04-grabcut.pdf)、[OpenCV GrabCut 交互说明](https://docs.opencv.org/4.12.0/d8/d83/tutorial_py_grabcut.html)、[PyMatting Closed-Form Laplacian](https://pymatting.github.io/laplacian.html)。颜色恢复实现是上述局部正则化方案，不是 PyMatting 的 multilevel foreground 算法，也不是 GrabCut 原论文的 border matting。
