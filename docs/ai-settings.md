# AI settings

Settings → AI contains separate OpenAI, DeepSeek, Qwen, and Kimi provider cards.
Enter an API Token, fetch the available models, choose a model in the searchable
dropdown, and save the configuration. A saved Token can be reused by leaving its input blank.
Changing the Token invalidates the displayed model list until it is refreshed.
Opening AI settings automatically fetches models for configured providers.

The logo pills at the top select the application's active AI provider. Selection
is saved as `ai_provider` and survives reopening. Each provider keeps its own
Token and model configuration; new AI analysis uses only the selected provider.
An unconfigured selection prompts for configuration instead of falling back to
another provider. Existing settings default to DeepSeek. Cached game recaps still
reuse their saved result when the save is unchanged.

Model IDs are discovered from authenticated provider APIs, without a fixed list:

- OpenAI: `GET https://api.openai.com/v1/models`
  ([API reference](https://developers.openai.com/api/reference/resources/models/methods/list)).
- DeepSeek: `GET https://api.deepseek.com/models`
  ([API reference](https://api-docs.deepseek.com/api/list-models/)).
- Qwen (Alibaba Cloud Model Studio, China/Beijing):
  `GET https://dashscope.aliyuncs.com/compatible-mode/v1/models`
  ([API reference](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope)).
- Kimi (Moonshot, China): `GET https://api.moonshot.cn/v1/models`
  ([API reference](https://platform.kimi.com/docs/api/list-models)).

Qwen uses a Beijing-region DashScope API key; Kimi uses a Moonshot Open Platform
API key. These are API credentials, separate from chat subscriptions and coding
plans. Both use their OpenAI-compatible Chat Completions API for AI analysis.
Choose a text chat model that supports non-streaming output and JSON mode.
Qwen requests disable thinking for this workflow; Kimi K2.5/K2.6 requests disable
hybrid thinking, while other Kimi models keep their default parameters.

The native backend sends Bearer authentication, applies a 30-second timeout, and
disables redirects. API error bodies are never displayed or logged. The picker
lists all model IDs returned for the Token; it does not infer their capabilities.
Saving revalidates the selected model with the provider before replacing a key.

Tokens are stored in Windows Credential Manager under provider- and
configuration-specific targets. Only selected model IDs are stored in
`settings.json` (`ai_models`). Clear removes both the provider's Token and model
selection. Existing settings files default to an empty AI configuration.

Per-game setup and recap caching are described in [AI前情提要](ai-summary.md).

## AI 查找存档

AI 搜索使用本地候选目录与存档文件修改时间；候选独立于有大小上限的目录列表，
目录清单被截断时也不会丢掉已经发现的存档证据。名称匹配兼容 `FHD版` / `FHD`
及 `HD版` / `HD`，同时保留版本标识，不把 FHD 版降为原版名称匹配。

返回路径必须位于允许搜索的范围内，且直接包含可读的存档候选文件。支持路径
开头的 `%APPDATA%`、`%LOCALAPPDATA%`、`%USERPROFILE%` 及 `$(NAME)` 写法；
不会将游戏包内字面名为 `$(APPDATA)` 的目录视为当前用户存档。目录存在、非空
或最近复制过，都不足以验证存档位置。候选仍是文件特征判断，不代表所有引擎
都已经验证实际游戏写入；无法确定时应手动指定。

自动选择按直属存档文件的修改时间比较，不使用目录、说明文件或日志时间。
旧 AI 自动选择会重新检查，排除占位目录和没有存档文件的父目录；用户手动
指定的位置保持优先，不自动覆盖。所有发现与校验均只读，不修改游戏存档。

AI analysis requires both resources beside the application executable:

- `tools/ai-agent/GalGear.AiAgent.exe` (or `ai-agent/GalGear.AiAgent.exe`).
- `Galgear-GalgameReverse-Skill/GalgameResourceExtract/SKILL.md` and its package resources.

If either is missing, the per-game AI option and generation buttons are disabled.
The backend checks again before starting work. An existing AI option can still be
turned off. Settings → Updates → **Skill** displays the local commit. Each launch
checks the remote main commit in the background and marks a newer or missing package
as **Update available**. Only clicking **Update** downloads and installs the package,
then refreshes availability and clears its pending-update count. Failed
updates retain the installed resources; running AI jobs block replacement until
they finish.
Development builds use the same executable-directory prerequisite for the UI.
