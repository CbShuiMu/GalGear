# 设置、存档资源与数据导出

- 游戏详情的封面、标签、资料来源，以及“确认游戏资料与启动器”的来源选项，只列出设置中保存并验证成功的源；2DFan 可免凭据搜索。确认窗口的“批量修改”用数据库图标打开来源菜单，对待确认行统一改源并丢弃旧候选和迟到的搜索结果。
- 2DFan 下载时根据资源标题、类型及文件名识别存档，以 `.galgear-resource.json` 随解压内容原子发布。详情补丁卡片显示“存档”标记；只有唯一存在且已确认的存档位置才显示安装按钮。后端重新发现并校验目标，在游戏停止时安装；原文件备份在补丁事务记录目录，支持恢复，不在存档目录创建原始补丁文件夹。同名不同扩展的存档不按游戏补丁规则移除。
- 路径与备份中的 Magpie 配置读取 `scalingModes`，修改默认 `profiles[0].scalingMode`。使用配置内容标识选项，避免 Magpie 重排后误选索引；方案被修改或删除时要求重新读取。保存复用既有退出、原子写入、重启流程，保留其他应用配置及未知字段。下拉框可搜索、键盘选择，长名称换行。
- 每游戏保留份数移到下一行；配置文件路径右侧的“导出”打开系统保存对话框，默认文件名为 `GalGear.pvnExport.zip`。取消不提示成功，失败保留目标原文件。

## PotatoVN 格式与范围

依据 [官方导入说明](https://potatovn.net/usage/advance/data-exchange.html) 与 [官方源码](https://github.com/GoldenPotato137/PotatoVN/tree/5573d0c9beda57306b57bead8d985e5dd0bdfcec) 的 `ExportTask`、`LocalSettingsService`、`GalgameAndUidConverter` 和游戏、游戏库、分类模型。

包内使用 `data.galgames.json`、`data.galgameSources.json`、`data.categoryGroups.json`、`data.dataStatus.json`，封面放在 `Images/`。稳定 UUID 连接游戏、安装条目及收藏分类；属性使用 PotatoVN 的 `Value`/`IsLock` 结构，迁移标志保留 JSON 导入路径。兼容游戏级旧启动字段和新安装条目配置。

转换游戏名称、简介、制作商、引擎、发行日期、标签、支持的来源 ID、启动器及参数、游玩状态、累计分钟和最近游玩时间。收藏父子层级转成分类名称路径。GalGear 不保存逐日时长，所以不构造虚假的每日游玩记录。封面只读本地缓存；不包含游戏文件、存档文件、账号凭据、云存储或 AI 配置。GalGear 独有的工作目录、工具路径与界面配置不能直接映射为 PotatoVN 设置。

PotatoVN 的“导入数据”会替换其已有数据。界面提示先在 PotatoVN 备份。本机未安装 PotatoVN，验证覆盖官方结构、ZIP 内容、关联字段和写入失败保留原文件，未运行 PotatoVN 客户端端到端导入。
