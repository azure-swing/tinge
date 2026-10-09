# 本地 Web 看图与圈选

0.2.1 新增“定稿并清理”；普通保存版本仍仅命名。定稿后按登记清单回收临时图片，源图、全部历史及最终导出保留；查看器预览直接在内存编码，不再写临时 PNG。详见 [临时文件工作流](storage-workflow.md)。

用户界面只负责看图、圈划、原图/版本对比和保存版本，调色仍通过原生 CLI、JSONL 或 MCP 由 agent 完成。HTML/CSS/少量 JavaScript 嵌入 Rust 可执行文件，没有 Node、前端构建工具、模型权重或云服务运行依赖。

## 启动

```powershell
.\target\release\tinge.exe view portrait.tinge
# 默认自动打开系统浏览器；只监听 127.0.0.1，端口 0 自动选择空闲端口。

.\target\release\tinge.exe view portrait.tinge --no-open
# stdout 的 viewer_ready JSON 包含 URL，可在 Codex 侧栏浏览器打开。

.\target\release\tinge.exe view portrait.tinge --port 6340
```

进程持续运行，Ctrl+C 停止。启动不修改项目。用户须先用 `init` 创建项目；headless 调色命令继续保持原来的行为。默认浏览器打开失败时 stderr 返回事件与 URL，服务继续可用。原生程序不会自行调用 Codex 的宿主 API；在 Codex 内直接打开该本地 URL 即可，打包插件桥接仍待完成。

预览进程必须在看图期间保持运行。若启动它的终端或任务会话结束，网页可能仍显示旧图片，但不能切换版本。API 请求有 15 秒超时，断线会显示明确提示、解除等待状态并暂停保存操作；同一服务恢复连接后重新请求所选版本，同时保留未提交圈划。重启服务会产生新 URL，应打开新地址。需要长期运行时，可在 Windows 用 `Start-Process -WindowStyle Hidden` 后台启动并将 stdout/stderr 重定向到日志文件。

页面每 1.2 秒读取项目头与版本摘要，agent 提交新修订后后台生成新预览，显示实际节点进度。浏览历史版本时保留该版本。存在未提交圈划时，新修订到达会保留原画面和版本，避免选区附着到错误图像；用户保存圈划后可切回“跟随最新”。切换版本会在当前网页会话中暂存圈划草稿，重新打开网页不会保留未保存草稿。

连续切换采用每个标签页独立的请求标识，并合并 120 毫秒内的连续选择：同一页的过时任务会取消，待处理队列清除这些任务后只保留最新请求。普通颜色节点按像素块检查取消，蒙版并行求值也有取消检查；空间算子、RAW 解码和 OCIO 仍使用已有阶段检查。相同版本、对比版本和预览尺寸的已完成结果在缓存保留期间直接复用；取消记录优先清理，已完成图像按最近使用顺序保留。其他标签页的活动任务不会被取消；独立活动请求队列仍限制为 8 项。

大型 RAW 的解码缓存优先保留，无法与其共存的中间节点不入缓存，合计仍受 512 MiB 引擎缓存预算限制。原图对比复用同一显影源帧，不重复解码。PNG 预览另外受 128 MiB 预算限制。0.2 在队列闲置 60 秒后释放全分辨率像素缓存，保留有界 PNG 结果供快速切换。首次查看未缓存版本仍需全尺寸计算；正式导出和预览的像素处理保持一致。MCP 的 viewer_open/status/close 复用并管理本会话启动的服务，详见 [Agent 工作流](agent-workflow.md)。

## 查看与对比

- 适应窗口、拖动平移、滚轮缩放、100% 视图；100% 加载原尺寸 PNG，一个图片像素对应一个设备像素。快捷键 F 适应、1 为 100%、B 切原图。
- 原图是所看修订使用相同 RAW 显影、输入色彩和显示变换后旁路节点图的结果，不是直接播放未管理的源文件。
- 对比菜单可选原图或任意修订；划分对比与并排均支持。版本对比各自使用该修订保存的显示链。两张图片尺寸不同会禁用划分对比并改为并排，不强行拉伸对齐。
- 默认预览长边 1600；100% 请求上限 16384，单张编码 PNG 上限 64 MiB。超限图片明确提示，不声称是完整原尺寸预览。后台队列 8、连接线程 8，保留的 PNG 总预算 128 MiB；这是查看器临时缓存，不是持久后台作业/恢复系统。
- 原始图先按全尺寸执行配方再缩小，半径和裁切与 CLI 一致。缩图在场景线性光下使用随缩放比例扩展范围的 Lanczos3 抗混叠滤波，避免密集建筑条纹变成粗大的彩色摩尔纹；原图和效果图共用这条缩图路径，原尺寸预览不滤波。大型项目首帧仍受原生 CPU 渲染耗时影响；GPU/tile 预览、持久任务仍待实现。

## 保存版本

“保存版本”给所看修订增加名字，历史下拉和对比菜单均可使用。保存历史修订的名字不替换当前调色；项目创建元数据提交以保证并发版本检查。恢复将选中配方/色彩链/RAW 设置复制为新修订，旧历史和批注保留。

所有保存/恢复/圈选请求带 `expect_revision`。发生并发冲突显示错误并保留待处理数据，不自动改期望值重试，也不覆盖 agent 的提交。

## 把圈划交给 agent

支持拖动自由圈划、椭圆，以及增加/减去区域。选区采用输出图归一化坐标 [0,1]，根据实际图片矩形换算，缩放、平移、侧栏开关不改变坐标。移动工具隐藏选区色块以查看图片颜色。网页一次圈选最多 12 个区域；羽化、精修和如何调整颜色交由 agent。

“保存圈选”原子追加到 `<project>.selections.json`，不添加调色节点、不改变项目 revision。字段包括唯一 ID、revision、source_hash、recipe_hash、output_node、全尺寸 width/height、timestamp、Mask 和可选 note。保存时再次检查项目头和基准。独立记录最多 1024 条/16 MiB；网页摘要显示最近 64 条，CLI 可读全部。项目搬迁时同时携带此文件和 `.assets`。

```powershell
.\target\release\tinge.exe selections portrait.tinge
```

CLI `run` / JSONL / MCP 同样支持：

```json
{"command":"selections","project":"portrait.tinge"}
{"command":"selection_save","project":"portrait.tinge","expect_revision":4,"revision":4,"mask":{"type":"ellipse","center":[0.5,0.4],"radius":[0.2,0.25],"rotation":0,"feather":0},"note":"天空冷一点"}
```

`selection_save` 执行原生渲染以获取真实输出尺寸，记录基准后在项目锁下检查期望版本并保存。只接受几何蒙版，拒绝 bitmap 文件及颜色 qualifier；网页无需读取任意本地文件。CLI/MCP 完整字段 schema 随 Request 类型生成。

agent 应读取批注和当前项目，核对 source_hash、recipe_hash 与输出几何。如果基准匹配，可以把记录的 mask 放入 `set_mask`，并将新局部调色节点接在 `output_node` 后面；蒙版在这个节点的输入图上求值。基准过期、裁切/旋转/输出连接变化时，应明确重新映射或请用户重圈，不能直接把坐标套到另一阶段的图片。批注不会自动触发 agent 会话或向其他任务发消息。

## 浏览器色彩

预览使用与 CLI 相同的 Rust 输出链：显式 sRGB SDR view → 8 位 PNG → sRGB ICC 标签。网页直接显示 `<img>`，不再 gamma、不用 CSS filter 或 canvas 重新调色。正式 PNG/TIFF/EXR 导出继续由原生色彩管线决定，预览的 sRGB/8 位不会替换项目工作数据或正式导出的格式。

这保证浏览器获得明确编码的预览文件，不保证物理屏幕已校准、HDR 亮度正确或与专业视频监视器一致。网页通过 `color-gamut`/`dynamic-range` 读取的只是浏览器报告的能力，展示在色彩状态的说明中，不作为校准证明，也不据此自动改变预览色域。

Codex 官方 [Browser 说明](https://learn.chatgpt.com/docs/browser)介绍内置网页预览/交互，所查文档没有承诺专业色彩准确性。MDN 对 [color-gamut](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@media/color-gamut) 的定义是浏览器与输出设备所支持的近似色域。专业显示校准、软打样和 HDR 查看器仍在完整目标的待实现项内。

## 本地服务边界与验收

绑定单一项目和 IPv4 loopback，启动时生成 OS 随机 192-bit URL token。检查 Host、Origin 和 POST 自定义 token；只提供内嵌资源、该项目摘要、受限预览、批注和版本动作，没有通用 RPC、文件路径浏览或 LAN 监听。HTTP 请求体上限 1 MiB、头上限 16 KiB、读超时 5 秒；响应禁止缓存和外部脚本资源。

`scripts/viewer-acceptance.py` 使用实际 release 进程和 HTTP 验收预览与 CLI 字节一致、ICC 标签、P3 项目的 sRGB preview view、任意修订对比、裁切后选区基准、旧修订命名/恢复、独立 CLI 更新、MCP 读取，以及并发冲突和访问边界。报告为 `artifacts/viewer-acceptance.json`。实际 Codex 浏览器另验收圈选手势、保存对话框、缩放/比较和窄侧栏/宽屏布局；这不等于物理屏幕测色验收。

前端请求合并、断连/超时、重连后历史版本和未提交选区保护，由统一入口 `node --test scripts/viewer-client.test.cjs` 验证。服务端快速切换、取消过时任务、跨标签页缓存复用和队列容量由 `cargo test -p tinge-cli --bin tinge web::tests --locked` 覆盖，无需依赖运行中的查看器日志或固定项目版本。
