# 完整功能覆盖与验收

目标仍是 DaVinci Resolve Studio 的静态图片调色、蒙版、修饰与图像处理功能，加上 Lightroom 的 RAW 和摄影工作流。CLI 负责让这些能力可被 agent 发现、参数化、检查和重现。

状态：**已实现**表示存在可执行代码；**基础**表示只有部分控制/算法；**待实现**表示没有可用后端。基础项目不计入完整对等。下表是本项目的需求映射，不是官方兼容认证；仍需按目标版本手册逐项补齐 Resolve FX 清单和新增功能。

参考基线（2026-10-07 查阅）：[Resolve Color](https://www.blackmagicdesign.com/products/davinciresolve/color)、[Resolve Studio](https://www.blackmagicdesign.com/products/davinciresolve/studio)、[Lightroom Develop](https://helpx.adobe.com/lightroom-classic/desktop/help/applying-adjustments-develop-module-basic.html)、[Lightroom Masking](https://helpx.adobe.com/be_en/lightroom-classic/desktop/process-and-develop-photos/masking.html)。

| ID | 图片阶段要求 | 状态 | 已有实现 / 剩余工作 |
| --- | --- | --- | --- |
| GR01 | 32 位浮点、非破坏性处理 | 已实现 | scene-linear sRGB，负值/HDR 保留，原图冻结 |
| GR02 | 串行、并行节点和图验证 | 基础 | 任意 DAG 和二输入合成；专用 parallel mixer、group/shared/compound 节点待补 |
| GR03 | 图层合成与混合模式 | 基础 | normal/add/multiply/screen/overlay/difference；完整模式和专用 layer mixer 待补 |
| GR04 | Lift/Gamma/Gain/Offset | 基础 | RGB 控制和节点 mix；精确色轮行为、主色条与亮度分离控制待验收 |
| GR05 | 曝光、对比、pivot、饱和度、自然饱和度 | 基础 | 可执行；Resolve/LR 参数响应和亮度保护待对照 |
| GR06 | 白平衡、色温、色调和中性采样 | 基础 | RGB gains、RAW 拍摄 WB、gray-world 建议；Kelvin/tint/中性点交互待补 |
| GR07 | 阴影、高光、白场、黑场 | 基础 | 简化亮度区间曝光；完整 highlight recovery 和局部重建待补 |
| GR08 | Log 色轮与可调分界 | 基础 | 阴影/中间调/高光、边界、falloff；专业 Log 输入响应待补 |
| GR09 | HDR 分区色轮与自定义区域 | 基础 | EV 区间曝光和 RGB 偏移；感知恒色、各 HDR 色彩空间待补 |
| GR10 | RGB、单通道与亮度曲线 | 基础 | 分段线性曲线；样条、软裁剪、通道独立联动待补 |
| GR11 | Hue/Sat/Lum 六类曲线 | 基础 | 六类可执行；Hue 周期插值、感知亮度与专业响应待补 |
| GR12 | RGB mixer / 单色混合 | 基础 | 3×3 矩阵、保亮度；摄影 B&W 色段混合待补 |
| GR13 | ASC CDL | 基础 | 旧 sRGB SOP；OCIO 原生 ASC v1.2/no-clamp、正反向、显式处理空间；.cc/.ccc/.cdl 参数检查/导入/导出、ID/index 选择、来源 hash 与标准描述、项目修订参数导出，官方向量和文件往返验收；ColorCorrectionRef/扩展 XML、决定级元数据、目标软件实机对照仍待补 |
| GR14 | HSL 八色段、分离色调 | 基础 | 任意 hue band 与双区 split tone；三向 grading/midtones 和 LR 一致性待补 |
| GR15 | Color Warper | 基础 | hue/saturation 控制点径向插值；网格约束、pin、luminance 网格和新版工具待补 |
| GR16 | ColorSlice / 色彩矢量工具 | 待实现 | 色段密度、饱和度、肤色保护、空间定义 |
| GR17 | Printer lights、通道色彩平衡 | 待实现 | 打印光单位、可复现参数语义 |
| GR18 | 自动平衡、Color Match、参考图匹配 | 待实现 | 灰卡/色卡测量、参考图色彩迁移与残差报告 |
| GR19 | Gallery、静帧、PowerGrade、版本对比 | 基础 | 配方 JSON、历史、branch/tag、before/after；gallery 管理与交换格式待补 |
| MS01 | 几何 Power Windows | 基础 | ellipse/rectangle/polygon，羽化、ellipse 旋转；Bezier、完整 window transform 待补 |
| MS02 | 线性、径向渐变、画笔 | 基础 | 线性渐变、ellipse 羽化、brush 轨迹；压力、flow、累积笔触、独立径向曲线待补 |
| MS03 | HSL / luminance qualifier | 基础 | HSL 和线性亮度范围；3D/RGB qualifier、clean black/white 待补 |
| MS04 | 蒙版合并、交集、减去、反选 | 已实现 | 递归 alpha 代数，严格深度验证 |
| MS05 | 外部 matte、节点 key、key mixer | 基础 | bitmap/trimap 冻结，cutout 节点改变 alpha 或输出 matte RGB，独立灰度 alpha 导出；专用 key mixer/节点 key 连接语义待补 |
| MS06 | Matte finesse | 基础 | grow 膨胀/腐蚀、高斯 feather、Closed-Form alpha、局部颜色恢复、despill；专用 denoise/clean black-white/in-out ratios 待补；见 cutout-workflow.md |
| MS07 | Magic Mask / 主体、天空、人物、物体 | 待实现 | 已有无模型 GrabCut 框选/标记分割，不能视为语义 Magic Mask；默认不下载大模型，真正语义后端仍待补 |
| MS08 | 深度、表面、背景、区域语义 | 待实现 | 深度估计、分割、confidence 与可检查产物 |
| MS09 | 圈选与坐标协议 | 基础 | Web 自由圈划/椭圆/加减选区；修订、源 hash、配方 hash、输出节点、全尺寸绑定；CLI/MCP 读取。跨几何阶段重映射与专业交互精修待补 |
| CL01 | 输入/工作/输出色彩空间 | 基础 | sRGB/P3/2020/ACEScg 输入矩阵和传递函数；可切换工作空间待补 |
| CL02 | ICC 输入和输出 | 基础 | moxcms RGB ICC 转工作空间、sRGB/P3/Rec.2020/ACEScg 输出 ICC、标准白点/PCS/TRC；CMYK/Gray、任意输出 profile 和 BPC 待补 |
| CL03 | OCIO / ACES / Resolve 色彩管理 | 基础 | 真实静态 OCIO 2.5.2、固定内置配置和 OCIOZ 快照、修订化上下文与显示链、线性原色映射、可组合/逆向/回退 Looks 节点；目录外依赖搬迁、所有算子任意域和完整 RCM 对照待补 |
| CL04 | CST、摄像机 Log、广色域映射 | 基础 | 真实 OCIO Log/广色域输入、数值 CST 与 display/view；原生 CDL/矩阵节点进入具名 Log/线性域后返回 canonical Frame；其他算子域、gamut mapping 和完整输出链待补 |
| CL05 | HDR PQ/HLG、显示、tone mapping | 基础 | OCIO ACES 2.0 HDR PQ view、cICP PNG、手动 PQ 亮度单位、独立 SDR preview；HLG 显示 OOTF、完整 HDR 元数据和校准 display 待补 |
| CL06 | Soft proof、监视器 ICC、校准、显示 LUT | 待实现 | 真实显示链和参考输出验证 |
| CL07 | Dolby Vision / HDR10+ / HDR Vivid | 待实现 | 图片适用部分先定义；涉及授权/SDK 的支持条件需调查 |
| CL08 | 输入/输出 data level 和 gamut checks | 基础 | 越界计数、chroma compression；legal/full levels、gamut overlays 待补 |
| CL09 | 1D/3D LUT | 基础 | Iridas/Resolve .cube、独立 DOMAIN/INPUT_RANGE、1D shaper + 3D、trilinear/tetrahedral；配方/项目修订的纯颜色 3D 烘焙、明确编码与采样误差报告；其他 LUT 格式、1D/shaper 烘焙、自动逆 LUT 和目标软件实机验收待补 |
| SC01 | 直方图、波形、RGB parade、矢量图 | 基础 | JSON density 数据；可视化、RGB/YUV/CIE、HDR scopes、参考标线待补 |
| SC02 | 像素采样、统计、裁剪诊断 | 基础 | 全图统计和分位；坐标/ROI 采样与 picker 待补 |
| FX01 | 模糊、锐化、纹理与中间调细节 | 基础 | Gaussian、unsharp、clarity；多尺度、边缘/深度/径向/镜头模糊待补 |
| FX02 | 空间降噪 | 基础 | bilateral；luma/chroma 分离、多尺度、neural NR 待补 |
| FX03 | 暗角、颗粒、Glow | 基础 | 可执行且固定随机种子；胶片模型、光学分布、halation 待补 |
| FX04 | 去雾、局部对比、纹理 | 基础 | clarity；dehaze、texture、多尺度细节待补 |
| FX05 | 胶片模拟 / 色彩质感 / 风格效果 | 待实现 | film look creator、胶片响应、halation/bloom 与颜色一致性 |
| FX06 | 美颜、Face refinement、皮肤处理 | 待实现 | 检测、分割、肤色/纹理与局部工具 |
| FX07 | Relight | 待实现 | 法线/深度/材质模型、虚拟灯光、固化结果 |
| FX08 | 修复、灰尘、划痕、红眼、物体移除 | 基础 | clone 笔刷；heal/inpaint、dust、red-eye、AI removal 待补 |
| FX09 | Super Scale / 超分辨率 | 待实现 | 模型、细节保持与输出验证 |
| FX10 | Resolve FX 中所有图片适用算子 | 待实现 | 逐一登记和实现，不能把一个通用 effects 插槽算作完成 |
| FX11 | OpenFX / DCTL / 插件隔离 | 待实现 | 完整宿主 ABI、参数、图像、渲染、版本和错误隔离 |
| GE01 | Crop、resize、rotate、flip、straighten | 基础 | crop、Lanczos3 抗混叠降采样/bilinear 放大、90° rotate；任意角度、flip UI 待补 |
| GE02 | 镜头畸变、透视、几何校正 | 基础 | k1/k2 反向采样；透视矩阵、自动校正、镜头库待补 |
| GE03 | 色差、去边、镜头渐晕 | 待实现 | 通道径向模型、defringe、镜头 profile |
| RW01 | 原生 RAW 解码 | 基础 | rawler 支持范围；工程 Bayer/单色 DNG 与 Canon Bayer、Fuji X-Trans、Leica 单色、Sony 四色四个真实样本；多机型/压缩模式仍待补 |
| RW02 | 黑/白电平、去马赛克、WB、相机矩阵 | 基础 | 修订化 v1：完整 sensor 黑白 repeat pattern、去马赛克前 WB/xy/Kelvin-duv、曝光、crop/orientation、解码器矩阵 illuminant 选择、sensor probes；所有 CFA、算法选择、DCP/profile、完整光谱/目标软件色温-tint 对等待补 |
| RW03 | 高光重建、RAW 降噪和传感器校正 | 待实现 | RAW 域处理，与 RGB 后处理区分 |
| RW04 | DNG/相机配置文件、双光源插值 | 待实现 | DCP、forward matrices、hue-sat maps、illuminant interpolation |
| PH01 | 照片目录、筛选、评分、关键字 | 待实现 | 索引与元数据数据库 |
| PH02 | 预设、virtual copy、同步调整、批量处理 | 基础 | JSON 配方、branch、batch；摄影目录关联和预设生命周期待补 |
| PH03 | EXIF/XMP/IPTC 与拍摄元数据保留 | 待实现 | 原图元数据仍在冻结源文件，导出未复制 |
| PH04 | HDR 合并、全景、focus stacking | 待实现 | 静态图片合成工作流 |
| IO01 | 普通图、高位深、浮点输出 | 基础 | PNG/JPEG/TIFF/EXR 等；专业 OIIO、完整格式/metadata 待补 |
| IO02 | 广色域和 HDR 输出标记 | 基础 | 广色域 RGB ICC、PQ PNG cICP、EXR colorInteropID/chromaticities/预乘 alpha、文件往返；完整 HDR 静态元数据、HLG 和专业格式覆盖待补 |
| AG01 | 原生 CLI、JSON Schema、JSONL、MCP | 已实现 | 共用引擎，真实进程 CLI/MCP 测试 |
| AG02 | 事务、版本冲突、分支、标签、回退 | 已实现 | OS 锁、原子提交、冲突结构化结果 |
| AG03 | 确定性缓存和项目资产 | 基础 | 内存预算、资源内容 hash、修改检测；tile/disk cache 和随机/模型/插件版本完整锁定待补 |
| AG04 | 常驻后台任务、进度、取消、恢复 | 基础 | JSONL/MCP 常驻、命名工具 background:true、会话内任务查询/幂等、节点进度和协作取消；重启恢复、checkpoint 和完整阶段取消覆盖待补 |
| AG05 | Web viewer（原生后端）与宿主插件 | 基础 | 本地网页自动打开；缩放平移/100%、原图/任意修订/划分/并排、圈划、版本命名/恢复、外部 agent 修改同步；sRGB ICC 预览。专业显示/HDR、GPU/tile、打包 Codex 插件桥接待补 |
| AG06 | GPU 和 CPU/GPU 一致性 | 待实现 | wgpu 后端、tile 执行、误差 corpus |

视频时间轴、音频、运动追踪、时间降噪、视频立体与视频交付只进入后续视频阶段；这不是静态图片能力的缩减。

## 验收标准

1. 功能必须有实际后端、严格参数 schema、稳定错误、可检查的预览或数值产物。
2. “基础”变为“已完成对等”前，需要覆盖公开控制、常见与边界输入、可复现项目，以及目标软件的代表场景对照。实现基础控制不等于实现目标软件的所有算法。
3. 颜色链必须明确 primaries、transfer、白点、亮度单位和显示/输出变换。精度测试使用标准向量与容差，OCIO 不允许 stub。
4. RAW 测试必须补真实 Bayer/X-Trans/monochrome/4-color、多相机与压缩样本；合成 DNG 只验证基础管线。
5. AI 输出需要真实模型、权重/许可/版本标识、seed（适用时）、confidence 或 mask、模型不可用错误，以及冻结产物后可离线复现。
6. 所有功能覆盖项完成之前，整体状态始终为未达到 Resolve/LR 全功能对等。

下一步优先顺序：OCIO 目录外依赖搬迁/节点域/完整 HDR 与专业显示 → RAW 参数/相机 corpus → Web 专业显示/圈选精修 → 后台任务/取消 → GPU → AI 蒙版/深度/重光 → 完整图片 FX/摄影目录。基础算子的专业化验证与这些阶段同步推进。
