# AI 模型能力读取

准备任务通过当前账号的官方 `GET https://api.deepseek.com/models` 查询所选模型的能力。
按完整模型 ID 精确匹配，读取 `context_window`、`max_output_tokens`、支持的思考强度、
输入输出类型，以及已知的协议行为字段。模型名、别名对应关系和输出上限不写死在代码中。
字段缺失表示未知；重复模型 ID、非法整数和相互矛盾的上下文／输出上限不能取得预算资格。

当前官方文档说明，`context_window` 包含输入和输出，`max_output_tokens` 是单次请求
`max_tokens` 的服务端允许上限。2026-10-25 查询文档时，示例模型公布的输出上限为
393,216，而定价页显示为 384K。运行时使用实际 API 返回的整数，不把文档示例当常量。
来源：[Models API](https://api-docs.deepseek.com/api/list-models/)、
[Models & Pricing](https://api-docs.deepseek.com/quick_start/pricing/)。
本轮使用已有系统凭证完成一次官方 GET，HTTP 200；所选模型实际返回上述两个整数，
并声明支持 `max` 思考强度。只保留了
[规范化能力记录](../analysis/kirikiri-autonomous-20261025/model-capabilities-read-20261025.json)，
没有保存原始响应或发起模型补全。

该请求不生成模型回答。它固定访问官方地址，不接受配置覆盖地址，不跟随重定向，
单次读取最多 1 MiB，且不自动重试。只有规范化后的白名单字段和安全状态可以持久化；
不返回原始响应、响应头、供应商自由格式错误信息或凭证。查询失败保留明确的
`unavailable` 状态，交由宿主决定既有回退预算，不能宣称上限已经核实。

宿主据此选择探索、编程和截断扩容预算，同时保留准备任务的最大思考强度。
`finish_reason=length` 仍属不完整模型输出，不能执行其中的程序或工具参数，也不能
将截断误记为引擎不支持。较大输出的传输读取上限按请求预算增长，始终限制在
8–32 MiB；该字节限制不替代服务端 token 上限或任务总时限。

离线验证覆盖精确 ID、新模型整数上限、缺失字段、重复和矛盾能力、错误与重定向、
响应体限制、凭证排除，以及较大输出的有界读取：

```powershell
python -m unittest discover -s crates/core/src/ai_summary -p test_model_capabilities.py
python -m unittest discover -s crates/core/src/ai_summary -p test_agent_transport.py
```
