# AI 剧情分析使用每任务 Windows LPAC

## 决策

AI 宿主通过 `CreateProcessW` 在 Less-Privileged AppContainer 中启动。每次调用使用随机包身份，权限由 Rust 根据本次游戏、候选存档、固定技能路径和运行库生成。模型工具参数不能新增 Windows 授权。

游戏目录、解析技能及运行库只读。候选存档按文件授权，所在目录仅允许列举，不将共享 AppData 目录递归开放。没有候选文件的旧准备入口沿用已发现的存档目录。当前工作目录拥有读写及删除权限。临时目录、用户配置目录环境变量均指向工作目录，AppContainer 默认配置目录的文件权限收紧为只读。

Windows 为 LPAC 提供的基础系统资源仍可访问。宿主仅增加联网、只读注册表、密码服务、COM 和 ETW 能力，支持现有 HTTPS 请求、PowerShell/.NET 解析器及 WinRT 读取器。纯解析调用不申请网络能力。模型和源码的 HTTPS 请求通过显式继承的管道句柄交给宿主网络线程，支持本机代理；转发仅接纳现有 OpenAI、DeepSeek、GitHub API 和 raw.githubusercontent.com 域名。Python 的 NoRedirect 策略、HTTP 状态码及 OpenAI 代理 / DeepSeek 直连策略保留。转发无文件接口，取消会丢弃在途请求并回收线程。

进程以挂起状态创建，先加入现有 Job Object 再恢复主线程。所有后代继承隔离，退出和超时通过 Job 回收整棵进程树。标准流使用明确的句柄继承名单。正常结束回收 ACL 和 profile，回收失败返回错误。应用被强制终止时，Job 负责杀死进程，下次启动根据持久记录回收残留权限和身份。

## 异常退出恢复

`windows_sandbox::journal` 将恢复记录保存在当前用户的 `%LOCALAPPDATA%/GalGear/ai-sandbox/v1`，与安装位置、游戏库和任务工作目录分离。拒绝把该目录包含在可写工作范围内，记录中只保存随机身份和授权路径，不保存模型密钥或请求内容。

创建身份前先创建并同步记录文件；每次修改输入或工作目录 ACL 前，将路径以 JSON 行追加并同步。身份名在文件名中，所以空文件也能回收 profile 初始化期间的崩溃。末尾未完成的行尚未触发授权，可以忽略；完整行损坏则保留记录并报告错误。profile 的默认目录由 Windows 按身份删除，无需记录其内部文件。

任务持有记录文件的独占、不可继承句柄。其他实例恢复时跳过占用文件，不依赖可能被重用的 PID。强制终止会由系统释放句柄。ACL 更新与撤销通过同一桌面会话的命名互斥量串行执行，避免并行实例覆盖彼此的权限。恢复只移除记录身份的 ACE，保留其他用户及活跃任务的授权，不恢复整份历史 ACL。

桌面在发布缓存首屏后、开放业务命令前，于原有启动后台线程调用 `ai_summary::recover_sandboxes`。依次撤销授权、删除 Windows profile 与身份注册，全部成功后才通过仍持有的句柄标记删除记录。恢复可重复执行，缺失的 profile 和已删除的输入路径不会导致失败。磁盘根失联、权限不足或路径变为重解析点时保留记录，下次启动重试；警告进入原有游戏库警告及诊断日志，不阻断其他本地功能。

恢复只处理本机制写入的有效记录，不扫描整块磁盘猜测旧 SID 的授权位置。增加记录机制之前、没有记录的历史残留不在自动恢复范围内。

## 运行库与兼容性

随附 Python 宿主改为 PyInstaller `onedir`。原有 `onefile` bootloader 自解包目录的 ACL 无法保证 LPAC 读取 DLL，实际原生测试会加载失败。发行包必须包含 `tools/ai-agent/_internal`，原有目录复制和 Tauri 资源映射会包含这些文件。旧开发目录需重新运行 `scripts/build-ai-agent.ps1`。

源代码调试入口额外开放 Python 解释器、标准库、依赖 DLL 和 AI 宿主源码目录，只授予读取权限。Windows 授权初始化失败时终止该次分析。ACL 不可修改、文件系统不支持 ACL 或根路径包含重解析点时，会返回具体错误。

## 验证

Windows 原生夹具覆盖输入读取、输入写入拒绝、共享存档中的其他文件拒绝、工作目录增删改、目录联接越界、profile 写入拒绝、PowerShell 启动、子进程继承及隔离进程树回收。恢复测试强制终止独立宿主，核对 ACL、profile、身份注册和记录均被清理，并验证活跃任务保留、部分初始化、截断记录、重复恢复及失败重试。所有文件操作使用隔离夹具。随附宿主测试通过真实解析入口运行，不请求模型或访问真实游戏和存档。

实现依据为 Microsoft 的 AppContainer 启动及 LPAC 说明。[Launch an AppContainer](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer)

Windows 允许重复删除不存在的 profile，失败后应重试。[DeleteAppContainerProfile](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-deleteappcontainerprofile)
