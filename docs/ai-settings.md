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

AI analysis requires both resources beside the application executable:

- `tools/ai-agent/GalGear.AiAgent.exe` (or `ai-agent/GalGear.AiAgent.exe`).
- `Galgear-GalgameReverse-Skill/GalgameResourceExtract/SKILL.md` and its package resources.

If either is missing, the per-game AI option and generation buttons are disabled.
The backend checks again before starting work. An existing AI option can still be
turned off. Restart the app after installing resources to refresh availability.
Development builds use the same executable-directory prerequisite for the UI.
