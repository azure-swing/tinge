# 工具定义全量审计

## 当前架构：42 工具

2026-10-09：将九项 submit_* 合并到同名工具的 background:true 选项，默认同步。
idempotency_key 仅用于后台；job_status/job_cancel 和原有会话队列、幂等/取消回执保留。
不保留旧工具别名。后台模式会创建会话任务，因此相关工具的 readOnlyHint 统一为 false，
同步 stats/analyze 的读取行为不变。

analyze 只接收源图分析参数，项目配方用 stats；lut_bake 直接接受 project/revision/output/options。
完整配方由高级 edit_preview/apply/grade/validate 接收，CLI/JSONL 原生请求保持完整能力。
工具 schema 从原生类型派生并明确收窄，移除不可达定义，不在运行时接受未公开的旧参数。

| 全目录测量 | 架构调整前 | 调整后 |
| --- | ---: | ---: |
| 工具数 | 51 | 42 |
| UTF-8 字节 | 269,768 | 159,421 |
| 参考 tokens | 66,816 | 38,889 |
| analyze tokens | 5,669 | 1,469 |
| lut_bake tokens | 5,594 | 796 |

tokens 减少 41.80%。单项 adjust 因包含后台选项从 638 增至 748；基础三工具从 1,299 增至 1,519，
五工具从 2,916 增至 3,246。此次优化针对整套重复接口；原来的后台工具不再另行加载。

42 项完整 schema 经 Draft 2020-12 检查；后台开关/key 条件单独验证。按新分析/LUT 边界归一化、
排除新增执行选项后，公共参数 7,435 次合法与 58,090 次非法样例校验一致。
Rust 回归逐一比较九项同步请求与后台内层请求，检查旧入口和不支持参数拒绝；真实 MCP 进程
覆盖后台编辑幂等、源图分析、项目 LUT，以及 OCIO log CDL 项目烘焙。完整 workspace、Clippy、
正式 release、Agent 和查看器验收通过。以上不代表真实宿主选择验收。

测量及样例记录：`artifacts/architecture-context-measurement.json`、`artifacts/architecture-contract-validation.json`。

## 先前结构审计：51 工具

2026-10-09，基于正式程序真实 `tools/list` 的全部 51 个工具，包含同步和后台入口。上一轮只精简工具级描述，本轮展开嵌套 schema。

## 覆盖范围

| 层次 | 检查内容 |
| --- | --- |
| 工具目录 | 全部名称、用途、同步/后台区别、安全标注与完整独立契约 |
| 参数 | properties、required、默认值、类型、范围、枚举、数组长度与未知字段拒绝 |
| 配方 | Node、31 类 Operation、编辑联合、输入/输出和蒙版引用 |
| 蒙版 | 10 类 Mask、递归组合/反转、几何参数、导入 bitmap、笔触 |
| 摄影显影 | RAW 白平衡、sensor levels、裁切、方向、曝光和校准参数 |
| 色彩管理 | OCIO 配置、输入/显示链、context、grade、ColorSpace、LUT 编码 |
| 交换与抠像 | CDL 元数据、导入/导出选择、LUT 烘焙、trimap、GrabCut、matte refinement |
| 引用 | 每个工具内的所有传递 `$ref` 与共享定义 |

## 已精简

- 联合分支共同的必填字段移至联合外层；各分支的额外必填字段及未知字段拒绝仍各自保留。
- 重复字符串数组共享本工具内的 StringList，字段默认值及说明保持原位。
- 删除嵌套类型中的兼容历史、配置版本设计理由；缩短哈希检查实现、算法出处和重复色彩链说明，保留单位及调用语义。
- 变换只遍历 schema 位置，不改写 default/examples 中的用户数据。

必填、范围、数组长度、类型区别和未知字段检查都有实际用途，保留。九个高级工具占旧目录约 77% 的体积，主要因为各自需要携带完整配方；本次未将字段说明挪到隐藏的外部契约，也未删除工具。

## 结果

| 紧凑 JSON 测量 | 修改前 | 修改后 |
| --- | ---: | ---: |
| 全部工具 UTF-8 字节 | 277,744 | 269,768 |
| 全部工具参考 tokens | 68,591 | 66,816 |
| edit_preview 参考 tokens | 6,121 | 5,939 |
| 五工具基础流程参考 tokens | 2,994 | 2,916 |

五工具为 init/project_info/preview/adjust/render。基础 adjust 仍为 638，三工具 project_info/adjust/render 仍为 1,299。分词使用 o200k_base，实际上下文由客户端加载决定。

全部 51 项新旧 schema 通过 Draft 2020-12 检查，13,680 次合法输入及 103,821 次非法输入校验结果一致。样例涵盖联合分支、递归蒙版、缺字段、额外字段、错误类型和数组长度；这是样例对比，不是穷举证明或真实宿主验收。记录为 `artifacts/full-schema-audit-validation.json`。

Rustfmt、Clippy、完整 workspace 回归及正式 release 构建通过；额外回归覆盖公共 required、共享数组的默认值/说明和用户数据保护。真实 MCP/Agent 进程验收与查看器脚本检查通过。
