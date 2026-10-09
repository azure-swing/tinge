# Tinge

面向 agent 的原生 CLI 图片调色软件，以 Codex 插件作为主要交互入口。

完整功能目标：覆盖 DaVinci Resolve Studio 中适用于静态图片的调色、蒙版与图像处理能力，补足 Lightroom 的 RAW 显影和摄影工作流。界面集中于图片预览、圈选、原图对比和版本保存。视频专属能力留到视频阶段。

## 语言约定

用户已确定：**尽可能使用 Rust 编写。**

- CLI、项目状态、版本控制、节点图、任务调度、缓存、图像分析、自研 CPU 算子和 MCP 服务使用 Rust。
- 本地查看界面优先使用 Rust；可嵌入网页的查看界面优先复用 Rust 代码并编译为 WebAssembly。
- 专业图像库允许通过 Rust 绑定接入。跨语言代码集中在独立适配模块，项目业务逻辑保持在 Rust 中。
- GPU 算子优先评估 Rust 编写的可行性。允许必要的 shader 语言代码，但必须说明所需后端、编译链和兼容性原因。
- 浏览器界面允许少量 HTML 与必要的 JavaScript 桥接，避免另建一套完整的 TypeScript 业务层。
- 优先采用现有能力；语言约定不要求重写成熟 RAW 解码器或专业色彩管理库。

## 模块划分

以下是计划中的 Cargo workspace 模块，尚未创建实现。

| 模块 | 责任 |
| --- | --- |
| `tinge-core` | 节点图、算子描述、参数验证、CPU 参考实现 |
| `tinge-project` | 原图引用、蒙版、非破坏性配方、分支与快照 |
| `tinge-engine` | 执行计划、任务调度、缓存、进度与取消 |
| `tinge-gpu` | GPU 资源、计算算子、节点执行和预览输出 |
| `tinge-io` | 图片格式、RAW、元数据及原生库适配 |
| `tinge-color` | 工作色彩空间、显示与输出变换、OCIO 适配 |
| `tinge-cli` | 原生命令行及结构化输入输出 |
| `tinge-mcp` | Codex 可调用工具、预览资源与结构化结果 |
| `tinge-viewer` | 看图、圈选、原图对比、版本查看与保存 |

CLI、MCP 和查看界面调用同一套引擎接口，并使用一致的项目状态。窗口关闭后，引擎仍能无头处理和导出。

## 候选技术

这些是待验证的选择，不代表已完成依赖锁定或 Windows 构建测试。

| 用途 | 候选 | 接入边界 |
| --- | --- | --- |
| MCP | 官方 Rust SDK `rmcp` | Rust 服务，可用 STDIO 接入 |
| GPU | `wgpu` | Rust 管理 GPU；算子语言另行验证 |
| Rust GPU 算子 | `rust-gpu` | 先验证编译链、目标平台和后端支持 |
| 查看界面 | `egui` / `eframe` | 可面向本地和 WebAssembly；专业显示路径单独验证 |
| RAW | LibRaw Rust 绑定；评估 `rawler` | 验证目标相机；Rust 绑定仍可能依赖 C/C++ |
| 专业图片读写 | OpenImageIO 的 `oiio` 绑定 | 独立适配，验证格式、位深、元数据与构建 |
| 色彩管理 | OpenColorIO 的 `ocio-rs` 或自有薄桥接 | 必须使用真实 OCIO，禁止将 stub 结果当成正式颜色处理 |
| 第三方特效 | OpenFX Rust 绑定及自研宿主 | 绑定不包含完整宿主的加载、参数、图像和渲染服务 |

## 运行与状态协议

- 本地引擎按需保持常驻，复用解码结果、GPU 资源与节点缓存。
- MCP 和 CLI 传递操作参数、资源标识与版本标识；避免反复复制整张原图。
- 修改带预期版本号，并返回新的版本号，避免基于过期状态写入。
- 多项修改以事务提交；失败后保留此前的有效状态。
- 圈选包含图片标识、版本标识、坐标空间和选区标识。
- 预览提供模型可读取的图片，并附带版本、显示变换和必要统计。
- 长任务返回任务标识，支持进度查询、取消与恢复。
- 项目保存引擎、模型、插件和处理配方版本，以及随机种子与需要固化的 AI 结果。
- 项目状态独立于聊天历史，重开项目可恢复处理结果。

## 优先验证

1. Windows 上构建和打包 Rust 程序及必要的原生依赖。
2. RAW 解码、浮点图片读取和高位深导出，保留必要元数据。
3. 使用真实 OCIO 做颜色变换，建立 CPU 参考结果和 GPU 容差验证。
4. 验证 OCIO 输出与 GPU 后端的接入方式，保留 LUT、纹理及动态参数语义。
5. 原生预览、用户圈选、局部调整、版本保存、关闭与重开形成完整闭环。
6. MCP 工具在 Codex 中可发现、可调用，并能让模型读取预览结果。
7. 验证 Rust / WASM 查看界面与目标 Codex 宿主的桥接；保留原生查看窗口。

## 资料

- [MCP 官方 Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [wgpu](https://wgpu.rs/doc/wgpu/index.html)
- [Rust-GPU](https://github.com/Rust-GPU/rust-gpu)
- [eframe](https://github.com/emilk/egui/blob/main/crates/eframe/README.md)
- [LibRaw Rust 封装 rawlib](https://docs.rs/rawlib/latest/rawlib/)
- [RAW 解码 rawler](https://docs.rs/rawler/latest/rawler/)
- [OpenImageIO Rust 封装 oiio](https://docs.rs/oiio/latest/oiio/)
- [OpenColorIO Rust 封装 ocio-rs](https://docs.rs/ocio-rs/latest/ocio_rs/)
- [OpenFX Rust 绑定](https://docs.rs/crate/openfx/latest)
- [OpenAI 插件架构](https://developers.openai.com/plugins/concepts/plugins)
- [OpenAI 插件打包](https://developers.openai.com/plugins/build/plugins)
