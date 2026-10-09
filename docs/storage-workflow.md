# 定稿与临时文件（0.2.1）

在查看器选好版本后点击“定稿并清理”，先显示可回收文件数和大小，确认后将登记过、内容未改变的临时图片移到 Windows 系统回收站。普通“保存版本”继续用于命名，不触发回收。用户未选择最终版本时不自动清理。

保留源图、`.tinge` 全部配方历史、不可变项目资产、圈选记录、正式导出，以及最终修订的临时导出。预览和对比图可重新生成；其他修订的临时导出可按历史配方重建。定稿写入独立侧车，不新建调色修订、不修改配方。

## 自动登记和存储

preview/compare 输出自动登记为临时预览。render 默认是正式导出；中间验收导出需显式 `temporary:true`（CLI `render --temporary`）。同一输出路径更新登记，不堆积重复记录。登记失败会在成功输出回执中给出 workfile_warning，不把正式导出伪装成失败。

MCP/JSONL 的 preview/compare 可省略 output，使用 `<project>.work/` 内按修订/配方/尺寸命名的可复用 PNG；显式路径仍兼容。edit_preview 也可省略 output。文件必须在项目目录内才能自动回收，目录外输出保留并提示。不会扫描用户目录、按名字或体积猜测临时文件；旧未登记文件需明确登记用途。

登记保存到 `<project>.workfiles.json`，包含相对路径、hash、大小、修订、用途和最终版本。最多 4096 项/4 MiB，使用 OS 锁和原子写入。移动项目时可一起移动该侧车及 `.work/`。侧车丢失不影响配方/资产，只停止识别旧临时文件。

## Agent 接口

```json
{"command":"preview","project":"portrait.tinge","include_analysis":false}
{"command":"render","project":"portrait.tinge","revision":2,"output":"draft-r2.png","temporary":true}
{"command":"cleanup_plan","project":"portrait.tinge","revision":5}
{"command":"finalize","project":"portrait.tinge","expect_revision":5,"revision":5}
```

CLI/JSONL 的 finalize 可通过 job_submit 执行；MCP 使用独立的 tinge_finalize 或 tinge_submit_finalize。要求用户已明确选定最终修订，Agent 不能根据最新 head 猜测。并发修改产生 revision_conflict 时不清理；定稿后单个回收失败不撤销定稿，回执列出 failed/skipped/retained，可重试。旧预览 URI 可能因清理而失效，重新 preview 即可。

明确属于项目的旧临时输出可以迁移登记，role 是 preview（预览/对比）、draft（临时导出）、export（正式保留）：

```json
{"command":"workfile_register","project":"portrait.tinge","file":"old-preview.png","revision":2,"role":"preview"}
```

## 保护与回收站

回收前重新检查 hash、归属和正规文件路径；已修改文件跳过。不递归搬移目录，不处理符号链接/junction、源图同内容副本或未登记文件。项目锁防止定稿与配方提交穿插。当前支持 Windows 本地固定磁盘的系统回收站，网络/其他磁盘和其他平台保留文件并报告不支持。使用原生 IFileOperation、FOFX_RECYCLEONDELETE 与失败检查，没有永久删除回退，也不启动 PowerShell 作为生产依赖。[系统接口说明](https://learn.microsoft.com/zh-cn/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags)。

回收站仍占用磁盘，插件不自动清空它。recycled_bytes 表示移出工作目录的大小，不等于磁盘可用空间增加同样大小。

## 无损减少体积

查看器直接在内存编码并提供带 ICC 的 sRGB PNG，不创建临时目录/中间 PNG；历史渲染及有界 PNG 内存缓存继续工作。不透明 PNG 写 RGB，透明图像保留 RGBA；位深、像素量化、ICC/cICP 和抗混叠不变，不降低 JPEG 质量。新写入项目 JSON 省去缩进，保留所有历史字段。

已有正式图片不自动重编码，源 RAW/依赖资产不压缩替换。节省来自避免重复落盘、回收可重建中间成果和无损编码，不降分辨率、不删除历史。

回归覆盖源图/历史保护、修改和未登记文件保留、失败与重试、稳定缓存名、8/16 位 PNG 透明/不透明样本与内存/磁盘编码逐字节一致。scripts/storage-acceptance.py 使用隔离文件验证实际回收、原样恢复及清理后历史查看；验收脚本依赖 Python/PowerShell，生产插件无需它们。
