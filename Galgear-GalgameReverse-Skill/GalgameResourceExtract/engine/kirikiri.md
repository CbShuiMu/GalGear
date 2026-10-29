# KiriKiri / KiriKiri Z：XP3 与 KAG / TJS

本页覆盖通用 XP3 容器与所引 KiriKiri Z 文本流；KAG 是脚本层，XP3 中也可以存放其他脚本和媒体。

## 解包与封包工具

| 工具与来源 | 解包或读取用途 | 封包或重建用途 | 适用范围与限制 |
| --- | --- | --- | --- |
| [GARbro CLI](../commands/garbro.md)，随附；[XP3 实现][garbro-xp3] | 识别、列出并解包支持的 XP3；读取器也能定位部分附加在 EXE 后的 XP3 | 创建 XP3 v1 / v2，可配置索引压缩、成员压缩及保留目录 | 写入能力不代表支持所有加密插件。CLI 的复杂 `Scheme` 不能直接赋值；需特殊方案且无法表达时停止，不用普通 XP3 冒充兼容产物 |
| KiriKiri Z 引擎[文本流实现][text]，外部源码参考 | 核对 BOM、默认编码、简易变换和压缩文本流的读取顺序 | 描述对应文本流写入方式；不是本技能的独立脚本 CLI | 不提供 TJS 字节码反编译或任意游戏专用过滤器；不需要运行游戏来阅读源码 |

归档命令使用 `i` → `l` → `x`；需要封包时先查 `options XP3`，保留目录设置为 `RetainDirs=true`。完整参数见工具文档。

## 格式特征与编码方式

| 层级 | 通用识别特征 | 编码或变换方式与判定依据 |
| --- | --- | --- |
| XP3 容器 | 标记为 `58 50 33 0D 0A 20 0A 1A 8B 67 01`；索引含 `File`、`info`、`segm` 等块 | 多字节整数按小端；标准 `info` 的名称为带长度的 UTF-16LE，不是 CP932；见[官方归档读取器][archive] |
| 索引与成员分段 | 索引压缩标志与成员 `segm` 压缩标志分别存在；成员可能分为多段 | 通用实现支持原样或 zlib 压缩。逐段合并并核对解压长度；解压与字符解码是两步，不能把整个 XP3 当文本 |
| KAG / TJS 明文 | 常见 `.ks`、`.tjs`，先检查实际内容和文件头 | 所引 Z 文本流识别 UTF-16LE BOM `FF FE`、UTF-8 BOM `EF BB BF`；无 BOM 时遵循引擎构建及 `-readencoding` 设置，Z 默认 UTF-8，兼容模式可用 Shift_JIS。不能把所有 KiriKiri 版本统一转成某一种编码 |
| 引擎文本流包装 | `FE FE` 后带模式字节，再带原始 UTF-16LE BOM | 模式 0 / 1 为简易字符变换，模式 2 为带长度字段的 zlib 压缩；这是文本流包装，不是 XP3 加密。按[实现][text]还原，再按 UTF-16LE 读取 |
| 归档过滤器与媒体 | XP3 读取器可调用外部 extraction filter；成员也可为 TLG、PNG、Ogg 等 | 过滤器不由 XP3 标记唯一决定。媒体按自身文件头和同版本解码器判断；文件名编码与音频编码无关 |

### AI 宿主中的公共研究接口

以下是 GalGear AI 宿主的可选能力，详见 `galgear_codecs.capabilities()`；不要求独立 CLI 提供这些接口。

- **公共文本流。** 观测到标准 `FF FE` 明文或 `FE FE` 模式 0 / 1 / 2 封装时，可用 `galgear_codecs.decode("tjs_text_stream", data)` 或 `galgear_codecs.view(data,[{"op":"tjs_text_stream"}])` 重放[固定读取实现][text]。输出为去掉外层 BOM 的 UTF-16LE 正文字节；不接收外部 key、mode 或布局参数。容器范围仍须明确切片；此解码不处理未知 XP3 成员过滤器。
- **逐输入原生声明。** 当前完整提取字节能与归档成员独立绑定时，parser 的 `metadata.get("native_archive")` 可含 `format="xp3"`、`adler32`、`size`、`entry_id`、`index_sha256` 等；`scope="native_declaration_only"` 不是过滤器或节点验收。批量解析使用各文件自己的声明，不复制样本 checksum / 长度；字段缺失时保留未知，不自造原生证据。
- **可选数学假说。** 若当前字节观察支持研究单字节 mask，由模型自行决定 key 和是否探索 XOR 前缀；这不是 KiriKiri 加密通则。`galgear_codecs.adler32_xor_prefix_lengths(data,key,target_checksum,max_candidates=64)` 扫描全部前缀长度（含零），要求显式非零 byte key 和 u32 目标。输入必须是完整原始成员字节，有声明时用 `native_archive.size` 交叉核对；不能把 `inspect_data` 默认局部窗口与全成员 `adler32` 比较。可用本输入的原生 `adler32` 作约束；返回完整 `match_count`、有界 `lengths`、`truncated`、`checked_prefixes`。原理是 [RFC 1950 的 Adler-32][adler32]，输入至多 32 MiB；不会搜索 key、自动选择 decoder 或证明真实 cipher。`match_count == 0` 仅排除给定 key 的此类前缀假说，应如实保留限制。

### PSB 内部加密与普通 MDF

`galgear_codecs.decode("psb_decrypt", data, key=<本轮显式 u32>)` 及标准 view
`{op:"psb_decrypt",key:<u32>}` 使用随附 FreeMote v4.9.0 的[原生完整 binary API][psb-crypto]。
只开放受限 Auto：完整明文先拒绝，再核对原生加密检测、标准 v2/v3 边界；输出须保留
全文件长度、完整 typed root 和资源／字段引用位置。不接收模式、偏移、路径、自定义
CLR 或游戏预置 key，不搜索 key，不手工修 flag，不经 JSON 重建。检测不能证明真实
cipher；未知布局、错 key 及无效完整结构都失败。

普通 MDF 可用 `decode("mdf", data)` 或 `{op:"mdf"}`：固定 `mdf`+NUL、原生 u32le
输出大小和完整 zlib，核对精确长度、EOF、校验与 32 MiB 上限。未开放 MT19937 加密壳、
seed、MFL、MZS 或任意嵌入位置。typed PSB 内层使用固定描述：
`{syntax:"psb",view:[],frame?:{codec:"mdf"},crypto?:{codec:"psb_decrypt",key:<u32>}}`。

XP3 的完整原始／独立过滤后成员先验收原生大小与 checksum。其 adlr 可能覆盖内部密文；
不能把不匹配的 outer filter 移入 `document.crypto` 绕过校验，也不把内部解密输出再
强迫匹配外层密文校验。名称和消息都要完整 typed 字段／引用；不允许 global/evidence
view 切池或补几字节来替代 PSB 根。同一 source 的消息与名字 ID 必须来自同一个完整
解码 binary（按 SHA 核对），描述不同但完整字节相同不误拒。

PSB 消息即使省略 native 声明，也由宿主标记为 `message_reference_only`；这不提供
current/choice/resume 角色。`typed_record`、历史 snapshot 和恰巧相等的 BMP／非当前
整数均不能升级当前节点。独立已审计的当前 selector 与标准 KAG resume 仍分别核验；
私有框架的状态语义不在这项字节能力内。

数学匹配仍可能碰撞。由模型选择并声明 `member_filter` 后，宿主仍须以实际归档元数据核验完整成员大小与 checksum；PSB 还须完整 typed root、实际消息／姓名引用及存档节点角色验证。可读头、部分文本、checksum 候选均不能替代这些验收，也不能把历史／恢复位置改称当前对白。

## 注意事项

- 原样资源根目录保留解包后成员字节及相对路径；文本包装解码、TLG 转图和脚本解析产物另存。记录工具是否额外执行成员过滤，避免把工作副本误作基线。
- `.ks` 不保证是 KAG 明文，`.tjs` 也可能是字节码。发现 PSB 或编译场景时，按文件头分流到 [Emote / PSB](emote.md) 等对应资源页；XP3 成功解包不能证明脚本可编辑。
- 选择解析样本前，结合全部归档条目、加载配置和实际保存状态中的资源引用，区分系统初始化、菜单、开发测试与剧情资源。先检查一个真实剧情样本中的本地化对白和姓名引用，再扩大到完整章节；小文件或可读文件不自动具有代表性。分类结论须绑定实际输入与观察，不能据此删除未检查的章节或宣称没有人物姓名。
- PSB 字符串池可同时包含命令、资源名、摘要、姓名及多种语言文本。完整解码字符串池只证明字符串存在；对白记录应沿对象树中的实际消息记录、文本引用和姓名引用建立。保留未使用及重复字符串的原始索引，同一字符串的多次引用仍是不同消息位置。不得按字符串池顺序、文件名或文本是否像台词筛选出所谓完整对白。
- 封包后用新目录重新解包，检查完整路径集合、数量及哈希；再核对脚本编码、引用和分支。补丁命名、加载顺序、密钥与特定插件参数由当前项目证据确认，不写成引擎通则。

### 存档的字面量字段与执行坐标

KAG/TJS 存档可能把数值序列化为文本 token，而不是二进制整数。若宿主提供
`galgear_literals`，先读取 `verification_contract(topic="structured")`；可用固定的
`tjs_const` 数据语法完整解析对象、数组和字段，并保留完整 token 的原始跨度。
该接口不执行 TJS，也不允许模型提供语法、回调或字节修补代码。字段路径必须从
当前原始输入确定；UTF-16 字符的高零字节不能代替十进制整数。

完整数据解析只证明字段及类型存在。场景名、保存标记、指令数组下标、文本 ID、
当前显示内容与历史记录可能属于不同坐标；必须核对同版本保存、恢复及执行逻辑，
不能按字段名称或最近一条历史直接认定当前节点。选择等待或恢复状态也不能自动
替代当前对白。缺少已核验的关联时保留未解决结果。

若宿主提供固定的 PSB / TJS 类型树读取器，可优先完整解析后按实际对象键及数组
下标查询，避免反复编写临时语法解析器。完整树应保留字符串引用节点的位置，并将
它与字符串池字节跨度分开；消息坐标不能统一改成字符串池偏移。明确当前处理的是
语言文本记录、显示指令、姓名／语音引用还是资源标识，再编写对应的对白记录程序。
结构化字段的原生值与节点角色仍须分别核验，固定读取器不代替场景执行语义。

标准 KAG 的[保存／恢复实现][kag-parser]保存脚本来源和标签，恢复时重新加载来源并
跳转到标签；不能把同时出现的物理行或字符位置当作最后显示台词。需要核验这种恢复
入口时，完整扫描有效原始 KAG 脚本的标签、重复标签别名和原始字节位置，并检查
[标准书签结构][kag-bookmark]中的权威解析器状态。恢复标记单独保留，不能作为对白
记录或阅读历史；单独提取标签不算完成剧情提取。定制框架的外层跳转入口不能证明
其内部场景播放器的剧情位置，须保留这层限制。

文本流模式 1 的相邻位交换可由宿主固定 `swap_adjacent_bits` 变换重放；容器头与
正文范围仍由适配器从实际输入确认。该变换不代替 XP3 的专用过滤器。若资源仍受
未知过滤器保护，必须确认完整原生校验与结构；局部可读文件头或字符串不足以证明
解密成功。缺少正确解码能力或节点映射时报告相应不支持原因，不能因此报告没有
剧情存档，或提供局部字节修补来凑出内容。

归档成员过滤与内部文本包装应分别声明和检查。恢复成员后核对完整原生大小及可用的
原生校验信息；校验值从实际归档索引读取，不能由分析程序自行提供一个期望常数。
XP3 不同加密方案对校验计算的阶段可能不同，按实际版本确认；校验失败应保留未知，
不能凭局部可读头部或字符串提升为完整过滤器。完整字节校验也不证明对白语言、姓名
关联、阅读顺序或存档节点角色正确。

## 来源、版本与验证边界

实现参考固定为 KiriKiri Z `fd5c4baa6a2ef5978db1bd043634351f48667daf` 和 GARbro `b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0`。本页为源码阅读等级，不声明真实游戏或运行验证。遇到非标准索引、专用过滤器、文本流模式或编码设置不符时失效，改查目标版本实现；项目特殊参数留在本地。

标准标签恢复的新增参考固定为 KAGParser `c2269b26b390bd09aa1f2b39d7d779cb79c26f2e`
和 KAG3 `1f3ab309106d210e3169bbbe0fb4e066ae463b42`，为公开源码阅读与合成验证等级。
替换脚本加载器、书签保存／恢复或标签规则的变体不在这项核验范围内。

[archive]: https://github.com/krkrz/krkrz/blob/fd5c4baa6a2ef5978db1bd043634351f48667daf/base/XP3Archive.cpp
[text]: https://github.com/krkrz/krkrz/blob/fd5c4baa6a2ef5978db1bd043634351f48667daf/base/TextStream.cpp
[garbro-xp3]: https://github.com/morkt/GARbro/blob/b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0/ArcFormats/KiriKiri/ArcXP3.cs
[kag-parser]: https://github.com/krkrz/KAGParser/blob/c2269b26b390bd09aa1f2b39d7d779cb79c26f2e/KAGParser.cpp
[kag-bookmark]: https://github.com/krkrz/kag3/blob/1f3ab309106d210e3169bbbe0fb4e066ae463b42/data/system/MainWindow.tjs
[adler32]: https://www.rfc-editor.org/rfc/rfc1950.html#section-2.2

[psb-crypto]: https://github.com/UlyssesWu/FreeMote/blob/d87be1f3b8458c81822825fd6d56123d4df860b4/FreeMote/PsbFile.cs
