# 游戏管理器数据导入

设置 → 游戏库中，“导入”位于“添加文件夹”左侧。选择来源和备份文件后先预览，确认后合并新增条目。取消文件选择或关闭预览不会导入。

| 来源 | 可选文件 | 对应来源功能 |
| --- | --- | --- |
| PotatoVN | `.pvnExport.zip` | 设置中的导出数据 |
| ReinaManager | `.db` / `.sqlite` / `.sqlite3` | 数据库备份；本地封面放在备份旁的 `covers/` 目录，远程封面保留 URL |
| LunaBox | 数据库或全量 `.zip` 备份 | 备份中包含 `database/games.csv`，也兼容旧版根目录 `games.csv` |

迁移名称、简介、开发商、发行日期、支持的资料源关联、封面、标签、收藏、游玩状态、累计时长、最近游玩时间，以及存在的存档目录、转区和缩放开关。PotatoVN、ReinaManager 的累计分钟换算为秒，LunaBox 汇总会话秒数。GalGear 暂不迁移逐日统计、评价、账号凭据、AI/云端设置、存档文件和工具私有配置；复杂启动参数仍需在游戏配置中核对。

只接收本机存在的绝对 EXE 路径及游戏目录。仅有条目资料、Steam 链接、跨电脑失效路径等显示跳过原因。按游戏目录和启动程序去重，已有或已排除条目保留当前资料；重复导入不会累加时长。确认时再次检查路径与重复项。导入条目沿用扫描器的目录标识，后续扫描不会重复生成。

ZIP 成员按需读取，不落盘解压，不执行 LunaBox 的 SQL 文件。拒绝路径穿越、符号链接、重复成员、超限数据与损坏格式。ReinaManager 以只读 SQLite 连接读取已知表，兼容标量旧结构、内嵌 JSON 元数据结构和 `game_sources` 结构。数据只写入 GalGear 自身配置/缓存，不改源软件备份和游戏文件。

配置以一次原子写入提交，`manual_games` 保留完整导入条目；派生目录缓存或时长缓存写入失败时记录警告，并可从配置恢复。后端预览计划留在进程内，确认只携带一次性编号，不能从前端提交任意游戏条目。

来源结构核对：

- [PotatoVN 官方导入导出说明](https://potatovn.net/usage/advance/data-exchange.html) 与此前导出实现核对的 [源码版本](https://github.com/GoldenPotato137/PotatoVN/tree/5573d0c9beda57306b57bead8d985e5dd0bdfcec)。
- [ReinaManager 数据库备份](https://github.com/huoshen80/ReinaManager/blob/main/src-tauri/src/backup/database.rs)、[实体](https://github.com/huoshen80/ReinaManager/tree/main/src-tauri/src/entity) 与 [统计分钟单位](https://github.com/huoshen80/ReinaManager/blob/main/src-tauri/src/database/repository/game_stats_repository.rs)。
- [LunaBox 备份实现](https://github.com/Saramanda9988/LunaBox/blob/main/packages/backend/service/backup_service.go) 与 [表结构](https://github.com/Saramanda9988/LunaBox/blob/main/packages/backend/migrations/init.go)。

读取依赖新增 `rusqlite`（内置 SQLite，Windows 无需另装数据库）和 `csv`（正确处理带逗号、换行、引号的字段）。解析、预览及合并逻辑位于 core；Tauri 仅协调文件选择、互斥和持久化；UI 只管理交互状态。
