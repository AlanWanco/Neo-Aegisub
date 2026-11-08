# Neo Aegisub GPUI 功能移植计划

目标：宿主只保留 C++ ASS 文档、选择/撤销、媒体后端、Automation 与命令行服务，全部界面由 GPUI 前端绘制；宿主以回环桥接接收前端请求，不再包含任何 wxWidgets 窗口或控件。只有在 GPUI 中完成交互并验证的功能才记为原生移植。

## 当前基线

- [x] 基础提交 `9f90e5bf5`：独立 GPUI 窗口、认证回环桥接、真实字幕列表/文本、文件操作、视频帧、播放和退出清理。
- [x] 宿主彻底移除 wxWidgets：源码不再包含 wx 头文件，Meson 不再查找或链接 wx，`otool -L build-ui-gpui/aegisub` 无 wx 库。
- [x] macOS arm64 / Rust 1.95 增量构建和桌面联调。
- [x] macOS 正常关闭 GPUI 前端后宿主及子进程均退出；可再次启动。
- [x] Windows、Linux 构建通过；Windows MSVC/ARM64 CI 与 GPUI 桌面冒烟通过，Linux 桌面交互仍未验证。
- [x] Windows 启动崩溃（`0xC0000005` / `0xC0000409`）已按 `STARTUPINFOW` 与宿主生命周期修复，Windows MSVC 桌面断言复测通过。

## 命令、字幕与文档

- [x] 从实际 C++ 注册表提供命令名称、帮助、可用/选中状态、原有快捷键和上下文绑定。
- [x] GPUI 显示平台/用户合并后的菜单、MRU 与可搜索命令入口；支持运行时 Automation 宏。
- [x] 显式区分 GPUI 交互、共享项目操作、Lua 宏与仍待移植的命令；旧版对话框入口已随 wx 宿主一并删除。
- [x] 工作台统一深浅主题、紧凑侧栏、标题和文件名同排。视频画布、音频、视频和字幕编辑保持同窗，全部由 GPUI 绘制；播放与视觉工具控件也在 GPUI 内。保留编辑区高度分隔条和上下微调按钮；音频图默认高 80 px。
- [x] 字幕多选、按字段批量修改、注释、复制/剪切/粘贴、光标处分行与共享撤销/重做。
- [x] 编辑层、起止时间、样式、角色、效果和边距；完整补丁先校验再修改。
- [x] 字幕文本草稿在切行、保存和执行操作前提交；按字符映射跨行 UTF-8 文本选区。
- [x] 原生样式编辑与管理：ASS 全字段、RGBA、重命名及 `\\rStyle` 引用同步、复制/新建/删除。
- [x] 原生样式库：既有 `.sty` 库的加载与持久化、目录增删、样式编辑/复制/删除、脚本与库互拷、文件导入预览和选中导入。
- [x] 原生脚本属性：元数据、脚本/布局分辨率、换行、缩放边框、YCbCr 信息及媒体详情。
- [x] 原生打开字幕/音频/视频/时间码、另存为文件选择器；操作继续使用当前 C++ 项目。
- [x] 字幕网格标签显示/简化/隐藏共用旧版格式化逻辑，编辑器保留完整 ASS 文本。
- [x] 翻译助手：按解析后的纯文本块导航、空白过滤、预览/提交并前进、保留标签和绘图、源文本冲突检测。
- [x] 样式助手：前缀筛选、选择/预览/应用并前进、原有助手快捷键与媒体定位。
- [x] 修复编辑并撤销后重新打开/新建文档可能错误标记未保存的问题。
- [ ] 中文 IME 预编辑、候选窗口与组合文本撤销；当前仅验证了 Unicode 文本写入，合成按键不足以完成 IME 验收。
- [ ] 非文本表单草稿跨文档/外部修改的完整验收；旧对话框返回后的焦点与快捷键覆盖。
- [x] 原生查找替换与条件选行：四类字段、完全/包含/正则、大小写/反向/注释过滤、选区集合操作、历史与共享撤销。
- [x] 共享搜索修复：NFC 匹配使用临时文本和 ICU 字节位置映射，保留捕获组、前后文及零宽 Unicode 匹配；旧版搜索窗口也验证替换与撤销。
- [x] 原生按字段覆盖粘贴：11 个字段、预设、单行向后/多选逐行覆盖、末尾截断、保留行 ID 与撤销。
- [x] 原生附件管理：字体/图像批量添加、导出、确认删除、过期列表拒绝、逐字节导出与共享撤销。
- [x] 原生指定编码打开：当前 iconv 支持列表、编码检测与常用编码；GB18030、Shift-JIS、UTF-16/32 大小端 BOM 验证。
- [x] 原生自动保存恢复：沿用原有扫描、文件名解析、分组和排序；自动保存/原始备份/异常恢复三类版本、筛选和桌面打开。
- [x] 原生拼写检查：复用平台 `SpellChecker` 工厂与 ASS token 解析，支持多语言、跳过项、替换/全部替换、忽略、建议和个人词典操作。本机实际使用 macOS 系统拼写服务，后端回归和个人词典桌面操作已验证；Hunspell 构建尚未实测。
- [x] 原生导出面板：格式、编码与过滤器的选择由 GPUI 完成；宿主经桥接接收导出请求。

## 音频和卡拉 OK

- [x] 复用既有 provider、波形/频谱渲染器与位图缓存；通过原始 BGRA8 像素块交给 GPUI 纹理绘制，选区、时间标记和播放位置由 GPUI 在同一工作区叠加。
- [x] 使用 `AudioTimingController` 处理左右键、Ctrl/⌘、Alt 和标记拖动；拒绝过期手势。
- [x] 播放/停止、提交/还原、提交并前进、自动提交/前进/跟随、吸附选项。
- 普通打轴释放语义与 wx 保持一致：单击松手不拖动终点，只有实际移动才调用标记拖动；Shift 临时反转吸附。起止输入显示未提交音频选区，回车提交字段，单独音频播放也刷新游标。
- [x] 滚动、选区/端点定位、缩放、增益、速度与共享音量状态。
- 波形和频谱支持以鼠标位置为中心滚轮缩放；缩放按钮从左到右为缩小、放大，以视图中心缩放。显示增益与播放音量分别使用波形、扬声器 SVG 图标，提示说明二者用途。
- 音频操作保留在字幕编辑工作区；独立“音频调时”侧栏入口、窗口路由及专用页面控件已移除。
- [x] 卡拉 OK 音节卡片、Unicode 分割/合并、应用/取消、`\\k` / `\\kf` / `\\ko` 与撤销。
- [ ] 旧版 `audio/karaoke` 全屏卡拉 OK 预览随 wx 画布删除；前端目前仅提示尚未移植，打轴用的音节卡片不受影响。
- [ ] 自动跟随、吸附和提交前进的组合操作、长文件解码/失败反馈、音量与增益联动验收。
- [x] 原生音频剪辑保存：按选中字幕的完整时间范围调用既有 PCM 写入器，WAV 参数/采样逐字节核对，桌面文件选择及覆盖授权。

## 视频和视觉排版

- [x] 原生时间轴拖动、逐帧/关键帧/当前句跳转、播放位置/速度、显示比例同步。
- [x] 视频帧由宿主以 BGRA8 二进制直传（`video_frame` + `since` 增量），GPUI 包成 GPU 纹理显示；原 wx 画布代码（`VideoDisplay`、`SetParent` 内嵌、`VideoOutGL`）已删除。
- [x] 视觉工具走 `visual` 协议：宿主把工具绘制记录成图元（`src/visual_render.cpp`），GPUI 覆盖层用路径与文本重画，并把指针位置按画面矩形换算回工具坐标；七种工具与八种矢量子工具可用。
- [x] 主工作区恢复左右两栏（音频条 + 撑满高度的视频卡片 / 编辑器 + 字幕列表），中间竖向拖动条调列宽；没有视频时预览区显示空态与“打开视频”按钮，标题行、画面和控制行分行排列。
- [x] 覆盖层单测覆盖 ASS 遮罩非零环绕与反相、缩放后面积不变、信箱留边坐标换算；宿主协议实测 `drag` 4 条、`scale` 18 条、`rotate/z` 37 条、`clip` 13 条图元，按下拖动提交 `\pos`。
- [ ] 所有主工具/子工具的完整桌面指针操作对照；窗口外松开、缩放/比例/选择变化期间的交互验收。
- [x] 原生精确跳转、自定义显示比例、分数帧率/颜色/棋盘格空白视频、解码器及实际/强制色彩矩阵和范围详情。
- [x] 原生关键帧与时间码保存；VFR 往返、CFR 视频帧数输出、桌面保存与字节核对。
- [x] 缩放、复位与中键平移由 GPUI 负责，画面与覆盖层共用同一变换；安全区方框在 GPUI 重绘，宿主只提供开关状态。“适应”缩放预设下拉按钮使用常驻描边胶囊；“适应窗口”按钮使用内置 fullscreen SVG，保留悬停提示与无障碍名称。
- [ ] Windows/Linux 的 `video_frame` 直传、覆盖层指针映射与 DPI 缩放验证。
- [x] macOS / Windows libmpv 实验接入（工作区“libmpv”开关）：连续播放且画面适应窗口时，使用固定在预览区的原生视频窗口，跳过 BGRA 取帧；暂停、视觉工具、安全区、缩放/平移时使用原 GPUI/libass 预览。音频仍由宿主管理。字幕通过 `native_subtitles` 导出临时 ASS，并按文档 revision 更新；宿主关闭时清理文件。mpv 命令、时钟读取和销毁不阻塞 GPUI 主线程。macOS 使用无边框子 NSWindow，Windows 使用禁用鼠标输入的 Win32 子容器，输入交给 GPUI。macOS 包内运行库已实际验证 1080p60 视频、ASS 字幕和暂停切回编辑预览；Windows GNU 目标交叉检查通过，Windows 桌面运行仍待验证。
- [x] Windows x64 / ARM64 打包动态 libmpv：固定 `zhongfly/mpv-winbuild` 的 `2026-11-07-55d3feb45c` LGPL 构建，校验下载归档、解压 DLL 与许可证 SHA-256 和 PE 架构；安装包、便携包随附 `libmpv-2.dll` 和 `licenses/libmpv`。本机已实际下载校验两种架构；x64 便携 ZIP 使用既有 CI 可执行文件和新运行库验证通过，不能替代新版 Windows 桌面验收。
- [x] macOS arm64 / x86_64 运行库：`tools/macos-build-libmpv.py` 固定源码版本与 SHA-256，构建 LGPL mpv 0.41.0、FFmpeg 8.0.2、libass 0.17.5、libplacebo 7.387.0 与 glslang 14.3.0，并直接链接固定的 MoltenVK 1.4.2。两种架构均已构建，新增动态库的架构、最低系统版本不高于 macOS 13.0、仅依赖包内或系统库的检查通过；打包递归搬运依赖和许可证，不需要 Vulkan loader 或 ICD 清单。实际 arm64 应用包通过结构检查并运行；该冒烟包复用了既有宿主依赖，不能证明整包在 macOS 13 上运行，也未在 Intel Mac 上执行。
- [ ] Windows libmpv 桌面验收：已有 CI 冒烟步骤检查真实 renderer 子窗口、预览边界、窗口缩放跟随、鼠标命中仍属 GPUI、播放时钟推进和暂停隐藏；本轮尚未运行 Windows CI。Linux 仍使用原 BGRA 预览。
- [x] 原生工具栏显示状态，偏好设置与原命令/快捷键即时同步。
- [x] 原生快捷键列表、筛选、组合键录入、绑定/移除、冲突检查与显式替换、默认恢复、导入旧版快捷键文件；桌面即时触发和重启持久化。
- [ ] 原生布局模式、窗口命令，以及全部上下文与特殊按键的对照验收。

## 时间处理与 Automation

- [x] 平移时间：抽取与旧版共用的处理，时间/帧、方向、全选/多选/选择及以后、起止字段、历史恢复与持久化。
- [x] 平移回归：非连续多选、负向归零、无效输入原子拒绝、CFR/VFR 帧换算、撤销/重做；桌面验证延迟 500 ms、撤销、无效输入。
- [x] Automation 管理器：原生脚本列表、名称/路径/作者/版本/警告/错误、宏与过滤器详情、添加/重载/卸载、重新扫描和全部重载。
- [x] 管理器使用既有 `ScriptManager` / `ScriptFactory`，动态刷新宏注册表；全部重载保留当前项目明确加载的脚本列表。
- [x] 管理器回归：宏执行/撤销、加载失败与修复、卸载注销宏、拒绝重复/过期操作；桌面验证添加、执行、撤销、重载、确认卸载。
- [x] 原生分辨率重采样：复用样式/坐标/绘图/边距和矩阵转换，比例/边框模式、输入原子拒绝与精确撤销，桌面应用/撤销。
- [x] 原生时间后处理：与旧版共用前后延时、相邻行间隙/重叠/偏向及关键帧规则，样式/选择/注释过滤、无变化不提交；桌面应用/撤销。
- [ ] Kanji 定时等剩余专业工具。
- [ ] 原生 Lua 生成的交互对话框、导出过滤器设置、进度和取消；目前仍使用旧版兼容界面。
- [x] 原生偏好表单：本机构建 143 个可用项，分类/搜索、类型/范围校验、整批验证、应用/放弃、局部默认恢复、写配置前备份。
- [x] 主题、工具栏、字幕列表/编辑字体与颜色、软换行快捷键、音频配色/频谱设置/吸附距离同步；未接入的 GPUI 控件行为逐项注明。
- [x] 修复 GPUI 获得焦点时修改热键导致 macOS 全局菜单以空文档上下文计算撤销名称的崩溃。
- [ ] 偏好项逐项对照后端行为；带“仍待移植/重新打开媒体后生效”说明的条目仍未迁移对应 GPUI 控件行为。面板桌面已验证搜索、主题动态下拉、应用和即时深浅色切换；快捷键页已验证录入、空命令屏蔽、移除。剩余控件行为仍需逐项对照。
- [ ] 跨平台打包；日志与帮助已改为 GPUI 原生窗口（见 2026-11-01 帮助窗口条目）。

## 验证证据和边界

源代码中有 241 处显式 `CMD_NAME` 定义；运行时还包括生成、平台、MRU 和 Automation 命令，不能把静态计数当成完成比例。当前测试环境加载七个自带宏；命令分类数量随注册表和项目状态变化。标记 `native` 表示 GPUI 已接管入口，不能替代逐工作流验收。本轮从 macOS 当前运行时导出的命令目录含 334 条：78 个原生入口、219 个共享操作、7 个 Lua 宏、13 个旧对话框入口、16 个仍依赖旧控件、1 个内部入口。平台、媒体、项目与 Automation 状态会改变命令目录；分类仅描述此次注册表，不代表工作流覆盖率。完整目录保存在忽略的 `build-ui-gpui/command-catalog-runtime.json`。

本机回归脚本和结果存放于忽略的 `build-ui-gpui/`：命令/文档、音频/视觉、样式库、翻译/样式助手、平移/后处理、Automation 管理器、重采样、搜索、覆盖粘贴、视频设置、附件、偏好/热键、媒体保存、拼写及文件恢复各有独立检查。拼写、偏好、热键和恢复面板本轮追加了真实窗口操作；截图记录在同一目录。去 wx 后重新导出的命令目录共 324 条：225 个共享操作、92 个 GPUI 原生入口、7 个 Lua 宏，不再有旧对话框或旧控件分类；完整目录保存在 `build-ui-gpui/command-catalog-runtime.json`。其余只由协议覆盖的条目不视为桌面端完整验收。

最新视频批次通过 Rust fmt/check、七项 release 测试和 release 构建、C++ 宿主增量构建；顺序通过 `video-overscan-check.py`、`video-settings-check.py`、`video-view-check.py` 和 `visual-port-check.py`。此前桌面回归结果为 `video-view-desktop-results.json` 和 `video-cancel-desktop-results.json`，对应缩放/平移后 ASS 坐标、单次撤销、视口外释放、窗口变化及滚轮期间取消；这些结果早于安全区新增。直接发送视觉协议请求时，GPUI 必须停在非排版页，避免排版页同时请求不同尺寸画布。打开新样本前关闭视频，避免测试触发关联媒体询问。共享预览上的有状态回归必须顺序执行。

工作台视觉批次通过 Rust fmt/check、七项 release 测试和 release 构建、C++ 宿主构建以及偏好协议回归。后台辅助功能操作验证属性字段修改/应用/单次撤销和覆盖粘贴/快捷键/命令页初始化；实际指针打开并选择倍率和平移菜单。检查深浅主题、1024×760 紧凑布局和安全区叠层，截图均只捕获预览窗口；结果为 `visual-polish-results.json`、`visual-polish-menu-results.json`、`visual-polish-contrast-results.json`。八组主题文字/按钮对比度均高于 4.5。另按用户要求将播放控制移至视频下方、文本编辑移至字幕列表上方；普通与窄窗口的位置和原生播放/暂停、逐帧、文本应用/单次撤销通过，结果为 `stacked-workspace-results.json`。完整键盘巡检、屏幕阅读器体验、跨平台和大字号布局尚未完成；有些旧表单只继承主题，未逐页重新排版。

macOS 个人词典属于系统状态，隔离 profile 不能隔离学习词。已通过共享拼写服务核查并清理已知测试词 `speling`、`wrold`，两者的 `can_remove` 均为 false；这不能推广为整个系统词典已恢复。测试文档无未保存修改，滚轮三项设置恢复为 0/2/4；最新分类与安全区结果见 `command-catalog-runtime.json`、`video-overscan-results.json`。

2026-11-01 打轴与播放修复：隔离窗口载入三行 ASS 与 18 秒 1080p30 含音轨视频。修复前宿主声明 1920×1080，但实际 JPEG 为 600×338；修复后两者均为 600×338，窗口可连续显示播放画面。实际鼠标与键盘验证单击设置起点且保留终点、左键拖选、右键改终点、Shift 临时吸附、Alt 平移保持时长、松手后停止拖动、S 播放未提交选区、Enter 提交并前进、时间字段回车提交、按帧显示随拖动更新、自动提交及单次撤销/重做。保存 ASS 逐行核对第二行 4.98–8.46 秒、第三行 8.05–10.00 秒；毫秒选区按 ASS 百分之一秒精度写入。Rust 8 项测试及 Meson 两组测试通过，宿主和前端 release 构建通过。截图与保存样本在忽略的 `build-ui-gpui/timing-smoke/`。本轮未实测 Windows/Linux、物理声卡输出、长文件跟随或全部快捷键组合。

2026-11-01 音频交互与播放响应：主窗口已实测鼠标锚点滚轮缩放、缩小/放大按钮及播放音量 100%→90%→100%；滚轮缩放保持选区不变，锚点误差在测试图像对应的数毫秒范围内。最终版本移除独立“音频调时”侧栏入口、窗口路由与专用页面控件，并在实际主窗口复核入口消失、音频载入、滚轮缩放、缩放按钮及音量控制。新增三项缩放测试覆盖锚点保持、媒体边界和同倍率限幅不漂移，移除独立页面后 Rust 共 11 项测试通过，前端 release 构建通过；此前宿主 release 构建通过。播放命令同步状态后立即请求视频帧，移除首次取帧对最长 350ms 空闲轮询的等待。在清除全部媒体提示的同一 18 秒 1080p30 测试视频上，实际点击下方播放按钮到视频区域首个像素变化，四次修复前为 189–206ms，修复后为 108–136ms；此测量不代表物理音频输出延迟。字幕属性展开时上下两排布局间距由 4px 增至 7px，预留高度同步增加 3px，并已在实际窗口核对。截图和测量结果保留于忽略的 `build-ui-gpui/audio-scroll-1793496660716315000/`；本轮未实测 Windows/Linux。

2026-11-01 样式下拉滚动：字幕编辑区的样式菜单启用现有组件的滚动功能，高度随窗口限制为窗口高度的一半且最多 450px，内容超出时显示滚动条。前端 release 构建通过；macOS 实际窗口以 60 个样式验证滚轮滚到末尾、滚动条双向拖动、选择末尾样式写回字幕及撤销，并在 1024×760 小窗口重复验证末尾选择。三个样式的短列表保持内容高度，选择与撤销正常。截图和结果保留于忽略的 `build-ui-gpui/style-scroll-1793500244996164000/`；本轮未实测 Windows/Linux。

2026-11-01 视图模式同步：四种视图的选中状态以 GPUI 当前布局为准，原生菜单和命令目录共用该状态，刷新目录后仍保留当前模式。音频+字幕视图改为音频在上、文本编辑和字幕列表在下，并占满可用宽度。前端 release 构建及 11 项 Rust 测试通过；macOS 实际窗口切换四种视图，核对音频/视频显隐与菜单单一勾选，刷新命令状态后复核音频模式，并在 1024×760 窗口核对上下布局。测试文档无未保存修改，截图和结果保留于忽略的 `build-ui-gpui/view-layout-1793501264784991000/`；本轮未实测 Windows/Linux。

2026-11-01 帮助窗口与旧版快捷键导入：帮助菜单的检查更新、关于、日志改为 GPUI 原生卡片窗口，点击遮罩或关闭按钮退出；更新检查改用 fork `AlanWanco/Aegisub` 的 releases，按修订号与语义版本取最新，跳过草稿和仅源码发布，预览版与正式版一并比较，结果含来源地址、当前版本和发布页链接，窗口关闭后轮询停止。快捷键页新增“导入旧版快捷键…”，同时接受旧版 `{modifiers,key}` 与现行字符串格式，同一上下文与按键以文件中靠后者为准，未知命令、无效按键与完全重复计入跳过数。修复启动后再次执行“GPUI 工作台”命令会开出第二个前端窗口的问题。macOS 实际窗口核对三个窗口的标题、内容、链接与遮罩关闭，导入实测 2 条、跳过 2 条并核对写入的 `hotkey.json`；前端 release 构建、11 项 Rust 测试与 Meson `gtest main` 通过；本轮未实测 Windows/Linux。

2026-11-01 语言目录补齐：从当前源码重新生成 `po/aegisub.pot`（新增 47 条、移除 9 条旧更新检查文案），并用 `po/update_po.sh` 合并进全部 33 个 `.po`，使每个语言文件都含当前全部 msgid。`po/zh_CN.po` 补齐新增与 fuzzy 条目后 1716 条全部翻译，无 fuzzy、无未翻译。新增 `po/build_catalogs.sh` 与 meson `catalogs` 目标，把各语言编译到 `<build>/po/<lang>/LC_MESSAGES/aegisub.mo`；`tools/preview-ui.sh` 启动前编译并导出 `AEGISUB_LOCALE_DIR`，构建树运行不再回退英文。macOS 实际窗口确认日志输出 `[locale] zh_CN catalog loaded`，wx 菜单与 GPUI 菜单（帮助：网站/Bug跟踪系统/IRC 频道/检查更新/关于/日志窗口；视图：四种视图与标签显示）全部中文，检查更新窗口显示“Aegisub 无可用更新。”。`msgfmt -c` 报 `po/be.po` 七条复数条目格式说明符不匹配（既有翻译缺陷，已记录未改），故目录编译不带 `-c`，避免该语言整体丢失目录。wxWidgets 自带的 Window 菜单（Minimize All/Zoom All/Fill/Center/Enter Full Screen）仍需 `wxstd.mo`，Homebrew 的 wx 3.3 未附带，本轮未处理。

2026-11-01 语言选择器收口：`WITH_GPUI_DEFAULT` 构建启动时把 `App/Language` 固定为 `zh_CN`，所以“界面语言”里另外 32 种目录只是死选项——选中后要重启，重启又被覆盖回中文。`src/gpui_dialogs.cpp` 的 `languages()` 在该宏下只返回当前生效语言并附 `note` 说明，前端把 `note` 显示为窗口副标题；非 GPUI 构建仍返回完整列表。macOS 实际窗口确认语言窗口只剩“中文（中国）”与说明文字，点击遮罩可关闭；前端 release 构建、`fmt --check`、11 项 Rust 测试与 Meson `gtest main` 通过。

2026-11-01 音频把手与缩放跟随：灰色竖线原来也画了手柄，但它们对应秒刻度、非活动行与关键帧，本来就不响应拖动；现在只有活动行的两个选区边界带手柄（起点取 `Colour/Audio Display/Line boundary Start`，终点取 `... End`），其余 marker 只画 2px 竖线。实际鼠标拖动手柄把选区终点 5000 改到 3581，宿主按同一手势收到 down/drag/up。滚轮缩放约 400ms 黏滞的成因是 `refresh_audio` 要求回复的 `view_start`/`span`/`mode`/`gain` 与当前视图完全相等才换图，滚动期间视图每帧都在变，回复被逐条丢弃，波形在整个手势里冻结、停手才跳变；改为只校验 `epoch` 与 `render_revision`，并记录每张图自身的渲染范围（`image_start`/`image_span`），按该范围定位绘制，新渲染在飞时旧图按比例跟随手势。一次连续滚轮（span 12000→4889）实测每 16ms 一帧、图片范围落后 1–2 步（约 20–40ms）而不是冻结，停手后 `image_span` 与 `span` 相等；灰色 marker 在顶部各行仍是 4 设备像素竖线，红/蓝手柄为 16×20 设备像素。`cargo fmt --check`、11 项 Rust 测试与 Meson `gtest main` 通过。

2026-11-01 标记可读性与主题取色器：音频快照原来只发 `markers` 时间点，前端只能统一画成灰色竖线，秒刻度因此盖住选区把手。现在宿主按各 provider 自己的画笔发 `{time, colour, width, dotted}`（关键帧取 `Colour/Audio Display/Keyframe`、秒刻度取 `.../Seconds Line` 且 `wxPENSTYLE_DOT`、播放位置取 `.../Play Cursor`、非活动行取 `.../Line Boundary Inactive Line`），前端在选区块之前绘制，秒刻度用 1px + 0.55 透明度，把手放大到 9×11 并加深色描边。像素核对：起点边界为 4 设备像素 `#d80000` 实线，秒刻度为 1px 虚化蓝线，把手区域不再被白/蓝线穿过。`Audio/Display/Draw/Seconds=0` 时 marker 列表只剩两条实线边界，说明关掉秒刻度后仍在的竖线是关键帧、播放位置与非活动行边界，不是漏关的秒刻度。

颜色偏好改用 gpui-component 的主题取色器（`ColorPicker` + `ColorPickerState`，弹层为同主题的 Palette/HSLA 面板，不是系统取色盘），十六进制输入框保留为次要编辑入口。宿主 `GetHexFormatted(true)` 的最后一个字节是 Aegisub 的“透明度”而不是 CSS 不透明度（`src/compat.cpp` 用 `255 - color.a` 构造 `wxColour`），前端必须取反，否则 `#D8000000` 会被当成全透明，色块直接看不见。取反后实测：`起始边线` 显示 `#d80000` 实心色块，`当前帧范围`（`rgba(255,255,255,160)`）显示半透明白色块；把 `#ffffffa0` 经取色器应用到 `config.json` 后仍是 `rgba(255, 255, 255, 160)`，取反不影响宿主语义。

同轮界面收口：`文本编辑` 标题前补 `Pencil` 图标（`StudioIcons` 之前没打包该 svg，图标枚举存在但画不出来），原来的放大/缩小编辑区箭头换成“上一条字幕/下一条字幕”，实测连按可在第 4→3→2→3 行之间切换；`样式 / 角色` 列改为同一行横向排列（样式限宽 60% 省略，角色随后），默认字号由 `grid_size * 0.8` 提到 `* 0.9`（下限 12）。

2026-11-01 波形滚动闪烁：闪烁不是缺少波形缓存，而是换图时机。`Image::from_bytes` 以字节哈希作 image id，新位图由 `image_cache` 异步解码，解码完成前 `get()` 返回 `None`；旧实现每个滚轮刻度都发一次 `audio_render` 并立刻 `image.replace` + `remove_asset`，于是每个刻度都空一帧。现在滚轮缩放走 `schedule_audio_render`：每次手势把 `render_generation` 加一，120ms 后只有代次未变才真正请求；换图时把上一张（连同它自己的 start/span）留在 `AudioPanel.fallback` 垫在下面，新图解码完成前仍有画面；回复末尾只在当前视图与已请求的 `(epoch, view_start, span, mode, gain, decoded, revision)` 不同时才续发。切换媒体时 `clear_audio_image` 连同 fallback 一起释放，不会残留上一条波形。实测：宿主临时打印 `audio/render`（验证后删除并重建），40 个事件的密集手势只产生 1 次请求；缩到最小 span 后继续放大为 0 次（已被夹紧）；同一次手势中连续抓 20 帧，音频面板亮像素始终 324322–325987（空白帧应接近 0）。Rust 11 项测试、前端与宿主 release 构建通过。

2026-11-01 播放倍速按钮：音频面板的倍速按钮原来把 `state.rate`（0.5–2.0 的倍数）和 `rate_steps` 里的百分数 50/100/150/200 直接比较，永远匹配不到，标签固定显示 `1×`，每次点击都重发 1.5×，所以点了像没反应。改为按倍率比较（容差 0.05），发送时再乘 100 取整给宿主。实测连点四次依次为 1.0→1.5→2.0→0.5→1.0，标签随之显示 `1.5×`、状态栏显示 `1.50×`；`audio_action play` 后按 1.5 秒墙钟采样：100% 走 1505ms、200% 走 3006ms、50% 走 751ms，即宿主侧倍速真实生效。Rust 11 项测试与 `fmt --check` 通过。

2026-11-01 字幕列表列分隔线：表头单元格本身就是拖拽调整列宽的把手，但原来整格只涂表头底色、没有任何分界，看不出哪里能拖。现在每个定宽列在右缘画一条 1px 竖线（`column_rule`），行与表头共用同一几何，线从上到下连续；颜色取 `muted.opacity(0.45)`（浅色主题下实测 `#aab2c7`，落在行底 `#e5eaff` 上），比 `Colour/Subtitle Grid/Lines` 的横线（`#e6eaf1`）明显，又不至于盖过文字。单元格改为 `h_full().flex().items_center()` 让竖线撑满行高，文字基线实测未移动（行文本 y 仍为 0.4255/0.4274），表头四个标签 x 位置与改动前完全一致；拖动 1975px 处的分界后列宽照常变化。

2026-11-01 列表时间列跟随按帧显示：`按帧显示` 只把编辑区的 `始`/`终` 换成帧号，字幕列表的 `开始→结束` 列仍打印时间码，两处对不上。现在列表与字段共用同一个开关：`frame_rate` 由 `time_in_frames` 且 `snapshot.fps > 0` 决定，`subtitle_cell` 的 `Time` 分支在拿到帧率时按 `ms * fps / 1000` 四舍五入成整数帧，否则退回 `format_time`。真窗口实测（24.0008 fps，四行 0/5/8/11/14 秒）：关闭时列为 `0:00:00.00 → 0:00:05.00`、`0:00:05.00 → 0:00:08.00`…，开启后同一列变为 `0 → 120`、`120 → 192`、`192 → 264`、`264 → 336`，与编辑区字段 `264`/`336` 一致；再次关闭后恢复时间码。帧号文本更短，不会撑破列宽。

2026-11-01 配置落盘：GPUI 的开关几乎都走 `cmd::call`，而这些命令改的 option 过去只在 `Options` 析构时 `Flush()`，进程被强杀就整批丢失。现在 `OptionValue::NotifyChanged()` 顺带递增一个原子计数器 `agi::OptionValueRevision()`，`src/gpui_bridge.cpp` 的 `Command()` 开头放一个 RAII 守卫，命令返回时若计数变了才 `config::opt->Flush()`——只读命令不写盘。语言选择另有单独的坑：`WITH_GPUI_DEFAULT` 分支过去每次启动都 `OPT_SET("App/Language")->SetString("zh_CN")`，把用户在偏好里选的语言覆盖掉；现在只读存储值，仅在存储语言不可用时用 `zh_CN` 兜底。隔离 profile 实测：`Audio/Auto/Scroll` 用 `audio/opt/autoscroll` 切换后立刻 `kill -9` 宿主，磁盘上已是新值；`App/Language` 写成 `ja` 后启动仍是 `ja`（修复前启动即被改回 `zh_CN`）。

2026-11-01 视频卡顿：两个独立缺陷叠加。其一是视频帧纹理泄漏——每帧的 `RenderImage` 都会进 sprite atlas，而 `replace_frame_image` 直接覆盖旧值从不释放，播放时进程按每帧一张图的速率膨胀（实测 1.8 秒 135MB→412MB 后被系统杀掉），现在换图前 `cx.drop_image(previous, None)`，清空时同样释放，同样负载下 RSS 稳定在 143–155MB。其二是大响应被截断——`OnSocket` 把 `socket->Error()` 当作致命错误，而一次写满 socket 缓冲留下的 `wxSOCKET_WOULDBLOCK` 会留在错误状态里，下一次 OUTPUT 事件到达时就被当成断线，于是 2000 行工程约 700KB 的快照只写出 319020 字节就关连接，前端 JSON 解析失败、连错 5 次自杀；现在只有 `wxSOCKET_LOST` 才算断线，写错误由 `WriteResponse` 自己判断，`BeginResponse` 也把 `response_offset` 归零。实测 2000 行工程 + 1080p30 视频 + 噪声音频连续播放 15 秒：进程存活、`frame/produced` 间隔 p50 31.0ms、步进 1（≈32fps）、RSS 143MB、零桥接失败（修复前同一负载 0.8–4 秒内必死）。

打开字幕时的关联媒体询问、未保存确认与关联脚本信任提示均由 GPUI 提示卡完成，宿主不再进入任何原生模态循环。输入法、Windows 栈溢出和跨平台运行仍为未验证项。

## 提交边界

每批只提交实际完成的范围，附构建/运行结果；提交或推送前展示对应完整 diff 并取得确认。本计划分别记录已实现功能、桌面验收与尚待验证范围，提交不代表全部迁移完成。保留根目录 41 张 JPEG、演示资源和 `subprojects/checkasm.wrap`。配置修改前备份；本机配置、测试、截图、日志和二进制只放在忽略的构建目录。


2026-11-02 仓库改名:GitHub fork 由 `AlanWanco/Aegisub` 改为 `AlanWanco/Neo-Aegisub`,本地 `origin` 同步到新地址。应用内更新检查随之改用 `https://api.github.com/repos/AlanWanco/Neo-Aegisub/releases?per_page=100`,版本清单与 GPUI 帮助页改用 `https://github.com/AlanWanco/Neo-Aegisub/releases`,清单里对 `html_url` 的前缀校验和对应单测一并更新。旧仓库路径与旧的 release 链接仍可用:实测旧 API 路径返回 301 跳到新仓库,新路径返回 200;GitHub 会对改名前地址持续重定向,但已发布版本里硬编码的旧 URL 依赖这一重定向,不在本次改动范围内。2026-11-01 条目里写的 `AlanWanco/Aegisub` 保留原样,记录的是当天真实的仓库名。

2026-11-02 改名 Neo Aegisub:产品显示名从 `Aegisub` 改为 `Neo Aegisub`,覆盖 C++ 用户可见文案(启动/崩溃/更新检查/拼写检查/字符集探测/视频属性/EBU 导出/About 对话框与标题栏)、GPUI 菜单与窗口标题、`SetAppName`、macOS bundle 目录与 `CFBundleName`/`CFBundleDisplayName`、`tools/osx-bundle.sh`、`osx-dmg.sh`、`osx-notarize.sh`、`osx-package-name.sh`、`linux-package.sh`、`preview-ui.sh`、Windows installer 的 `DefaultDirName`/`AppName`/`OutputBaseFilename`、`create-portable.ps1` 的 zip 名、`.desktop` 的 `Name`、metainfo 的 `<name>` 与现状描述、CI 归档 glob、`release_manifest.h` 的 `Neo-Aegisub-` 前缀、`test-gpui-windows.ps1` 窗口标题、README 与 `docs/developer_docs.md`、`docs/ui-prototype.md` 的正文和打包路径。刻意保留标识符与兼容面:`CFBundleIdentifier` 仍为 `com.aegisub.aegisub`,数据目录仍是 `Application Support/Aegisub`(真窗口运行后仍在原目录写 autosave/catalog),可执行文件名 `aegisub`/`aegisub-gpui-prototype`,ASS 扩展键 `[Aegisub Extradata]`/`[Aegisub Project Garbage]`/`Aegisub Video Zoom Percent` 等,注册表 `SOFTWARE\Aegisub` 与 `{userappdata}\Aegisub`,wx 控件名 `AegisubMainCanvas` 等,meson 工程名与 CI `--suite Aegisub`,上游仓库 URL、`Aegisub Group` 版权署名、`Aegisub-dictionaries`,以及 “Visit Aegisub's …” 三条指向上游站点的 msgid。翻译目录重新生成 `po/aegisub.pot` 并合并 33 个 po,再按“旧 msgid 改名后与新 msgid 完全相等”逐条去掉本次改名引入的 fuzzy,共 580 条;各文件 fuzzy 数与 HEAD 基线一致,另在 20 个文件里同步改写 63 行 msgstr 的产品名。`be.po` 的复数格式错误在 HEAD 就存在(3 处),合并后 `msgfmt -c` 仍报 2 处,不属于本次引入。验证:宿主增量构建与 GPUI release 构建通过;把实际产物装入 `Neo Aegisub.app`(PlistBuddy 写入 `CFBundleName`/`CFBundleDisplayName`)后启动,窗口标题为 `Neo Aegisub`、打开面板后为 `Neo Aegisub · 样式管理`,菜单项为“关于 Neo Aegisub”“退出 Neo Aegisub”,About 对话框标题为“关于 Neo Aegisub”、版本页显示 `10076-ui-modern-wx-prototype-8ecbb0e1f`,系统“强制退出“Neo Aegisub””同步;配置与存档仍写入原 `Application Support/Aegisub`。Windows、Linux 的安装包与 `.desktop` 未在本机运行验证。

2026-11-02 频谱高度与轨道宽度解耦：用户报告频谱高度随音频容器宽度变化。结构上确有三处耦合：渲染请求把位图高度写死 260，绘制方框高度却跟随轨道；`TileKey` 不含高度，改高度后仍复用旧位图；图片层未钉高度，只靠 `ObjectFit::Fill` 跟着布局走。现在 `TileKey` 增加 `height`，`track_pixels(height)` 按轨道逻辑高度取整并钳到主机支持的 64..512，`refresh_audio` 用 `key.height` 作为请求高度；绘制时跳过高度不符的切片，并按切片自身高度 1:1 摆放，不再由布局或位图长宽比决定纵向尺寸；面板高度下限 60 提到 64，保证轨道不会比位图矮。macOS 实际窗口加载 `flicker-tone.wav`，1321/1100/1000/900 四种宽度、窗口高 700 下轨道恒为 160 设备行（80 逻辑像素），亮色低频带峰值都落在轨道高度的 0.975 处；把默认高度临时改成 200 复核，轨道变 400 设备行，位图最后一行正好落在轨道最后一行（下一行即页面底色），说明是整幅重渲染而不是按长宽比拉伸，复核后已恢复 80。旧的 1321 与 1100 两张截图里两宽度轨道同为 123 设备行、峰值同贴底，具体触发场景未复现，本条目只声明耦合已从结构上移除并给出上述测量。新增 `a_taller_track_is_a_different_slice`；`cargo fmt --check`、19 项 Rust 测试与 release 构建通过。拖拽改变面板高度这一手势本轮未能用合成鼠标事件复现，只用默认值改 200 复核了同一渲染路径；Windows 尚未运行验证。

2026-11-02 频谱绘制对齐容器高度：位图高度改由 `#audio-graph` 的实测高度决定，绘制循环也用它比对切片，不再用布局请求的面板高度，二者在窗口变矮、图形被压缩时并不相等，用请求值会让轨道永久空白。切片铺在容器自身矩形上：容器不低于 64 逻辑像素时 1:1 摆放，被压到 64 以下时整幅缩放到容器高度，避免只显示上半段频谱。实际窗口 1321×700/500 轨道恒为 160 设备行（80 逻辑），峰值在 0.975；1321×380 时容器压到 64 设备行、1321×340 时 56 设备行，低频亮带分别落在 0.968 与 0.982、底行颜色 `99f8a2` 与 `a0f9a9`，修复前同尺寸峰值停在 0.0、底行为 `040207`（低频被裁掉）；缩回 700 后恢复 160 行且无“正在生成音频画面”占位。首帧尚未布局时不再发渲染请求，等第一帧记录高度后由下一次快照补齐。

2026-11-02 频谱切片缓存的失效与复用：切片键含高度，因此改高度必然重渲染，位图不存在跨高度复用；缓存淘汰的 stale 判定也补上高度，否则旧高度的切片会占着 `TILE_CACHE_LIMIT` 48 个槽位却不参与绘制。每个高度一代通常 6 片（视口 4 片加左右各 1 片备用），48 个槽位可同时留住几代；平移不改键、直接复用，缩放改 `span` 才失效。调整窗口高度的瞬间，旧高度切片按容器矩形缩放绘制作为过渡，等匹配切片到达即替换，轨道不会在拖动期间变成空片或“正在生成音频画面”。实际窗口 1321 宽下 700→380→700 往返，三次截图轨道分别为 160、64、160 设备行，低频亮带 0.975、0.968、0.975，无占位文字；是否命中缓存没有直接观测手段，本节只给出键与淘汰规则。并发渲染仍受 `TILE_INFLIGHT_LIMIT` 3 限制。

2026-11-02 字幕列分隔线对齐：表头 7 px 悬停握把向右偏移 3 px，使其中心与 1 px 竖线重合；表头与表体共用 `column_rule`，上下各留白 8 px，表头保留灰色，表体使用淡主题强调色。macOS 真窗口 2× 截图验证：高亮位于 x=1584..1597，竖线位于 x=1590..1591，两侧各 6 device px；表体竖线同为 x=1590..1591。拖动后表头与表体同步移到 x=1627..1628，悬停高亮仍居中。采样表头为 `#4b505b`、表体为 `#4c526e`；Windows 尚未运行验证。

2026-11-02 音频图片随高度缩放：继续修复向上拖短波形区时频谱底部被裁掉的问题。`audio_image_layer` 移除自动高度外层，图片直接作为图区的绝对定位子元素，使用 `h_full()` 与 `ObjectFit::Fill` 跟随图区最终高度，保留原有切片横向位置与宽度。macOS 实际窗口加载 `flicker-tone.wav`，在 1321×700 窗口中拉高后再向上拖短，频谱底部亮色频带均完整保留；缩为 1321×500 后音频滚动条与控制栏仍可见。Release 构建通过；Windows 尚未运行验证。

2026-11-02 字幕文本操作按钮：文本编辑框正下方新增一排胶囊按钮——还原（`edit/revert`）、清除（`edit/clear`）、清除文本（`edit/clear/text`）、插入原文（`edit/insert_original`），沿用格式工具条的 ghost 小按钮样式，可用状态逐帧取自命令目录的 `can_invoke()`。macOS 实际窗口核验：`{\i1}测试文本{\i0}尾巴` 点“清除文本”后只剩标签 `{\i1}{\i0}`，“清除”清空整行；程序写回 `XYZ` 后“还原”回到空行；单行工程原文 `原文ABC` 改成 `ZZZ` 后“插入原文”得到 `原文ABCZZZ`。前端 release 构建、`fmt` 与 13 项 Rust 测试通过。每个按钮前加 Lucide SVG 图标：还原 `Undo2`、清除 `Eraser`、清除文本 `Delete`、插入原文 `TextCursorInput`（后三者登记进 `workspace_theme.rs` 的 `StudioIcons`）；真窗口像素扫描每个标签前各有约 20 device px 宽的图标块。

2026-11-02 音频标尺拖动与滚动条：波形上方补上时间标尺，下方补上横向滚动条，两者都只平移视图（`view_start`），不改游标。标尺语义对齐 wx `AudioDisplayTimeline::OnMouseEvent`（`src/audio_display.cpp:397`）：左键拖动时按 `-(Δx/宽度)*span` 反向平移时间轴；滚动条对齐 `AudioDisplayScrollbar`（`src/audio_display.cpp:132`），滑块宽度为 `视窗/总长`、不小于 28 px，轨道点击与滑块拖动按真实轴长映射。刻度步长取 `[50,100,200,500,1s,2s,5s,10s,15s,30s,1m,2m,5m,10m]` 中第一个使相邻刻线间距 ≥ 76 px 的值。标尺高 20 px：9.5 px 标签在顶部、5 px 刻度线贴底；刻线原先用 `bottom_0()` 完全不绘制（像素核对整行都是背景色），改 `top` 锚定后正常。macOS 实际窗口（20000 ms 音频，窗口 1321×888）：拖动标尺 `400,176 → 250,176` 后标签由 `0:00:00`–`0:00:10` 变为 `0:00:04`–`0:00:14`，5 s 蓝色标记由设备 x 794 移到 519；点击滚动条轨道 x=600 跳到 `0:00:08`–`0:00:18`，滑块由 368..994 移到 788..1414；再把滑块从 550 拖到 350 回到 `0:00:02`–`0:00:12`。`ruler_drag_scrolls_opposite_the_pointer_and_clamps` 与 `thumb_wraps_the_visible_window_and_keeps_a_grab_width` 两项单测随 13 项一起通过。滚动条初版只有 5 px、用 `p.muted`（`#282c36`）贴着 `#20232b` 卡片底，又被面板 `gap_3` 推到波形下方 12 px，实际无法辨认；现改为 16 px 条带贴住波形（零间距容器）、6 px 轨道 `ink` 16%（实测 `#3b3f47`）、8 px 滑块 `muted_foreground` 全不透明（实测 `#a1aabd`），复验点击 (600,284) 跳到 `0:00:08`–`0:00:18`、滑块 500→400 回到 `0:00:04`–`0:00:14`。

2026-11-02 分段控件滑动指示器与字幕表头收口：`page::segmented` 的选中胶囊改成一块会滑动的药丸——每个分段外层 `div().relative()` 在 `on_prepaint` 里把自身 `Bounds` 记进新增的 `SegmentBounds` 全局（键为 `(workspace 实体 id, 控件首段 id)`，因为 detach 到独立窗口的面板与控制本身共用同一批控件 id），药丸挂在首段 wrapper 内，`left`/`width` 由 `with_spring`（280 ms、0.85 阻尼，即 `SpringConfig::new(503.6, 38.15, 1.0)`）驱动，`segment_at` 把浮点索引插值成位置与宽度，越界会钳到首/末段；首帧还没有几何时退回原来那块整体 chip，读取时 `truncate(segments.len())` 防止控件少一段后滑向过期槽位。macOS 实际窗口（2x 截图，音频面板 波形/频谱）：波形选中时药丸占 x 380..465，点频谱后为 483..568，点击瞬间抓到的中间帧是 400..485，说明它在两端之间连续移动而不是跳变；`#a7b4ff` 选中文字只出现在药丸内（药丸区间 21 个像素，频谱区间 0 个）。两段等宽，宽度插值由新增的 `the_pill_travels_and_morphs_between_segments`（0 / 0.5 / 1 与越界钳制）覆盖。同轮收口字幕列表表头：列宽拖拽只由右缘 7 px 握把响应，`cursor_col_resize` 从整格移到握把并加 `muted.opacity(0.12)` 悬停底色，标签区不再触发；表头竖线由整高的 `muted.opacity(0.45)`（与正文 `subtitle_cell` 完全同色）改成上下各留 8 px 的短刻线 `muted.opacity(0.28)`。像素核对（同一 x=1590）：表头刻线 `#3f444e`、正文竖线 `#636e87`；从标签区拖 20 px 分界仍在 1590，从握把拖 20 px 分界移到 1627，握把实测占 1615..1628（7 逻辑 px）。14 项 Rust 测试与 `fmt --check` 通过。

2026-11-02 音频面板随高度收缩：窗口变矮时波形原先完全不缩——音频卡片、`graph+scrollbar` 包装层与图区都带 `flex_shrink_0`，左侧列本身也 `flex_shrink_0`，于是先压扁下方 `flex_1` 的预览，预览见底后卡片仍保持 296 逻辑 px 的自然高度并被父级 `overflow_hidden` 裁掉底部控件行。改为：图区 `h(px(height))` 加 `min_h(px(28.))` 并允许收缩；卡片 `panel` 与包装层补 `min_h_0()`；`title_row`、模式行、`controls`、`karaoke` 显式 `flex_shrink_0` 保持各自自然高度，缺口只由波形吸收；`main.rs` 左侧列的 `flex_shrink_0` 只在 `side_by_side` 时保留，非并排时允许纵向收缩。macOS 实际窗口（1321 宽，2x 截图，滚动条轨道顶端位置换算图高）：888/700/600/500 时图高仍为 80 px（预览先让位，符合预期），420 时 37 px、380 时 37 px、340 时 33 px；420 与 360 高度下 `波形/频谱`、`1×`、`音量 100%`、`自动/前进/跟随/吸附/卡拉OK` 各行仍在卡片内，不再被裁掉。手柄 `#audio-panel-resize-handle`（4 px、48×3 握把）拖动仍有效：从握把中心 (445,398) 下拖 60 px 后图高 80 → 121.5 px（滚动条轨道 475 → 548 设备行）。`cargo +1.95.0` release 构建、`fmt` 与 14 项 Rust 测试通过。

2026-11-02 音频切片缓存、预览掉帧与适应窗口：波形原先每次滚动/缩放都整幅重渲染并整图替换。改为按稳定网格切片（`tile_span` 取视口四分之一并向上取 1-2-5 步长），每片按 `(epoch, revision, mode, gain, span, index)` 缓存，`TILE_CACHE_LIMIT` 48 片、同时最多 `TILE_INFLIGHT_LIMIT` 3 个 `audio_render` 请求；`wanted_tiles` 取视口及两侧各一片并钳到媒体末尾，缺片时其余切片仍照常绘制，不再整幅空白；标尺/滚动条拖动期间即时补片。真窗口用 30 分钟 `long-30min.wav` 跳到 20 分钟，整幅无空列，测得 5 s 切片渲染约 12 ms、200 s 缩小切片约 11 ms。预览掉帧：`src/gpui_bridge.cpp` 的 JPEG 编码在缩小时改用 `wxIMAGE_QUALITY_BOX_AVERAGE`、质量 82→72，1080p30 视频真窗口播放 4 s 内帧号 140→200，约 30 fps，连续两张截图差异 1.3%。适应窗口无法关闭：原先 `media_changed || video_window_zoom 变化` 都会把 `fit_video` 置回 true，手动缩放后快照回报新缩放即被重置；现在只有换媒体才重置，缩放命令/滚轮显式关闭后保持关闭。真窗口验证：`video/zoom/200` 后 3 s 内两张截图零差异，点击“适应窗口”可来回切换且 3 s 稳定。已清除全部临时计时插桩。

2026-11-02 统一取色控件：所有取色入口改用设置页同一个 `gpui_component::ColorPicker`。新增 `gpui-prototype/src/colour_picker.rs`：`parse_hex_colour` / `format_hex_colour` / `format_hex_rgb` 在 `#rrggbb(aa)` 与 `Hsla` 间转换，aa 按 ASS 透明度字节处理（`00` 不透明，写入时取反）；`LinkedPicker` 把取色器与原文本框双向绑定（取色→写文本框并按键入同样标脏，键入合法颜色→取色器跟随），按字节比较避免浮点漂移被当成修改。样式管理器 4 个颜色字段（`STYLE_COLORS`）和空白视频“颜色”改为取色器加原文本框；覆盖标签颜色对话框删掉自带的十六进制/透明度输入和 10 个预设色块，改用同一个取色器，主机最近颜色作为 `featured_colors`，应用时发送 `#RRGGBBAA`；设置页私有解析函数移到共用模块。真窗口验证：样式管理器选色后文本框变 `#713f1200` 并提示“有未应用修改”，应用后主机规范化为 `#713F1200`；覆盖标签对话框（格式工具栏调色板→主要色）选 `#06B6D4`、应用后第 2 行文本为 `{\c&HD4B606&}第1行 测试`；空白视频选 `#06b6d4` 后文本框同步，载入后预览采样 `#07b6d5`（视频色彩转换 ±1）。键入文本→取色器跟随仅由 `LinkedPicker` 逻辑覆盖，未单独在真窗口演示；次色/描边/阴影三个样式字段与主色同一代码路径，未逐一点击。

2026-11-03 视频播放呈现节拍：此前每帧解码完成即上屏，到达抖动直接变成画面间隔抖动。改为呈现时钟（`gpui-prototype/src/main.rs` 的 `PresentClock`、`QueuedFrame`、`queue_playback_frame`、`present_due_frames`、`playback_period`、`playback_due`）：每帧带槽位时间，`base + PLAYBACK_PRESENT_LAG`（50 ms）加上帧号差乘周期；周期为 `1 / (fps × playback_rate)`，`fps` 未知时按 30 fps，倍率非正或非有限时按 1x。计时器到点才上屏，到点的多帧只显示最新一张、其余释放；队列上限 `PLAYBACK_QUEUE_LIMIT` 8 帧，超出释放最旧帧。以下情况重新锚定并清空队列：帧号未按队列顺序递增（后退 seek）、帧号低于锚点、周期变化（倍率或 fps 改变）、槽位落后或超前超过 `PLAYBACK_RESYNC_PERIODS` 4 个周期（前跳 seek 属于超前）；同一帧重复送达只保留队列中的副本；播放停止时新帧直接上屏并清空队列。真窗口 `frame-test-1080p30.mp4` 1x 连续播放 6 s（临时插桩，验证后已删除）：修复前按到达上屏的相邻间隔 p50 32.7 / p95 41.2 / p99 50.9 / max 57.2 ms；修复后按槽位上屏 p50 33.4 / p95 35.7 / p99 36.4 / max 37.1 ms，167 个间隔的帧号步长全为 1；上屏相对槽位晚 p50 1.0 / max 5.1 ms；到达早于槽位 35.7–61.3 ms（p50 50.3），本次无帧晚于槽位到达。2x 复现与修复：修复前（周期未乘倍率、重锚只处理落后）2x 播放 6 s 内只上屏 6 帧（帧号 2→9），队列积压 241 帧，槽位被推到约 5 s 之后（queued `due_ms` 最大 4992）。修复后 2x 连续 3 s 上屏 159 次，帧号步长 1 出现 141 次、2 出现 16 次、4 出现 1 次，即约 11% 的帧被丢弃（到点时只显示最新帧），`late_ms` p50 5.9 / max 8.3；无截图的宿主端 1 s 窗口计数为播放帧号 59.3–60.9 帧/s。1x 播放中前跳 60→120：上屏由 60 直接跳到 120，之后逐帧递增；1x 播放中后跳 89→30：上屏由 89 直接转为 30，之后严格递增到 102，无旧位置帧残留。1x 普通播放回归：6 s 内上屏 174 次，帧号步长全为 1，`late_ms` p50 2.8 / max 11.3。代价：相对宿主选定的帧时刻固定多约 50 ms 显示延迟，未用音频时钟实测音画偏移。以上只证明 GPUI 侧呈现间隔均匀且 2x 不再冻结；真窗口里是否仍然卡顿、2x 约 11% 丢帧是否可接受，需用户目视确认。JPEG 质量 72 / BOX_AVERAGE 的取舍仍待决定。`cargo +1.95.0 fmt`、release 构建与 19 项 Rust 测试通过（最终代码，探针已删除）。

2026-11-03 视频播放改为显示刷新驱动（替换上一条的计时器上屏）：计时器在 macOS 上仍按醒来相位落在刷新之间，1x 下形成 33/50 ms 交替。现在播放期间每次显示刷新由 `window.on_next_frame` 调用一次 `present_tick`（`gpui-prototype/src/main.rs` 的 `present_next_refresh` 自注册，`present_due_frames` 挑选槽位已到的最新帧上屏）；`present_loop_armed` 保证只有一条回调链，`render` 在播放中补注册，停止播放时回调链结束并把队列里最新帧直接上屏。修复一处预先存在的丢帧：`start_frame_pump` 每个会话从 `sequence=0` 编号，而 `FramePump.applied` 跨会话保留，新会话的回复被 `applied >= sequence_now` 丢弃，直到序号超过上一会话的值；现在新会话从 `applied + 1` 继续编号。真窗口，`frame-test-1080p30.mp4`，1x 播放 5.9 s（临时插桩，已删除）：刷新回调间隔 p50 16.67 / p95 18.44 ms（与 60 Hz 刷新同步）；上屏间隔 p50 33.34 / p95 35.23 / max 35.40 ms，全部在 30–37 ms 内，173 个相邻上屏帧的帧号步长全为 1。同一进程连续 1x、2x、1x 三段播放全部上屏；2x 段 3 s 上屏 133 帧，帧号步长 1×109、2×21、3×2，上屏间隔 p50 16.76 ms。修复前：1x 段之后 2x 段 1.8 s 内 0 帧上屏（队列恒为 0，帧号卡在 180，宿主已到 298）。未解决：1080p60（`build-ui-gpui/frame-test-1080p60.mp4`，ffmpeg 生成，60 fps，6 s）真窗口 1x 播放 3.8 s 只上屏 21 帧（约 5.5/s），帧号步长 3 至 23，上屏间隔 p50 200 ms；宿主进程 CPU 60–95%，前端约 13%；同一文件裸 ffmpeg 解码约 0.26 s。瓶颈在宿主请求/解码路径（播放期间 `QueueFrame` 不主动请求，依赖 `VideoController::OnPlayTimer` 每 10 ms 调用 `RequestFrame`；`AsyncVideoProvider::ProcAsync` 丢弃被新版本覆盖的请求；FFMS 逐帧 `FFMS_GetFrame`），尚未逐段插桩定位，宿主代码未改动。视觉平滑仍需用户在真实播放中确认。

2026-11-03 Windows 便携版不再随包发布 `config.json`：此前 `packages/win_installer/portable/config.json`（内容仅 `{"App": {"Local Config": true}}`，代码不读取该键）被 `packages/win_installer/portable/create-portable.ps1` 复制进包根目录。该文件会在解压新包覆盖旧目录时替换用户原有设置（推断机制，未在 Windows 上复现）。改动：删除该文件与复制步骤（`stepTotal` 10→9，与 `Write-Step` 调用数一致）；`src/main.cpp` 的 Windows 分支（`#ifdef __WXMSW__`）在 `?data/config.json` 与 `?data/installer_config.json` 都不存在时，用默认配置以 `agi::Options::FLUSH_SKIP` 生成 `config.json`（安装版由 Inno 安装器 `ssPostInstall` 写入 `installer_config.json`，因此保持 `%APPDATA%` 位置）；`tools/test-gpui-windows.ps1` 不再复制包内配置，改为写入只含烟测所需键的 `config.json`。未验证：Windows 编译与运行、覆盖解压后的设置行为；macOS 无法编译 `#ifdef __WXMSW__` 代码。已发布的旧 zip 仍含该文件，解压覆盖时仍会替换设置，本改动只对新包生效。

2026-11-03 预览帧不再为中宽面板重采样，桥接响应读到换行即止：宿主原来只要请求宽度小于视频显示宽度就整幅重采样，1080p 面板请求 1529 宽时 `wxImage::Rescale` 实测 5.7–8.2 ms、JPEG 1.5 ms，而按原尺寸只编码只需 2.3 ms；`src/gpui_bridge.cpp` 的 `to_base64_jpeg` 现在只在请求宽度小于显示宽度的 0.7 倍时才缩小（`kMinShrink`），其余交给前端 GPU 缩放，行为与原版 wx `video_out_gl.cpp` 上原生纹理 + `GL_LINEAR` 一致；缩小与放大统一走一个 `resample` lambda，任何缩小都用 `BOX_AVERAGE`，顺带修掉 else 分支缩小用 `BILINEAR` 的锯齿。前端 `Bridge::call` 改为读到换行就返回，不再 `read_to_end` 等对端关闭握手，每帧省下约 0.5–1 ms。1080p60 实测（`build-ui-gpui/frame-test-1080p60.mp4`，帧间隔探针已随后删除）：上屏 55.5–59.0 次/s，宿主编码 2.9 ms、`rescale` 0、往返 p50 16.7 ms；1080p30 上屏 30.2 次/s、180 个帧号步长全为 1。60 fps 内容仍有约 5% 的单个 33 ms 间隔：往返恰好等于一帧周期，宿主按墙钟解码是节拍源，双请求流水线实测反而因请求超前于解码时钟退化到 50.7 次/s，已回退。Windows 尚未运行验证。

2026-11-03 配置解析失败先留副本：`agi::Options::LoadConfig` 原先遇到 JSON 解析异常只写日志并返回，内存里仍是从默认配置载入的值，之后任何一次 `Flush` 都会把整份默认值写回 `config.json`，用户的原始文件被静默覆盖。现在 `LoadConfig` 返回 `bool`（`libaegisub/include/libaegisub/option.h`），两处解析异常 `return false`；`Options::ConfigUser()`（`libaegisub/common/option.cpp`）在返回 false 时调用 `backup_unreadable_config`，把原文件复制成同目录的 `<config>.invalid` 并写一条 `LOG_E("option/load")`，复制失败只记日志、不阻止启动。回归测试 `unreadable_config_is_kept_next_to_the_original`（`tests/tests/option.cpp`）写一份截断 JSON，构造 `agi::Options` 后调用 `ConfigUser()`，断言 `.invalid` 存在且与损坏文件逐字节一致；`lagi_option.*` 20 项通过。覆盖范围仅限启动时读取用户配置：`ConfigNext`（安装版 `installer_config.json`）仍忽略返回值，损坏时不备份。

2026-10-27 打包修复：ARM64 Lua 模块失败来自 BOM 与有符号 `char` 比较，改为无符号字节判断；本机用 `-funsigned-char` 编译真实读取器加载 `aegisub/unicode.moon` 通过，新增回归后 381 项 C++ 测试通过。未签名 Intel 前端的嵌套 `.app` 签名回归通过。隔离 macOS 预览确认默认自动打开 GPUI、宿主隐藏、关闭 GPUI 后两个进程退出。Windows portable/installer 已修正遗漏前端的问题，最终产物仍需以新 CI 归档内容验证；不以此本机检查宣称 Windows/Linux 桌面已通过。

2026-11-03 播放取帧改为宿主缓存即答：用户的 OBS 逐帧证据（Windows 便携版 10083）显示原版播 30 fps 时每两次 60 Hz 刷新换一帧（`101010…`），而 Neo 呈 `10100010000101…`。原因是取帧链路一次只有一个请求在途：`QueueFrame` 在播放中不主动请求解码，只等 `OnFrameReady` 的下一帧，所以每次往返至少一个帧周期（同机实测约 30 ms），前端拿到的帧总带着宿主墙钟解码与请求相位之间的漂移，呈现时钟的槽位随之偏移，偶尔触发重新锚定。改动两处。宿主 `src/gpui_bridge.cpp`：新增 `CachedFrame`（`jpeg`/`frame`/`width`/`height`/`max_width`/`epoch`/`valid`），`OnFrameReady` 编码后留一份，播放中的 `frame` 请求在 `max_width` 与 `epoch` 一致时直接从缓存拼回复（不再走 `quote_json`）；无人等待的帧也刷新缓存，但仅在最近 1 s 内收到过播放取帧请求时刷新（`last_playback_poll`），避免没人看时逐帧空转编码。前端 `gpui-prototype/src/main.rs`：`FramePump` 增加 `period`（微秒）与 `delivered`，泵按半个帧周期轮询而不是每回复一次；同一帧号不重复解码；请求放弃时间从 40 ms 改为 250 ms（每 20 ms 检查一次，只在播放停止或宿主无响应时结束），不再把"晚于一个帧周期"的回复丢掉；队列为空且该帧已在屏时不重复上屏。实测 macOS 真窗口 `{"op":"frame"}` 往返 p50 1.28 / p90 2.87 / max 41.2 ms（改动前同类测量约 30 ms，即一个帧周期）；`frame-test-1080p30.mp4` 播放 5 s 画面变化 146 次（29.2 次/s），停留 p50 33.3 / p90 34.2 / p99 65.4 / max 66.4 ms，145 个间隔中 2 个超过 45 ms、帧号步长除 2 次外全为 1；`frame-test-1080p60.mp4` 停留 p50 16.7 ms、上屏 51 次/s，60 fps 的丢帧仍由宿主解码节拍决定，本次未改。窗口失去焦点时 gpui 自身把刷新限到 30 fps（`gpui-pre-0.3.6/src/window.rs` 的 `inactive_frame_interval`），被遮挡的测量会掉到 5 次/s，不能作为回归依据。前端 19 项 Rust 测试、`cargo fmt` 与宿主增量构建通过；Windows 未在本机验证。

2026-11-03 播放计时开关：Windows 便携版 10084 仍报告卡顿，而本机无法复现该环境，因此把播放链路的关键时刻做成可开关的日志，用用户机器上的数据代替推测。前端 `gpui-prototype/src/trace.rs` 在设置 `AEGISUB_GPUI_TRACE=<路径>` 时按行写入微秒时间戳，记录 `tick`（每次显示刷新）、`req`/`resp`（取帧请求与往返）、`same`（宿主重复帧）、`dec`（base64 解码加通道交换耗时）、`show`（上屏帧号、相对槽位的迟到量与队列深度）；宿主 `src/gpui_bridge.cpp` 的 `HostTrace` 在 `AEGISUB_GPUI_HOST_TRACE=<路径>` 时记录 `ready`（解码帧间隔）、`encode`（重采样加 JPEG 的耗时与字节数）、`hit`（缓存即答的轮询间隔）。两者都只在环境变量存在时打开文件，未设置时不写盘也不取时间。便携包新增 `packages/win_installer/portable/trace-play.bat` 一键带变量启动，`create-portable.ps1` 增加对应复制步骤（`stepTotal` 9→10）。macOS 真窗口验证：1080p30 片段播放 4 s 得到前端 `tick` 240 条、`resp` 140 条、`dec` 19 条、`show` 18 条，宿主 `ready` 23 条、`encode` 21 条、`hit` 141 条，字段与预期一致；该机当时负载 load average 9.9（10 核），绝对数值不作性能结论。19 项 Rust 测试与 release 构建通过；Windows 上的日志尚待用户回传。

2026-11-04 播放进度条移入视频画面并可拖动预览：进度条原来占控制条一整行，现在由 `gpui-prototype/src/video_timeline.rs` 的 `scrubber()` 绝对定位在 `video-frame-surface` 底边（16 px 高，3 px 轨道，8 px 圆点，`pb(px(4.))` 让圆点不被 surface 的 `overflow_hidden` 裁掉），控制条里原 `Slider` 删除。拖动时第一个 `SliderEvent::Change` 置 `resume_after_drag` 并发 `video/stop`，`Release` 后由 `flush_video_seek` 恢复播放；一次只允许一个 seek 在途，拖动中 `pending_frame` 被覆盖为最新位置，seek 完成时若仍有更新位置就不再为中途帧请求画面，避免拖过多少帧就解码多少次。修掉两个使"拖动只动进度条不动画面"的缺陷：`refresh_frame` 的 `frame_request_inflight` 在成功分支从不回落，第一次静态帧之后所有 seek 都被去重挡住（`gpui-prototype/src/main.rs:6285` 起现在每次回复都清 inflight，过期回复按代次丢弃后立刻重新请求）；宿主 `src/gpui_bridge.cpp` 新增 `last_decoded`/`last_decoded_frame`/`last_decoded_epoch`，seek 时控制器 `JumpToFrame` 自己解码的那一帧被前端紧随的 `frame` 请求直接复用编码（实测 2.9–3.4 ms），省掉第二次 seek+decode，commit 监听器里清空以防烧进画面的字幕过期。本机真窗口验证：seek 到 120/400/700 后视频区像素哈希各不相同（此前恒为同一值），播放中 30 fps 与暂停/恢复正常，进度条填充率 15.3/34.6/54.0/73.1/88.6% 与帧号线性一致；Rust 19 项测试、Meson 两组测试通过。未验证：本机 `AXIsProcessTrusted()` 为 false，无法注入真实鼠标拖拽，拖动跨多帧时的手感只能由 Windows 真机复测。

2026-11-04 播放进度条骑在读出行与 transport 块的接缝上并带悬停动画：用户要求进度条放在「帧 … · 1.78:1 · 29.970 fps / ⌘/Ctrl 滚轮缩放 · Shift 平移」与时间码行之间，且悬停变粗要有过渡。`gpui-prototype/src/main.rs` 的读出行改为平铺 `h_flex`，分隔线挪到它自己的 `border_b_1`（`video-frame-surface` 底边那条删除）；`gpui-prototype/src/video_timeline.rs` 的 `scrubber()` 改成零高度 band（`.h(px(0.)).flex().items_center()`）作为 transport 块（`#video-playback-controls`，去掉 `border_t_1`/`px_3`/`py_2`）的第一个子元素，band 里 16 px 命中条上下各溢出 8 px，条心正好压在两个容器的边界上；时间码行与按钮行整体下移进内层 `px_3 py_2 gap_2`。厚薄由 `SCRUB_SPRING = SpringConfig::new(650., 40., 1.)` 经 `with_spring` 插值：薄 3 px 轨道 / 8 px 圆点，悬停或拖动时 6 px / 11 px，约十分之一秒收敛、不过冲。`BaseSlider` 只保留拖动交互（`SliderTrack` 不画），轨道与圆点自绘——`SliderIndicator` 包进 `with_spring` 后只画出 1~2 px 碎片，是改成自绘的直接原因；`BaseSlider` 不加 `w_full()` 时在 flex 行里宽度为 0，整条不可见。轨道底色用不透明的 `p.border`：半透明白条跨两种底色会双色，容器 `border_t_1` 又会画在子元素之上把条切断。本机真窗口像素验证（2x 截图，window y = 图 y/2 − 28）：条占设备行 1556..1561（3 逻辑 px），divider 在 1557..1558，条心 1558.5 与 divider 中心 1557.5 对齐；横向 355..1428，等于卡片宽减去左右各 5 px；frame 225 时圆点中心 1165，`(1165-355)/1073 = 0.755` 与 `225/298` 一致；把弹簧临时调软到 `(30., 12., 1.)` 后连拍得到条高 6 → 8 → 12 设备行，证明变粗是插值而非跳变。播放 30 fps 与 Rust 19 项测试照旧通过；本机 `AXIsProcessTrusted()` 为 false，合成指针事件被系统丢弃，悬停手感仍需 Windows 真机验收。

同一轮修掉裸 `BaseSlider` 的点击退化：`SliderState` 的取值边界只由 `SliderIndicator` 在 prepaint 时写回（`gpui-base-0.6.6/src/slider.rs:681-683`），自绘轨道把这些指示器删掉后 bounds 一直是零，`update_value_by_position` 里 `inner_pos / total_size` 变成 `0/0 = NaN`，于是任何一次点击都把进度算成 NaN——Rust 里 `NaN as i64` 取 0，视频跳回第 0 帧，填充宽度和圆点也因 NaN 一并消失（用户实测「点了一下进度条后画面变黑了进度条也没了」）。现在 `scrubber()` 里保留一个透明、无命中框的 `SliderIndicator`，绝对定位到 `left(px(5.)) right(px(5.)) top_0 bottom_0`，正好等于轨道的取值区间；临时打印验证到 bounds 从 `0px × 0px` 变成 `536.5px × 16px`、seek 150/299 后 `pct = 0.0..0.50167227`，填充实测 356..899 设备行（≈50.2%）加上 8 px 圆点，与本机 300 帧素材一致。

2026-11-04 拖动预览跟手，进度条向上加粗并改用游标针：用户报告拖动进度条时画面延迟约 500 ms。链路实测（本机 1080p30、300 帧）单次 `seek` 0.5–49.7 ms，随后 `frame` 35.9–185.4 ms（p50 66.4）；连续 60 次 seek 之后取最终帧要 104.5 ms，而顺序「seek→等帧→下一帧」p50 只有 13.6 ms——说明每次指针事件都在打断上一次解码并从关键帧重来，而不是编码或传输慢。三处改动。前端 `gpui-prototype/src/main.rs`：`flush_video_seek` 的门控加上 `frame_request_inflight`，一次只跑一个 seek 加取帧的循环，最新位置覆盖留在 `pending_frame`；新增 `seek_target: Option<i64>`，因为 seed 之后宿主快照还没回推，取帧请求必须自带目标帧，`refresh_frame` 用 `seek_target.unwrap_or(snapshot.frame)` 作为请求帧，成功、失败、过期、无图、开始播放五个分支都清掉它。宿主 `src/gpui_bridge.cpp`：`Seek()` 的 `JumpToFrame` 本身已经 `RequestFrame`，紧接着前端的 `frame` 请求在 `QueueFrame` 里又请求一次，provider 版本号自增把正在解码的任务取消——新增 `pending_frame_request`/`pending_frame_request_at`（`pending_request_window` 500 ms），`Seek()` 记录目标，`QueueFrame` 只在没有在途请求时才请求，`OnFramePoll` 的重发改走 `RequestProviderFrame`，`OnFrameReady` 命中即清，commit 监听器、`Stop()` 与播放分支也清。实测脚本化拖动 90 步 × 16 ms 扫过 239 帧：旧行为（每次请求都打断）上屏 38.2 次/s、画面延迟 p50 18 ms 但每步都要从关键帧重解，在慢解码器上退化成用户看到的 500 ms；只加前端门控 44.0 次/s、p50/p90/max 21/53/69 ms；门控加宿主去重 53.5 次/s、26/40/57 ms、显示帧落后指针 p50 2 帧、p90 3 帧。视觉上 `gpui-prototype/src/video_timeline.rs` 的悬停加粗改为底边固定在接缝、只向上长（`const BASE: f32 = 9.5;` 轨道 `top(px(BASE - track))`，实测薄态 1556..1561、厚态 1550..1561 设备行），圆形圆点换成 2 px 宽 10 px 高的竖直游标针（与填充同色 `p.accent`，实测设备列 1024..1027、行 1550..1567，上下各露出 3 逻辑 px）。

2026-11-04 进度条只在外侧两端保留圆角：填充块原来 `rounded_full()`，靠近游标针的一侧也带圆角，与针之间露出两个缺口。改为左端仍按 `track / 2` 收圆、右侧直角，只在 `end > 0.995`（填充已顶到轨道末端，它自己就是条子的右端）时才把右侧圆角加回来。像素验证（设备坐标，2x 截图）：薄态 3 px 轨道在 x=356/357 只有中段 4 行，是左端的圆角；填充体 6 行齐平；填充右缘列（如 frame 149 的 887）满 6 行，与后面 4 列宽的游标针直接相接，没有收圆；frame 299 时 x=1425 收成 4 行，右端圆角正确。

2026-11-04 音频波形标题行显示正在对轴的那一行原文：`gpui-prototype/src/audio_panel.rs` 的 `render_compact` 多一个 `line_text: &str` 参数，`title_row` 在「音频波形」标题之后插入该文本，样式与编辑器「显示原始字幕」那一行一致（`text_sm()`、`Palette::current(cx).muted`、单行 `truncate()` 省略），时间范围提示 `start → end · duration` 留在右侧且不再吃满剩余宽度。文本由 `gpui-prototype/src/main.rs` 的 `timed_line_text()` 取自 `snapshot.active` 指向那一行的 `text`——是**带 `{…}` override 标签的原文**，不是渲染后的对白，也不按时间过滤，因为对轴时关心的是正在编辑的这一行。先前尝试并否掉的两版：视频预览右上角浮层、波形区域右上角卡片浮层（用户要求改为标题行 + 原文样式），随之删掉只服务于浮层的 `preview_text()` 及其单元测试。OCR 验证（2x 截图）：标题行为「音频波形 {\pos(960,540)}Overlay\Nprobe line… 0:00:00.20 → 0:00:02.00 · 0:00:20.00」，标签原样保留、超长部分省略、提示仍在最右。

2026-11-04 字幕列表按住拖选与活动行贴顶跟随：`gpui-prototype/src/main.rs` 的列表行新增 `on_mouse_move`——按下时记录 `grid_drag_anchor`/`grid_drag_row` 并置 `follow_suppressed`，光标移到别的行就按锚点调 `select_range(anchor, id)` 画一段连续区间（`{"op":"select","id":target,"ids":[…]}`），同一行内移动不再发请求；`grid_drag_anchor` 在 `on_mouse_move` 见到左键已松开时清空，列表视口的 `capture_any_mouse_up` 兜住释放在行间留白的情况。原先 `select_lines(extend)` 内联的 Shift 区间计算抽成 `select_range()`，两处共用。跟随一侧在 `apply_snapshot()` 里 `self.selected_line = selected;` 之后：活动行变了、且不是指针手势（`!follow_suppressed`）时，用与列表同一套过滤谓词 `matches_subtitle_filter()`（从渲染时的内联闭包抽出）算出该行在过滤后列表中的序号，`list_scroll_handle.scroll_to_item_strict(index, ScrollStrategy::Top)` 把它钉在顶部，随后无条件复位 `follow_suppressed`。验证走的是真实事件路径：临时加了 `AEGISUB_GPUI_DEBUG_DRAG` 环境变量驱动的 `debug_drag()`（`Window::dispatch_event` 发 MouseDown/16 次 MouseMove/MouseUp，验证后整段删除并重建）：120 行探针脚本上从第 2 行拖到第 5 行，`snapshot.selected` 得到 `[3,4,5,6]`、active 落在末行，列表没有跟着滚；宿主侧 `select` 到第 40 行后截图 OCR 显示顶行即该行（`039/0`，id 对应行号减一），说明贴顶跟随生效。系统层合成鼠标要 `AXIsProcessTrusted`，`swift`/`osascript` 均未获授权（`AXIsProcessTrusted: false`、System Events `click at` 返回 `missing value` 且不产生事件），因此改走应用内分发。

2026-11-04 波形单击不再误建零长选区：GPUI 在按住鼠标时可能收到坐标未变的移动事件，旧处理无条件设置 `drag_moved`，从而把宿主返回的结束标记拖到开始位置。现在按下时记录横坐标，移动达到 3 逻辑像素后才进入拖动。真实窗口事件验证：同坐标按下、移动、抬起，修复前为 2665–2665 ms，修复后为 2665–5000 ms；实际拖动仍能移动标记，右键仍只调整结束时间。

2026-11-04 波形缓存满后不再持续加载：48 张图片的缓存淘汰顺序修正为先淘汰过期参数的图片，再按距离选择；原先布尔值取反，刚生成的有效图片反而会被立即丢弃并反复请求。真实窗口连续改变增益填满缓存后删除单行，修复前一直显示「正在生成音频画面…」且有 3 个请求在途；修复后波形与时间标记正常，缓存仍为 48 张、在途请求归零。

2026-11-05 视频焦点支持空格播放／暂停：Video 快捷键上下文缺少默认 Space 绑定；GPUI 在用户自定义快捷键未命中时补上 `video/play`，文本输入与已有绑定优先级不变。真实窗口点击视频后发送两次空格，第一次观察到 playing=true、frame=17，第二次 playing=false、frame=18。

2026-11-05 字幕列宽保存到 `Subtitle/Grid/GPUI Column Widths`：拖动结束后由宿主备份并刷新用户配置，新窗口首次快照恢复列宽。真实窗口拖动时间列后配置值为 220.67337，重启宿主与前端后仍为该值，截图确认列分隔线位置恢复。

2026-11-05 点击视频预览标题打开独立 GPUI 视频窗口，主窗口切到音频＋字幕布局；独立窗口按窗口高度分配预览区域。真实窗口点击验证已得到独立「Neo Aegisub · 视频预览」和只保留音频、编辑器、字幕列表的主窗口。

2026-11-05 播放画面延迟由 50 ms 降到 33 ms：在 30 fps 素材上扫描 `PLAYBACK_PRESENT_LAG`，0 ms 的显示间隔标准差 82 ms，16 ms 仍有 29/104 个间隔偏离，25 ms 有 2 个，33 ms（一帧周期）为 0 个、标准差 0.7 ms；画面相对音频的偏移从约 50–90 ms 降到约 −31 ms。仍未用音频时钟反馈校正，VFR 素材继续按平均帧率排期。

2026-11-05 双击字幕行跳转视频改为选项 `Video/Seek On Double Click`（默认开启）：宿主 `select` 带 `double_click` 时才跳到行起始时间，开关放在视频工具栏“双击跳转”，同时出现在偏好设置的视频页。

2026-11-05 视觉排版拖动减少重复工作：拖动中不刷新命令目录、不排入额外绘制请求；静态取帧返回时若画面修订仍落后，则重新请求最新画面。每次拖动仍会读取完整项目快照。真窗口拖动 `\pos` 控制点 40 步，画面修订逐步跟上文档修订，队列为 0，文本变为 `\pos(1298.7,709.41)`；没有修复前后的性能基准，Windows 掉帧仍待验收。

2026-11-05 字幕列表右键菜单复用宿主 `grid_context` 定义和命令校验：右键已选行保留多选，右键未选行切换到该行，选中状态同步后再生成菜单。真窗口操作验证单行重复（120→121 行）、多选连接（120→119 行）、删除后活动行和波形范围更新，音频图片请求归零。

2026-11-05 视频工具栏增加“循环播放”，对应 `Video/Loop Playback`（默认关闭）：整段播放到结尾后从头重播，播放当前行时循环该行，手动停止不触发重播。真窗口按钮切换后，10 秒视频从第 290 帧开始播放能越过结尾回到第 20 帧继续播放；当前行循环保持在第 120–149 帧，手动停止保持暂停，关闭循环后正常结束。

2026-11-05 独立视频窗口关闭时恢复主窗口弹出前的音频/视频可见状态。原生菜单命令统一由主工作区处理，工具窗口不覆盖主布局的菜单勾选；弹出期间切换显示模式会关闭独立视频窗并应用选择。真窗口关闭验证恢复音频＋视频；弹出期间选择视频＋字幕、随后切换纯字幕、音频＋字幕、完整模式均与菜单选择一致。

2026-11-05 最近打开子菜单的文件名使用可收缩单元格（`flex_1().min_w_0().truncate()`），超出菜单宽度（上限 500px）时末尾显示省略号；其他菜单项不变。已编译通过，尚未在真实窗口验证（无菜单驱动），Windows 待验收。

2026-11-05 音画时钟与隐藏视频取帧：OpenAL 播放位置改用 `AL_SAMPLE_OFFSET` 加已出队缓冲长度；该偏移已包含队列内已处理缓冲，不重复计数，也不再用秒表估算。视频播放跟随音频源时间，音频停止后从最近锚点继续。帧回复增加 `time` 与 `clock_ms`，GPUI 按媒体时间调度，移除收到解码帧后固定等待 33 ms 的调度。隐藏视频的工作区停止取帧与持续呈现回调，避免独立视频窗与主窗重复工作。实际运行采样 180 次，音频位置无回退，视频帧时间减音频位置为 -45 至 0 ms（包含帧取整与定时器间隔，不代表声学同步测量）。前端日志记录 288 次上屏，间隔中位数 33.30 ms、最大 66.61 ms，287 个间隔中仍有 41 个超过 50 ms；Windows 持续卡顿未在本机复现，不能据此判定根治。视觉工具实际拖动 20 步后文本由 `\pos(960,540)` 改为 `\pos(1080,580)`，随后撤销恢复。解码器已有源帧缓存，因此未叠加额外缓存或整帧复制。宿主与前端 release 构建通过，Rust 19 项测试与 Meson 两组测试通过。

2026-11-05 循环播放按钮缺失图标：`IconName::Repeat` 已被按钮引用，但不在 `StudioIcons` 的显式嵌入列表或默认资源列表内。将 `Repeat` 加入应用图标资源列表，随二进制打包。release 构建通过，重启应用并打开实际视频成功；当前截图捕获失败，尚未完成视觉确认。未单独推送触发 CI。

2026-11-05 视频显示切换到独立原生 OpenGL 窗口：GPUI 仅保留播放、进度、循环、缩放与视觉工具选择，主工作区改为紧凑视频控制条、音频与全宽字幕列表。新增 `video_surface` 控制接口；宿主创建仅含 `VideoDisplay` 画布的原生窗口，不创建旧版菜单或工具栏。解码与 libass 合成后的帧通过 `EVT_FRAME_READY` 直接上传 `VideoOutGL` 并交换缓冲，删除桥接 JPEG/Base64 视频帧传输、GPUI 帧轮询/解码/呈现队列以及 JSON 视觉覆盖层适配器。隐藏的旧画布不处理帧上传，关闭原生窗口后恢复上下文指针；既有快照报告窗口可见状态。视觉工具使用原生画布交互，保留 GPUI 的 `pos ↔ move` 转换与矢量裁剪子工具。便携版 `trace-play.bat` 改为仅采集原生 OpenGL 帧日志。macOS 用用户提供的实际 MP4 验证原生帧上传与播放、视觉工具选择、缩放、隐藏/重新打开、转换标签与撤销；初次采样 112 次画面推进的帧号全部连续，绘制耗时中位数 1.54 ms。截图权限不可用，未完成截图目视验收；Windows 流畅度与窗口行为仍需该平台运行验证，不能将本机数据作为 Windows 修复结论。

2026-11-05 音频显示改为原始像素块：保留既有音频 provider、`AudioRenderer` 及波形/频谱缓存，`audio_render` 返回一行 JSON 元数据与紧随其后的 BGRA8 二进制像素，不再编码 PNG、Base64 或在 GPUI 中解码图片。GPUI 直接包装 `RenderImage` 并缓存纹理，音频与字幕继续处于同一主窗口。缓存键补齐渲染宽度；移除 120 ms 缩放等待，最多三个并发请求优先填补可见区域，淘汰或替换时释放对应 GPU 图片。选区和播放游标继续独立绘制。

macOS 用用户提供的实际 MP4 对比同一时间片，波形和频谱输出在通道顺序归一后与旧 PNG 解码像素逐字节一致。320×100、两秒切片各采样十二次，频谱完整请求耗时中位数由 6.98 ms 降到 2.08 ms，波形由 1.76 ms 到 1.74 ms；这是本机切片请求时间，不代表跨平台主观延迟。真实 GPUI 窗口经临时事件驱动验证十二次滚轮缩放、三十次标尺拖动、波形切换到频谱、单击与右键拖动收尾；缓存达到 48 块后请求仍归零，无生成失败状态。另验证 4 MiB 像素块分段读取与传输中断后宿主继续响应。临时驱动和入口已移除；截图捕获不可用，尚无截图目视验收，Windows 交互流畅度仍待实际运行验证。

2026-11-05 原生视频画布改为 Windows 跨进程内嵌（已撤回；当前代码不存在 `SetParent`/父 HWND 挂载路径，视频统一走 GPUI 纹理）：GPUI 在布局后发送父 HWND 与物理像素区域，宿主将无边框 OpenGL 子窗口挂载到该区域；倍率仅调整画布内容，不再调整父窗口大小。主工作区恢复受限宽度的视频预览与字幕编辑器并排布局，音频和字幕列表保留可用高度。切页、宿主对话框及视频倍率菜单覆盖时隐藏子窗口；父窗口失效后清理原生句柄并恢复视频显示上下文。macOS/Linux 尚无跨进程内嵌实现，明确拒绝挂载请求，不自动弹出独立视频窗。

验证：macOS release 构建、Rust 12 项测试及 Meson 两组测试通过；Windows GNU debug 交叉编译检查通过。Windows release 交叉检查受依赖 `gpui-pre-windows` 在 macOS 不生成 `shaders_bytes.rs` 阻塞。实际宿主加载视频并从第 300 帧播放到第 329 帧，播放状态为真；不支持平台的挂载请求返回明确错误，原生窗口保持隐藏。Windows 的实际画面、菜单遮挡、焦点、DPI 和父窗口关闭行为仍未验证。

2026-11-05 视频回到 GPUI 纹理显示，删除 wx 画布：宿主 `src/gpui_video.h/.cpp` 的 `VideoSurface`（wxFrame + `VideoDisplay` + `SetParent`/HWND）改写为 `VideoFrames`，`Frame(request)` 读可选 `since`，返回一行 JSON 头（`serial`/`frame`/`width`/`height`/`pixel_format=bgra8`/`byte_length`/`unchanged`）加紧随其后的紧密排列 BGRA 字节，`frame->flipped` 时逐行翻转，并校验 `pitch >= width*4` 与缓冲区长度。`src/gpui_bridge.cpp` 的 `video_frame` 走同一收发路径，`video/tool/*` 改为返回“视觉工具正在重建”的错误；GPUI 侧删除嵌入窗口与 `raw-window-handle` 依赖，新增 `VideoView` 与 `Bridge::raw_call`，取帧按 `serial` 增量、失败重试 25 次×40 ms。实测 1920×1080 单帧 8,294,400 字节，单次往返 8–170 ms（中位约 15 ms），`since` 命中返回 0 字节 `unchanged`。

布局与控件：主工作区恢复左右两栏——左列音频条加撑满剩余高度的视频卡片，右列编辑器加字幕列表，中间竖向拖动条（`WorkspaceResizeDrag::ColumnSplit`，最小 280 px），窗口宽度不足 900 px 时退化为上下排列；视频卡片拆成标题行、画面区、读数行与时间轴四段，没有视频时画面区显示绝对定位空态与“打开视频”按钮。平移下拉（四向各 20 px）和安全区开关加回卡片，安全区按 5%/10% 内缩画两层方框，选项仍是 `Video/Overscan Mask`。点击标题打开独立视频窗口（`Panel::Video`，`detached`+`video_popped`），主窗口随即停取帧，关闭独立窗口时恢复并重新取当前帧。

轮询容错：一次快照反序列化失败会让前端连续 5 次轮询失败并 `app.quit()`，而宿主加载音频时主线程卡顿就能触发。现在改为按时间判定——连续失败 15 秒才退出，期间状态栏显示“连接中断”，恢复后显示“已重新连接”。根因是 `src/gpui_bridge.cpp` 里安全区字段按 `<<` 输出成 `0`/`1`，而 Rust 侧是 `bool`；宿主拼 JSON 布尔必须写字面量 `true`/`false`。

验证：`cargo +1.95.0 fmt`、release 构建、12 项 Rust 测试与 Meson 两组测试通过；桥接直连核对取帧尺寸/字节数、`since` 增量、删除字幕行后音频保持 `loaded` 且标记数与视口不变、列宽写入 `Subtitle/Grid/GPUI Column Widths` 往返生效。真实窗口截图仍因本机缺“屏幕录制”权限（`screencapture` 报 could not create image from display）无法进行，画面与交互需用户目视验收。

2026-11-05 播放同步与读数行：`VideoController` 以音频为主时钟，帧请求过去只在时钟处发出，解码耗时（1080p 实测中位 30 ms、p90 66 ms）直接变成画面落后声音；实测前端上屏帧号与音频位置相差中位 −54 ms，约两帧。现在 `RequestFrame` 记录每个仍在途帧的请求时刻，帧落地时由帧接收方调用 `NoteFrameArrived` 更新一个 0.3 系数的 EMA（上限 120 ms），`OnPlayTimer` 用「时钟 + 该提前量」决定要哪一帧，`Stop` 则先取停止时刻再落帧，避免停在提前量上。同一测量点复测：中位 −24 ms、p90 +13 ms；播放 3 秒内 `frame` 26→51→74 且 `video_time` 2466 ms 对 `audio.position` 2461 ms。wx 侧 `VideoDisplay::UploadFrameData` 同样回调，休眠路径行为不变。注意 wx 对同一 handler 上同一事件只派发一个绑定，所以延迟测量不能再 `Bind(EVT_FRAME_READY)`，改由帧接收方回传帧号。视频读数行（帧号 · 显示比例 · 帧率）此前多套了一层 `text_sm`，比所在行的 `text_xs` 大一档，已还原。Rust 17 项测试与 Meson 两组测试通过。

2026-11-05 宿主错误日志不再弹出模态框：GPUI 接管界面后 wx 主窗口是隐藏的，`wxLogGui` 仍会把错误日志排成对话框，`wxMessageBox` 在不可见窗口上 `NSAlert runModal` 直接把主线程和桥接一起卡死（复现：视频上下文按下帮助键，浏览器启动失败后整机无响应，`sample` 停在 `wxLogGui::Flush`）。`src/main.cpp` 新增 `FrontendLogTarget`，`gpui::FrontendAttached()` 为真时把日志写到 stderr，否则交回 `wxLogGui`。验证：同一操作现在只输出 `wx log: 无法在默认浏览器中打开 URL "http://docs.aegisub.org/3.5/Main_Page/"。（错误 2：No such file or directory）`，桥接持续应答，视频上下文空格键播放／暂停连续五次切换均生效。

2026-11-05 去 wxWidgets 界面：宿主不再包含任何 wx 窗口，改成纯服务进程 `aegisub`（`meson.build` 的 `gpui_host`）。它启动后创建单个 `agi::Context`、启动回环桥接，再以 `--bridge`/`--token` 拉起 GPUI 前端；前端退出即宿主退出。删除 100 余个 wx 界面文件（主窗口、菜单、工具栏、全部 `dialog_*`、音频/视频控件、字幕网格、查找替换等），命令层统一改为 `gpui::PushCommand`/`PushStatus`，提示与询问改为 `PushNotice`/`PushChoice`，需要同步答案的编码、音轨、FPS 选择走 `gpui::AskChoice`（在自己的事件循环上等待前端 `notice_choice`）。帧回调改为 `AsyncVideoSink` + `agi::signal`，播放计时与桥接轮询改用 `gpui::Timer`/`EventLoop`。音频渲染不再依赖 `wxBitmap`，改为自有 BGRA8 `AudioCanvas`；音频标记样式改为 `agi::Color` + 宽度/虚线；字幕格显示文本、自动保存扫描、EBU 设置、版本检查、进度对话框都改为无 wx 实现。验证：`ninja -C build-ui-gpui aegisub` 通过，Meson 两组测试通过；真实进程烟雾测试覆盖打开/编辑/撤销/保存 ASS、音频频谱与波形像素、MicroDVD 的 FPS 询问往返、导出过滤器与 58 种编码列表、253 个字体名，宿主随 SIGTERM 退出。当时仍链接 wx 的部分只剩文本尺寸估算、图像编码/剪贴板、字体枚举、本地化等基础类型；后续已全部替换。


2026-11-05 去 wxWidgets 基础层：上一批之后继续清空所有非界面 wx 依赖。文本尺寸估算改为 CoreText／GDI／fontconfig+FreeType，字体家族枚举改为 CoreText／GDI／fontconfig，剪贴板改为 NSPasteboard／CF_UNICODETEXT／`wl-copy`+`xclip`，PNG 编码改用 zlib 自有实现；本地化改为直接解析 `.mo`，热键、TTXT XML、配置文件默认值、崩溃回溯、音频播放器线程、DirectSound/OSS 线程和 macOS 崩溃报告也全部去 wx。`src/audio_player_portaudio.{h,cpp}` 的输出设备列表由 `wxArrayString` 改为 `std::vector<std::string>`。Meson 删除 wx 依赖、PCH 中的 wx 头文件与 `wx_version` 选项；`otool -L build-ui-gpui/aegisub` 已无 wx 库，`src/` 中不再有 wx 头文件引用。验证：宿主与前端 release 构建通过；桥接实测 `video_frame` 的 `display` 返回 1920×1080 BGRA8，`raw`/`subs` 返回同尺寸 PNG（PNG 魔数正确），前端帧复制、坐标复制和缩放/复位命令已接入；命令 `subtitle/close` 在文档干净时直接关闭，有修改时转由前端先保存。Lua 宏 `validate()` 现在允许只返回布尔值（此前命令目录生成会在可选帮助字符串缺失时抛出 `agi::lua::error_tag`，前端显示“unknown bridge error”），目录恢复为 324 条；`video/detach` 现在真正打开独立视频窗口，实测窗口数 6→7。Meson 测试通过（旧的 wx 语言匹配测试和已删除的缩帧辅助测试一并移除）。

2026-11-07 Windows 启动与 CI 收口（`7655c02c0`）：Windows 宿主原先在 `CreateProcessW` 后崩溃（`-1073741819` 访问冲突、`-1073740791` fail-fast），根因是 `lpStartupInfo` 传了空指针；`src/gpui_host.cpp` 现在传入 `STARTUPINFOW startup{}; startup.cb = sizeof startup;`，并保存 `info.hProcess`、关闭 `info.hThread`。同一轮修复还包括：`agi::dispatch::Init` 必须先于 `new agi::log::LogSink`；`GpuiBridge` 必须在 `agi::Context` 之前销毁；命令行首项必须是 `argv[0]`，只对含空白或引号的参数加引号，保证启动脚本正则能解析 `--bridge`/`--token`。诊断代码已由 `51ceb034b` 删除。验证：CI run `42307865018` 八个任务全部成功（Ubuntu Release/ARM64/Debug、macOS arm64 Debug/Release、macOS x86_64、Windows MSVC、Windows ARM64）；Windows MSVC 的 `Exercise Windows GPUI desktop` 通过，`result.json` 中 `actual_windows_desktop`、`visible_amount_field`、`exact_form_shift_ms=1250`、`undo_restores_clean_document`、`independent_history_scroll`、`compact_stacked_cards` 全为真。ARM64 上一轮 `Configure` 失败是 `git clone` ffmpeg 的网络中断，重跑通过，非源码问题。

2026-11-07 libmpv 跨平台运行库与播放验证：Windows x64 / ARM64 安装包和便携包接入固定 LGPL DLL 与许可证；桌面冒烟步骤已覆盖原生 renderer、预览边界与窗口缩放、鼠标输入归属、时钟推进和暂停隐藏，本轮未运行 Windows CI。macOS 改为固定源码构建并直接链接 MoltenVK，解决最小 OpenGL 构建无法初始化内嵌视频输出的问题；无脚本引擎时只允许 `load-scripts` 和 `osc` 返回 option-not-found，其他初始化错误仍上报。arm64 包内运行库实际播放 1920×1080、60 fps H.264 视频：`vo-configured=true`，时钟从 1.416667 秒推进到 5.683333 秒，两个采样点的渲染丢帧与解码丢帧计数均为 0；这只是本机短时测量，不代表所有媒体和平台的显示帧率。实际画面含 ASS 字幕；暂停后 700 ms，编辑预览与时间轴均到第 478 帧。证据位于忽略目录 `build-ui-gpui/mpv-spike/source-vulkan-*.png`。两种架构运行库构建与最低系统版本检查通过；整包 macOS 13 运行及 Windows 新版桌面验收仍待相应环境验证。

2026-11-08 跨平台 CI 首轮（`ff3099b4e`，run `42390176961`）：Ubuntu 三项、macOS arm64 Debug/Release 通过；Windows x64 / ARM64 编译通过，安装包生成因 runner 的 Windows PowerShell 无法调用 `Get-FileHash` 而失败，尚未执行桌面冒烟。`tools/win-installer-setup.ps1` 改用 .NET 流式 SHA-256，保留所有归档、DLL 和许可证固定摘要比对；本机关闭 PowerShell 模块自动加载后，两种架构的真实归档、DLL 与许可证全部匹配固定值，Windows CI 复验待执行。macOS x86_64 下载 FFmpeg 源码时连接被重置，需要重跑该失败项。

2026-11-08 跨平台 CI 复验（`d9d8f383d`，run `42393594702`）：八项构建全部通过；Windows x64 / ARM64 安装包、便携包及包内容检查通过，Windows x64 的 `Exercise Windows GPUI desktop` 通过，覆盖原生 renderer、输入归属、时钟推进与暂停隐藏。本地下载 10132 两种架构便携包，复验双程序、libmpv、许可证、VC 运行库及 ZIP 内容一致性通过。桌面证据仍有 YCbCr 矩阵不匹配提示框，截图不作为无遮挡预览的视觉验收结论。

2026-11-08 停止维护 macOS Intel：移除 x86_64 CI／发布任务、Rosetta 安装和测试步骤，以及专用交叉编译机器文件和 sccache 层；macOS 仅保留 Apple Silicon Debug／Release，Release 原有源码依赖参数与最低系统版本保持不变。`actionlint` 通过，解析后的构建矩阵为七项，已无 Intel macOS 任务；未为本次配置删减另触发 CI。

2026-11-08 恢复音频相邻字幕范围：`src/gpui_audio.cpp` 从既有 `AudioTimingController::GetRenderingStyles` 导出当前及非当前字幕区间，GPUI 为可见区间绘制填色和起止边界；范围设置与偏好表单统一使用“不显示 / 前一条 / 前一条与后一条 / 全部”，沿用原有选项索引。宿主与前端 release 构建、Rust 格式检查通过；真实宿主加载五条字幕逐项核对四种模式的区间，首尾字幕不产生不存在的相邻范围，当前句保持 3500–4500 ms。实际 GPUI 窗口已确认波形、频谱上的相邻范围及下拉菜单文案，证据保存在忽略目录 `build-ui-gpui/waveform-range-smoke/`；未触发 CI，Windows 视觉验收仍待相应环境。

2026-11-08 修复桌面验收发现的三处问题：带字幕复制帧显式请求 `video_frame` 的 `format:"png"`，宿主按需编码合成帧，前端只把返回格式为 PNG 的数据写入图像剪贴板；播放预览仍默认使用 BGRA8 与 `since` 增量。另存为在原生保存面板确认后发送 `overwrite:true`，宿主据此允许替换，未授权的覆盖仍拒绝。命令行文件载入移至首个通过认证的前端请求，保证关联文件提示和载入过程中的问题有前端接收；沿用已有脚本相对路径解析，不依赖短连接状态的定时轮询。

验证：宿主与前端 release 构建、Rust 格式检查及现有 17 项测试通过。真实宿主核对 PNG 签名、分块 CRC 与 IEND，并经 FFmpeg 独立解码；带字幕 PNG 与默认 BGRA 预览逐像素一致，1920×1080 测试帧有 22,536 个字幕像素，原始帧与仅字幕 PNG 均正常。真实 GPUI 前端的三种复制命令写入系统剪贴板后，均由系统图像解码器成功读取。CLI 覆盖关联媒体禁用、自动加载、询问确认、询问拒绝和显式媒体参数；实际前端显示询问并确认载入音视频。macOS 原生另存为覆盖、取消和新文件保存均验证落盘与文档状态；缺少覆盖标记或标记为 false 时目标文件不变。未触发 CI；Windows 原生覆盖对话框及三处修复仍需新包复验。
