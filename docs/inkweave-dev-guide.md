# Inkweave — Cordis 式插件化跨平台手写笔记软件 · 开发指南

> 版本：**v2.4**（第五轮：原生插件装入契约）· 2026-10-13
> 变更：合并架构评审（4 致命缺陷修复）与可用性评审（23 项遗漏补全）；**本版新增 ADR-006~008 与 §8「插件管理与运行时治理」**，把「添加插件入口 / 插件热加载与关闭 / 统一控制面板 / Material Design / 手写笔与 WebDAV 预置插件 / 后台机制与内存管理」六项要求正式纳入正文
> 读者：本项目的后续开发者与 AI 协作者（任何模型读完后应能独立推进开发）
> 设计信条：**一切皆为插件 · 注册即可撤销 · 无特权核心 · 零妥协的低延迟**

---

## 修订记录

| 版本 | 日期 | 变更 |
|---|---|---|
| v1.0 | 2026-10-12 | 初稿，综合 5 份调研 |
| v2.0 | 2026-10-12 | 修复 F-1（dylib→crate/WASM）、F-2（输入路径 POC 前置）、F-3（PDFium 线程模型）、F-4（无锁服务解析）；补全撤销/橡皮擦/选择工具等 23 项；新增名词表、monorepo 结构、数据流图、测试策略、不移植清单、技术债务 |
| **v2.1** | **2026-10-12** | **合入六项新要求（本次改动）**：① §8.4 新增「添加插件」入口；② §8.2 定义插件**热加载/关闭**分级语义（ADR-007）；③ §8.3 定义**统一控制面板**（ADR-008）；④ ADR-006 确立 **Material Design 3** 为 UI 设计语言；⑤ §8.7 明确**手写笔系统**与 **WebDAV 同步**为可开关**预置插件**；⑥ §8.5/§8.6 补齐**后台机制**与**内存管理**统一治理层。同步修订 §0 包体积决策、§2.3 不移植清单、§9.5 技术债务、§10 路线图。**合并锚点见本节** |
| **v2.2** | **2026-10-13** | 新增 **ADR-002 补充：延迟测量方法**——承认摄像机不是唯一手段，给出 Perfetto 与应用内探针两种替代、各自精度，并明确「软件方法可判失败、不可判通过」；记录「未做真机标定即推进 M1」为已接受风险。依据：M0 已交付完毕、Spike-0 无法在本机取得真机数据 |
| **v2.3** | **2026-10-13** | **ADR-003 修订**：用户要求笔记功能由用户自行导入，故允许**原生动态库（cdylib）**插件，但**只走 C ABI**（保留原裁决中关于 Rust 无稳定 ABI 的正确观察，约束比原方案更严）；同步修订 §8.1 的分发策略。明示未解决项：**Dart 侧 UI 不能运行期加载**，插件 UI 仍由宿主渲染。依据：§7.1 既已确定 WASM 无法触达 GPU，渲染型插件只能走原生 |
| **v2.4** | **2026-10-13** | **ADR-003 补充：原生插件的装入契约**（见下）——把 v2.3 的「可以装载」推进到「可以装入」：探测先于确认、包复制进宿主私有存储后再复核、挂载对象必须是用户确认过的那份字节；`PluginKind::Native` 与 WASM 共用同一套四动词与同一个控制面板分区。附 Android 模拟器端到端实测结论。**仍未解决**：原生通道无沙箱，权限清单是告知而非隔离边界（WASM 沙箱仍为 M6） |

---

## 0. 项目愿景与硬性指标

一款以 Cordis 插件思想为骨架、以手写体验为灵魂、以 PDF 为底座的跨平台笔记软件。

| 指标 | 目标 | 度量方式 |
|---|---|---|
| 手写端到端延迟 | 120Hz 设备 < 28ms；60Hz 设备 < 40ms | 高速摄像机 240fps 拍摄笔尖到像素出现；设备矩阵：iPad Pro M4 / 小米 Pad 7 / 中端 Android 平板（60Hz） |
| 冷启动到可书写 | < 1.5s（中端 Android 平板） | `flutter run --profile` + systrace |
| 单页 10000 点笔迹渲染 | < 10ms/帧 | wgpu timestamp query |
| 包体积（Android 基线 APK） | < 40MB | **v1.0 第一方内部模块全部内置、不做按需下发**（避免动态分发的版本兼容复杂度）；**WASM 外部扩展走应用内「添加插件」装入，不计入基线包体积**（ADR-007 / §8.1） |
| 主进程崩溃率 | < 0.1% crash-free session 反向计 | Sentry 或同级；插件崩溃独立统计且不影响主进程 |
| 插件启停耗时 | 内部模块启用/停用 < 200ms；WASM 扩展装入/卸载 < 800ms | 控制面板操作端到端（点击 → Fiber 状态终态 → UI 确认）；中端 Android 平板 + Release 构建 |
| 控制面板冷开 | < 300ms 至可交互（插件数 ≤ 20） | `flutter run --profile` + 时间线；插件状态来自 Rust 侧快照，不阻塞首帧 |
| 平台覆盖 | Android / iPadOS / Windows / macOS / Linux；Web 为降级模式（Canvas 2D 渲染，无 wgpu） | — |

非目标（洁癖红线）：
- 不做 Electron/WebView 壳方案（手写延迟无法满足）
- 不引入 AGPL 依赖（MuPDF 一票否决）
- 不在核心中硬编码任何"功能"（同步、导出、笔刷、手写识别——全部是插件）
- **不做代码级热替换（HMR / DSU）**：移动端无场景，且会破坏「身份 = 不变量集合」的可逆性保证。插件动态性一律落在 §8.2 定义的**运行时装/卸 + 启停**语义上，不做已加载代码的原地替换

---

## 0.5 名词表（先读这个，避免歧义）

| 术语 | 定义 |
|---|---|
| **Context** | cordis-rs 的层级化服务容器；与 Fiber **1:1 绑定**，子 Context 沿父链查找服务 |
| **Fiber** | 单个插件实例的生命周期状态机（六态）；一个 `ctx.plugin()` 调用产生一个 Fiber；子插件有独立 Fiber 并随父级联卸载 |
| **Effect** | 经 Context 注册的、可逆的副作用，卸载时 LIFO 回收 |
| **Definition / Provider / Consumer** | 服务接缝三层：Definition 定义 trait+数据类型；Provider 是实现方插件；Consumer 是调用方插件。后两者只依赖 Definition |
| **笔画 (stroke)** | 一次落笔→抬笔的连续 StylusData 点序列；**持久化与撤销的最小单元** |
| **笔迹 (ink)** | 一个图层内所有笔画的集合 |
| **`.nbk`** | Inkweave 笔记文件的 ZIP 容器扩展名（弃用 `.notebook`） |
| **内部模块 (internal module)** | 以 Rust crate 静态链接进主二进制、但遵循 Plugin trait 的第一方功能（ink/pdf/storage/sync）。**v2.0 起第一方不再使用 dylib** |
| **外部扩展 (extension)** | 第三方 WASM 沙箱插件 |
| **预装插件 (built-in plugin)** | 默认启用的内部模块，可在设置中禁用 |
| **统一控制面板 (Control Panel)** | 应用内唯一的插件治理入口：查看/启停/装入/卸载插件，呈现状态、能力、权限、内存与后台任务。**本身也是一个预装内部模块**（无特权核心的自我印证） |
| **启停 (Enable / Disable)** | 对**已装入**插件做运行时挂载/卸载 Fiber 的操作。内部模块与 WASM 扩展均支持，即时生效、无需重启宿主 |
| **装入 / 卸载 (Install / Uninstall)** | 对 **WASM 外部扩展**的动态增删：把扩展包写入扩展目录并 `ctx.plugin()` 挂载；卸载则 `fiber.dispose()` 并删除扩展包。**内部模块不适用**（编译期已组合） |
| **热加载 / 关闭 (Load / Shutdown)** | 本指南对「插件动态性」的总称，严格等价于 §8.2 的四动词模型 `Load / Enable / Disable / Unload`；**不含**已加载代码的原地热替换（HMR） |
| **后台调度统一层 (BackgroundScheduler)** | 宿主提供的**唯一**后台任务通道，对 `WorkManager`(Android) / `BGTaskScheduler`(iOS) 的封装。插件不得自行创建线程/闹钟 |
| **内存治理层 (MemoryGovernor)** | 宿主提供的统一内存治理服务：插件内存预算、`onTrimMemory` 广播、缓存回收与空闲插件卸载 |
| **前置缓冲** | 渲染结果在 VSync 前部分进入显示管线的低延迟技术 |
| **StylusData** | 标准化手写笔输入 `{x, y, pressure, altitude, azimuth, t}` |
| **capabilities 位图** | 手写笔驱动声明的能力集合，UI 据此显隐功能 |
| **LWW** | Last Write Wins，毫秒级时间戳冲突解决策略 |
| **墓碑 (tombstone)** | 已删除实体的标记记录，保留 30 天后物理清除 |
| **PlatformView** | Flutter 嵌入原生视图的机制，用于承载 wgpu 手写画布 |
| **waterfall / bail** | cordis 事件分发模式：中间件链 / 同步短路 |

---

## 1. 总体架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                        UI 层（Flutter / Dart）                       │
│   文件库 · 设置 · 工具栏 · 插件管理 —— 全部为内部 UI 模块的视图        │
├─────────────────────────────────────────────────────────────────────┤
│                Platform View 手写画布（每平台 1 个原生 Surface）      │
│   Android: SurfaceView+Vulkan │ iOS: CAMetalLayer │ Desktop: winit   │
│   ⚠ 输入路径：原生 View 直接捕获 MotionEvent/UITouch → FFI → Rust    │
│      （不经过 Flutter Dart 层——见 ADR-002）                           │
├──────────────────────────────┬──────────────────────────────────────┤
│        Rust Core（libinkweave，单静态二进制）                         │
│  ┌───────────────────────────────────────────────────────────────┐  │
│  │ cordis-rs 微内核：Context 树 · Fiber 状态机 · Effect 栈 · 事件总线│  │
│  │ 并发模型：ArcSwap 无锁读 + 启动期单线程注册（见 ADR-004）          │  │
│  ├──────────┬──────────┬──────────┬──────────┬──────────────────┤  │
│  │ ink-     │ pdfium-  │ storage  │ sync-    │ ext-loader       │  │
│  │ engine   │ engine   │ (SQLite) │ webdav   │ (WASM/wasmtime)  │  │
│  │ (wgpu)   │ (句柄池) │ +FTS5    │          │                  │  │
│  │ crate    │ crate    │ crate    │ crate    │ crate            │  │
│  └──────────┴──────────┴──────────┴──────────┴──────────────────┘  │
│   以上为内部模块：静态链接，Cargo feature 控制启用                    │
├──────────────────────────────┴──────────────────────────────────────┤
│           平台适配层（手写笔 SDK / 文件权限 / 后台任务 / 输入预测）     │
└─────────────────────────────────────────────────────────────────────┘
```

### ADR-001：Flutter UI + PlatformView 画布（维持 v1.0 决策）

Flutter 负责一切非手写 UI；手写画布用 PlatformView 承载原生 Surface，Rust/wgpu 独占渲染。理由见 v1.0：Impeller 无法嵌入自定义 Vulkan/Metal 管线。

### ADR-002：输入路径与渲染路径分离论证（F-2 修复）

v1.0 只论证了渲染路径绕过 Flutter，**输入路径未论证**。补充裁决：

- **Android**：PlatformView 的原生 `SurfaceView` 直接在 View 层级的 `onTouchEvent` 捕获 `MotionEvent`（含 `getHistorical*()` 历史点），经 JNI 直调 Rust FFI。**不经过 Flutter Dart 层**，Flutter 的指针调度器不在此路径上。
- **iOS**：PlatformView 的原生 `UIView` 子类在 `touchesBegan/Moved` 捕获 `UITouch`（含 predicted touches），主线程直接调用 Rust FFI。同样不经过 Flutter。
- **遗留风险**：Flutter 3.44+ 的 HCPP 合成模式与 `SurfaceView` 前置缓冲（`GLFrontBufferedRenderer` 语义）能否共用 EGL context 未经证实。
- **强制门禁（M0 新增，一票否决）**：先做 **Spike POC**——最小 App 验证两端完整 touch-to-pixel 链路并实测延迟。若实测超预算 >30%，启用备选架构 B：**全屏原生 Surface 为底、Flutter 以透明背景叠加在上层**（牺牲布局集成，彻底消除输入路径 Flutter 依赖）。POC 未通过前 M1 不得开工。

### ADR-002 补充：延迟测量方法（v2.2 新增）

原 ADR-002 把「高速摄像机（240 fps）」写成唯一手段，这实际上把门禁变成了「没有摄像机就无法开工」。
补充可接受的替代方法并写明各自精度——**这是方法补充，不是降低标准**。

| 方法 | 覆盖区间 | 精度 | 用途 |
|---|---|---|---|
| 高速摄像机 240 fps | 笔尖 → 光子（全链） | 真值，约 ±1 帧（4.2 ms） | 门禁最终标定；每个设备家族一次 |
| Perfetto `frame_timeline` + `input`（Android 12+） | HAL 时间戳 → 帧 present | 高（系统级时间戳同一时基） | **免摄像机的输入延迟测量**；持续回归 |
| 应用内探针：`FrameTiming` 帧内跨度 | vsync → rasterFinish | 高（同一 `FrameTiming`，无跨时钟运算） | 开发期回归帧内成本；**不含输入/合成/面板** |

**实测结论：纯 Dart 探针无法测输入延迟（2026-10-13）**

首版探针把 `PointerEvent.timeStamp` 与 `FrameTiming.rasterFinish` 相减，在模拟器上得到 **p50 = 26 570 ms**
这一不可能的结果。原因是两者**不同时基**：指针时间戳是应用相对的，帧时间戳来自平台单调时钟，
差值中混入一个约 26 秒的常数偏移。因此：

- **纯 Dart 无法把输入时间戳与帧时间戳对齐**；应用内探针只能给出**帧内跨度**（同一 `FrameTiming`
  的两端，无跨时钟运算），它是有意义的回归信号，但**不构成延迟**。
- **免摄像机的输入延迟测量必须使用系统级追踪**：Perfetto 的 `input` 与 `frame_timeline` 数据源把
  输入事件与帧上屏放在同一个系统时钟上，这是唯一可信的软件路径。
- 应用内探针据此**不再给出门禁结论**，只报告帧内跨度，并显示跨时钟差值的原始范围作为证据。

**判定规则（修订）**

1. 任一覆盖了时间戳的方法，只要 **p95 超预算**，即为**不通过**——不需要摄像机佐证。
2. 软件方法**通过不等于门禁通过**：它看不到两端物理量（数字化仪采样 + 面板响应），二者合计可达
   十余毫秒，正是预算里最不该被忽略的部分。门禁结论仍须由摄像机——或 Perfetto 全链追踪——在一次
   真机运行中给出。
3. 应用内探针的登记用途**仅限帧内回归**，不得用于输入延迟结论。

**当前状态与风险接受（2026-10-13）**：测量设施已实现并可在设备上运行
（`app/lib/modules/debug/latency_probe.dart`，统计与屏幕均有测试），但**真机输入延迟数据仍为零**：
模拟器数字无门禁意义，且如上述纯 Dart 方法本身不成立。据此，项目**决定在未完成任何真机标定的前提下
推进 M1**——这是一项**明示的已接受风险**；首次真机运行（Perfetto 或摄像机）后必须回写本节并给出结论。

### ADR-003 修订：原生动态库回来了，但只走 C ABI（v2.3 修订）

v2.0 的裁决「第一方 = 静态 crate，弃 dylib」建立在一处**正确的观察**上——Rust 没有稳定 ABI，
跨编译单元传 `dyn Service` trait object 是未定义行为——但由此推出的结论过宽：它把「不能用 Rust ABI」
误当成了「不能用动态库」。用户要求**笔记功能由用户自行导入并启用**，这需要运行期装载，
因此重新裁决。原文不删除，保留推理痕迹。

**保留且不可协商的约束**

1. **宿主与插件之间只走 C ABI**：`#[repr(C)]` 结构体 + 函数指针表 + `extern "C"` 导出符号。
   `String` / `Vec` / `dyn Trait` / 泛型等 Rust 类型**一律不得跨越边界**。
2. **共享 ABI crate**：`inkweave-plugin-abi` 由宿主与插件**共同编译**，类型按 C 布局一致，
   因此**不依赖 Rust ABI 的稳定性**。宿主与插件的 Rust 版本可以不同，只要 target 与 ABI 版本一致。
3. **导出符号最少化**，且装载前由宿主校验 `abi_version`；不匹配即拒绝装载（同 §7.1 的 ABI 策略）。
4. **宿主服务经函数指针表注入**（与 §7.1 的 WASM host function 同构）：插件**不链接宿主 crate**，
   只能通过表里的能力调用。表带 `size` 字段，宿主可向后兼容地扩展能力。
5. **平台约束不下放**：插件不得自行创建线程/闹钟/前台服务，后台工作仍只能经 §8.5 的调度通道。

**分发形态（修订后）**

| 形态 | 装载时机 | 适用 |
|---|---|---|
| 内部模块（静态 crate） | 编译期 | 平台适配、内核配套、无法动态加载的部件 |
| **原生动态库（cdylib）** | 运行期 `dlopen` | **笔记功能等由用户导入的能力** |
| WASM 扩展（wasmtime） | 运行期 | 纯逻辑、需强沙箱的第三方 |

**为什么渲染能力只能走原生**：§7.1 明确「WASM 无法触达 GPU/平台 API」，而 ink-engine 的核心是
wgpu 渲染。WASM 通道因此保留给不需要 GPU 的第三方逻辑，渲染型插件走原生动态库。

**本修订不解决的（明示，避免被误读）**：**Dart 侧 UI 无法运行期加载**——Flutter 的 AOT 构建不支持
载入 Dart 代码。因此「笔记插件」的 **UI 部分仍须由宿主提供或随宿主发版**，插件贡献的界面一律走
§8.3 的「宿主渲染贡献点」路径。也就是说，**"用户导入"覆盖的是能力与逻辑，不包括 UI 代码本身**。

### ADR-003 v2.4 补充：原生插件的「装入契约」

v2.3 说的是「装载」（`dlopen` 一个 cdylib 是可行的）。把「装载」变成用户可用的「装入」，还差一条
契约——**用户确认的字节，必须就是被挂载的字节**。原生通道没有沙箱，权限清单只是**告知**，因此这条
契约是它在安全上唯一能拿出的东西，必须写成规范而不是实现细节。

**装入五步（顺序不可调换）**

1. **探测先于确认**：清单**从二进制内部**读取（`inkweave_plugin_abi_version` → `..._manifest`），
   不读同名 sidecar 文件——只有与代码同一次编译产出的字节才可信。此步会运行库的初始化段，但**不调用
   任何生命周期函数**。确认对话框据此列出插件**真实申请**的权限，而非一套默认值。
2. **复制进宿主私有存储**：`<support>/extensions/<id>/plugin.<so|dll|dylib>`，并创建
   `<id>/files/` 作为经 ABI 交给插件的私有数据目录。用户选中的文件可能位于选择器缓存区，随时可能被
   回收，因此**不能直接挂载它**。
3. **复核副本**：对副本再探测一次，与第 1 步的清单**逐字段比对**；不一致即拒绝装入并删除已复制的目录。
   这一步关掉"确认与装载之间文件被替换"的时间窗。
4. **挂载即 `dlopen` 副本**：`enable` 在每次装入时重新打开库（而非安装时打开一次），
   这样停用/启用循环才能正常走 `Suspended → Active`。
5. **拒绝即回滚**：权限未被全部接受、ABI 不匹配、清单不一致、挂载失败——四种情况都必须**不留下任何
   卡片与磁盘残留**。

**与四动词的关系**：`PluginKind::Native` 与 `Wasm` 共用 ADR-007 的四动词，控制面板**不新增管理路径**，
只在卡片上标注通道（预置 / WASM 扩展 / 原生扩展）——因为"同进程无沙箱"这一点不能对用户隐藏。
`uninstall` 先 `disable`（触发 `disable` + `destroy`，并让宿主 `dlclose`），再删除
`<support>/extensions/<id>/` 整个目录。

**平台事实（Android 实测，非推断）**：应用私有存储中的 `.so` **可以** `dlopen`——SELinux 会对
`/data/data/<pkg>/files/extensions/<id>/plugin.so` 记录 `avc: granted { execute }`，不是拒绝。
因此"只能加载 APK 内 `lib/` 里的库"这一常见说法不适用于按权限表赋权的应用私有目录。
选择器（DocumentsUI）对 `.so` 的可选性也不受 MIME 过滤影响，`FileType.any` + 宿主侧扩展名校验即可。

**仍然不解决的**：原生插件与宿主**同进程**，它能访问宿主能访问的一切；权限清单是**披露**不是**边界**。
需要真隔离的能力只能等 M6 的 WASM 沙箱（且 WASM 触达不了 GPU，见 §7.1）。

### ADR-003：第一方插件形态 = 静态 crate（F-1 修复，已由 v2.3 修订）

v1.0 的"Rust dylib 插件"方案**废除**。原因：Rust 无稳定 ABI（rust-lang/rfcs#600 仍 Deferred），跨编译单元传递 `dyn Service` trait object 是未定义行为；`abi_stable` crate 要求全部接口改为 `#[sabi_trait]`，代价不可接受。裁决：

- **内部模块 = 普通 Rust crate**，静态链接进 `libinkweave`，`Cargo feature` 控制启用/禁用。它们照样实现 `Plugin` trait、走 `ctx.plugin()` 注册、享有完整 Fiber 生命周期——**插件语义不变，只是分发形态从动态库变为编译期组合**。
- **动态性需求只留给第三方外部扩展**：WASM（wasmtime）沙箱，详见 §7。
- 手写笔厂商插件（小米/华为 SDK）属特殊情况：SDK 本体是平台 jar/framework（经 JNI/ObjC 桥接入），Rust 侧适配器仍编译进主二进制，按设备能力位图在**运行时激活/停用**（功能开关，而非动态加载）。

### ADR-004：cordis-rs 并发模型 = 无锁读 + 启动期注册（F-4 修复）

v1.0 的 `RwLock<HashMap>` + 父链递归读锁存在写饥饿与锁链风险。裁决：

- 服务注册表使用 **`arc_swap::ArcSwap<HashMap<ServiceKey, ServiceEntry>>`**：读（服务解析）完全无锁；写（注册/注销）复制-替换整个 map。服务注册集中在启动期与插件装/卸的稀有时刻，查询是热路径——读写不对称场景下 ArcSwap 最优。
- **解析快照**：Fiber 进入 `Active` 时将其 inject 依赖解析为 `Arc<dyn Service>` 缓存于 Fiber 内，运行期热路径不再走父链动态查找；provider 替换时级联重建依赖方 Fiber（见 §2.2 不变量 3）。
- `fiber: Weak<Fiber>` 改为 **`Arc<Fiber>` 双向持有**（Context↔Fiber 互持，dispose 时显式断开），并定义契约：Fiber 进入 `Disposed` 后，经其 Context 发起的一切注册调用返回 `Err(Disposed)`，绝不 panic。
- 事件总线监听器表用 `ArcSwap` 同样处理；`emit` 快照读，不持锁回调。

### ADR-005：PDFium 线程模型 = 句柄池（F-3 修复）

PDFium **线程不安全**（`FPDF_DOCUMENT` 不可跨线程并发使用；`pdfium-render` 的 `PdfDocument: Send + !Sync`）。裁决：

- 实现 **PDFium 句柄池**：N 个渲染 worker 各持独立 `FPDF_DOCUMENT`（同文件的独立加载副本），任务队列按页分发；LRU 缓存的是**渲染产物 tile 位图**，不是 PDFium 句柄。
- 句柄池大小 = `min(2, cpu_cores/4)`，移动端默认 2——每个句柄独立解析 xref 表，内存开销可控（文档通常 <50MB）。
- 所有编辑类操作（注解/页面管理/保存）串行到**单一写线程**，与渲染 worker 通过读写闸门协调（编辑时暂停渲染该文档）。
- v1.0 引用的"58 FPS"指标源自 MuPDF/CGPDF 实测，**PDFium 需 M3 实测验证**，偏差 >10% 则降级为方案 B（单渲染线程 + 异步调度）。

### ADR-006：UI 设计语言 = Material Design 3（v2.1 新增）

Flutter 侧全部界面（文件库、设置、工具栏、**统一控制面板**、插件贡献视图）统一采用 **Material 3**：

- 以 `ThemeData(useMaterial3: true)` 为基线，颜色取 `ColorScheme.fromSeed` 动态种子色（Android 12+ 支持时读系统动态色）。
- 组件一律取自 `flutter/material.dart` 的 M3 组件（`NavigationBar` / `FilledButton` / `Card` / `Switch` / `SnackBar` / `AlertDialog` / `SegmentedButton`），禁止自绘硬编码配色。
- 令牌化：间距、圆角、海拔、动效时长统一走主题令牌，暗色模式由 `ColorScheme` 自动适配（含画布暗色适配，见 §3 与 M1 末交付）。
- **插件界面同样受约束**：WASM 扩展不直接产出 UI，其贡献点（命令/菜单/设置项）由宿主以 M3 组件渲染（§8.3），保证视觉一致性，杜绝插件各造一套皮肤。
- 无障碍：触控目标 ≥ 48dp，全部图标带语义标签，支持系统动态字号（M2 同步要求）。
- 例外：手写画布（PlatformView 原生 Surface）不适用 M3 组件，但其**工具栏与外框**仍为 M3。

### ADR-007：插件「热加载 / 关闭」分级语义（v2.1 新增）

新需求「插件须支持热加载与关闭」与 §2.3「不移植 HMR」、§9.5 技术债务「移动端无插件热重载」需要显式对齐。裁决：

- **两件事被区分开**：*动态性*（插件能否在运行时装/卸、启/停）≠ *热替换*（已加载代码能否原地换新版本）。
- **本体论**：一切插件都是 `Plugin` trait 的实现，装在同一个 cordis-rs 内核上。差别仅在**分发形态与装载时机**：

| 插件形态 | 装入 | 启停 | 卸载 | 代码热替换 |
|---|---|---|---|---|
| 内部模块（第一方 crate） | 编译期组合，随宿主启动即"已装入" | ✅ 运行时 `ctx.plugin()` / `fiber.dispose()` | 不适用（仅可停用） | ❌ 明确不做 |
| WASM 外部扩展（第三方） | ✅ 运行时从扩展目录装入 | ✅ 同上 | ✅ `dispose` + 删除扩展包 | ❌ 明确不做 |

- 因此**"热加载/关闭"= 运行时装/卸 + 启停**，全部落在 Fiber 状态机（§2.4）与 effect 回收（§2.1）既有能力上，**不新增内核机制**，也不需要 HMR。
- 内部模块的启停即 §6 手写笔 provider 激活、§5.5 同步 Provider 启停所用的同一机制——**统一实现，无第二套开关**。
- 需要"换版本"时：WASM 扩展走 **卸载 → 装入新版本**（干净状态重启 Fiber，符合 HMR「不做状态迁移」的既有结论）；内部模块换版本必须随宿主发版。

### ADR-008：统一控制面板 = 预装内部模块（v2.1 新增）

新需求「统一的控制面板」在「无特权核心」原则下有唯一正确形态：

- 控制面板**不是**内核特权功能，而是与 ink/pdf/storage 平级的**预装内部模块** `plugin-manager`，自身实现 `Plugin` trait、走 `ctx.plugin()` 注册、享完整 Fiber 生命周期，**可被停用**（停用后仅剩一个最小降级视图，避免用户把自己锁死——见 §8.3）。
- 它作为 **Consumer** 消费 `PluginRegistry` 服务（Definition 层新增，见 §8.1），只读内核快照，不持有其他插件实例（§7.2 红线）。
- 所有插件的启停、装入、卸载必须经它发起 → 内核 API；**禁止任何插件自建管理界面**（否则破坏"统一"）。
- 与 §2.4 的 **Lifecycle Bridge** 对接：控制面板 UI 的状态取 Rust 侧 Fiber 快照 + `Stream<FiberStateChange>`。

---

## 2. cordis-rs 微内核

### 2.1 五原语（按 ADR-004 修订）

```rust
pub struct Context {
    parent: Option<Arc<Context>>,
    services: ArcSwap<HashMap<ServiceKey, ServiceEntry>>,   // 无锁读
    fiber: Arc<Fiber>,                                      // 双向持有
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn inject(&self) -> &[ServiceKey] { &[] }
    async fn apply(&self, ctx: &Context) -> Result<()>;
}

pub enum FiberState { Pending, Loading, Active, Failed, Unloading, Disposed }

impl Fiber {
    pub fn effect(&self, f: impl FnOnce() -> Disposer);
    async fn recover(&self);   // LIFO 逆序回收 + 级联卸载子 Fiber
    fn resolved(&self, key: ServiceKey) -> Option<Arc<dyn Service>>; // Active 期快照
}
```

### 2.2 核心不变量（洁癖条款，Code Review 必查）

1. **一切经 ctx 的操作都是 effect**：注册命令、菜单、事件监听、服务提供、子插件挂载，全部走 `ctx.effect()`，无豁免。
2. **卸载即回滚**：`recover()` LIFO 执行全部 disposer 并级联卸载子树。插件作者不允许手写清理代码。
3. **inject 响应式（降级版）**：依赖缺失时 `Pending` 等待；provider 替换时依赖方级联重启。**但 v1.0 的"存储后端热切换"场景降级为「重启应用后生效」**——运行中切换存储是伪需求，级联重启在数据面（已打开文档、undo 栈）的复杂度不值得（评审 M-5/矛盾 7）。inject 响应式保留给真正有价值的场景：手写笔 provider 按设备激活、同步插件启停。
4. **事件五模式**：`emit` / `parallel` / `serial` / `bail` / `waterfall`。笔记场景映射：
   - `waterfall("note/before-save", doc)`：压缩 → 加密 → 索引，每环可改写或拦截。
   - `bail("note/can-delete", id)`：锁定插件返回 `false` 即阻止。
   - `parallel("note/saved", id)`：缩略图重建、FTS 索引、WebDAV 入队扇出。
5. **无特权核心**：内部模块全部是 Plugin，核心只有 cordis-rs 本身。

### 2.3 不移植清单（明确放弃的 Cordis 特性）

| Cordis 特性 | 不移植原因 |
|---|---|
| HMR 热重载 | 移动端无场景；桌面开发期用 cargo watch 重启替代。**注意：这不影响「插件热加载/关闭」需求——该需求由运行时装/卸 + 启停满足（ADR-007 / §8.2），与代码热替换是两件事** |
| Config Schema 校验（schemars 运行时） | v1 内部模块配置编译期确定；WASM 扩展用 manifest 静态声明 |
| Context `intercept`（服务访问拦截改写） | 增加解析复杂度，评审期未发现真实需求 |
| Loader 声明式配置协调（cordis.yml diff） | 内部模块编译期组合，无运行时清单 diff 需求；扩展管理用简单 enable/disable 列表 |
| Proxy 动态属性（TS 特性） | Rust 用显式 `ctx.get::<T>(key)`，类型安全由泛型保证 |

### 2.4 Dart 侧 UI 模块与 Rust Fiber 的生命周期桥

内部 UI 模块（Dart widget）与 Rust Fiber 分处两个运行时，通过 **Lifecycle Bridge** 同步：

- Rust 侧 Fiber 状态变更经 `Stream<FiberStateChange>` 推送到 Dart（frb stream，at-least-once；Dart 侧按 `(plugin, generation)` 去重）。
- Dart 侧 `PluginHostWidget` 监听 stream：`Loading/Pending` → 骨架屏；`Failed` → 错误卡片+重试；`Unloading` → 灰度遮罩；`Disposed` → 移除 widget。
- Dart 发起操作（用户点"禁用"）→ frb 调用 `fiber.dispose()` → 异步，UI 立即进入 `Unloading` 展示态，等 stream 确认 `Disposed`。
- 所有 Dart 侧响应必须在 Flutter UI 线程（frb 默认保证）。

---

## 3. ink-engine 手写引擎

### 3.1 渲染管线与延迟预算

```
原生 View 捕获 MotionEvent/UITouch（ADR-002，不经 Flutter）
  → 历史点/合并事件提取（getHistorical* / coalesced / predicted touches）
  → [FFI 零拷贝：写入共享 ring buffer，非 frb 消息] StylusData{x,y,pressure,altitude,azimuth,t}
  → One Euro Filter（min_cutoff=1.0, beta=0.01）
  → 2px 等距重采样（对象池复用，栈分配优先 smallvec，堆分配进 typed-arena）
  → 压感曲线 → width（用户可校准；无压感退化为速度映射）
  → Triangle Strip 网格化（切线平均法线，圆角 join）
  → wgpu 增量写 ring vertex buffer，单 draw call 追加
  → 前置缓冲提交
```

延迟预算（120Hz，28ms 目标）：采样+批处理 ~4ms → 应用处理 <2ms → GPU 提交 ~2ms → VSync+显示 ~8ms = **16ms 已分配，余量 12ms 预留**给 shader 预热缺失、系统调度抖动与预测回退重绘——余量是有意设计而非遗漏（评审 ③-3.3）。60Hz 目标 <40ms，同一预算结构，VSync 项变为 16.7ms。

**热路径纪律**：禁止堆分配（对象池 + `smallvec` + `typed-arena`）、禁止锁竞争（ring buffer 单生产者单消费者无锁）、禁止日志调用（debug_assertions 编译剥离）。

### 3.2 撤销/重做（v2.0 新增，P0）

与 Cordis Effect 天然同构——**每个编辑操作就是一个 effect**：

```rust
enum InkCommand {
    AddStroke { page: PageId, stroke: Stroke },          // disposer = 移除该笔画
    RemoveStroke { page: PageId, stroke: Stroke },       // disposer = 恢复（橡皮擦的逆）
    TransformStrokes { ids: Vec<StrokeId>, before: Transform, after: Transform },
    AddObject / RemoveObject / ...                        // 文本框、图片同理
}
```

- 粒度：**笔画级**；连续书写不合并，一次"笔画提交"（抬笔+整形完成）= 一条 command。
- 栈结构：每文档一个 undo/redo 栈，上限 200 条（可配），跨页允许。
- 持久化：command 日志追加写入 `.nbk` 内 `undo_log.bin`（与笔画同编码），**支持跨会话撤销**；文件保存成功点 = 日志 checkpoint。
- 崩溃恢复：启动时检测上次退出未 checkpoint → 重放 undo_log 至最后一致状态（见 §5.4 自动保存）。

### 3.3 橡皮擦（v2.0 新增，P0）

两种模式，工具栏可切换：

| 模式 | 行为 | 实现 |
|---|---|---|
| **笔画擦除**（默认） | 笔尖轨迹碰到的笔画整条删除 | 空间索引（每页一个 Grid 索引，cell=64px）做 AABB 粗筛 → 笔画多段线与橡皮轨迹的精确相交检测（segment-segment 距离 < eraser_radius）；命中笔画标记删除，提交一条 `RemoveStroke` command（撤销粒度=一次擦除行程内全部命中） |
| **像素擦除** | 只擦掉轨迹覆盖的部分 | 命中笔画按交点**切分**为多条子笔画（原始点序列切割，非位图涂抹——保持矢量本质），旧笔画替换为新子笔画集合 |

橡皮轨迹本身渲染为半透明圆形光标。Apple Pencil 双击 / 笔尾橡皮（`TOOL_TYPE_ERASER`）自动切换。

### 3.4 选择工具与对象操控（v2.0 新增，P1）

- **套索/矩形选择**：轨迹闭合多边形与笔画的点-多边形包含测试（笔画采样点命中率 >50% 视为选中）；Grid 索引粗筛加速。
- 选中集操作：移动（实时位移渲染，抬笔提交 `TransformStrokes`）、缩放/旋转（双指手势，同上）、删除、复制/剪切/粘贴（跨页允许）、成组。
- 选中渲染：选中框 + 笔画高亮 overlay 层，不修改笔画本体。

### 3.5 笔刷系统（插件化示范 + 混合模式细节）

笔刷是插件：`trait Brush { fn width(&self, p: &StylusData, v: f32) -> f32; fn blend(&self) -> BlendState; fn fragment_shader(&self) -> &str }`。

- 钢笔/铅笔/激光笔：标准 alpha blend。
- **荧光笔**：wgpu blend state 配置为正片叠底（`src_factor=DstColor, dst_factor=Zero`，颜色通道 multiply），同层叠加不加深；同一条笔画自交区域靠 stencil 标记防重复混合。
- 铅笔纹理：片元着色器采样噪声纹理，按笔迹 u 坐标拉伸。

### 3.6 图层系统（v2.0 补全定义，P1）

- 每页 1..N 个图层 + 第 0 层固定为页面背景（PDF 页/模板/纯色，不可删除不可手绘）。
- 图层属性：`visible / locked / opacity / name`；渲染按序合成（opacity 经 offscreen target 合成）。
- 笔迹/文本框/图片对象归属单一图层；选择工具默认只作用于当前活动层（锁定层不可选）。
- 导出扁平化时按序合成所有可见层。

### 3.7 手势与防误触（v2.0 新增，P1）

| 手势 | 行为 |
|---|---|
| 单指（笔模式） | 平移画布（可在设置改为绘制） |
| 双指 pinch | 缩放（0.25x–8x）+ 平移 |
| 双指点击 | 撤销（iOS 惯例）；三指点击重做 |
| 长按 | 上下文菜单（对象上）/ 切换临时橡皮（空白处，可配） |
| 笔悬停（支持设备） | 笔尖光标预览 |

防误触：以 `TOOL_TYPE_STYLUS`/`UITouch.type == .pencil` 为准——笔激活窗口内（笔 down 起至 up 后 300ms）丢弃全部手指触摸；软件级兜底：大面积接触（touch major 超阈值）+ 非预期轨迹起点的掌纹过滤。书写中双指手势打断当前笔画（cancel，不提交）。

### 3.8 降级策略表

| 能力 | 首选 | 降级 |
|---|---|---|
| Android 前置缓冲 | Android 13+ `GLFrontBufferedRenderer` 语义 | 普通双缓冲 + 预测点补偿（接受 +1 帧延迟） |
| iOS 低延迟提交 | `CAMetalLayer presentsWithTransaction=false`（接受理论撕裂风险，实测中未见——M1 用高速摄像机回归验证，若出现撕裂则回退事务提交） | 事务提交（+0~8ms） |
| wgpu 后端 | Vulkan/Metal/DX12 | wgpu GL 后端（OpenGL ES 3.0） |
| 运动预测 | Android 14 系统预测 API | androidx MotionEventPredictor 库 → 无预测 |

---

## 4. pdfium-engine PDF 底座

**引擎裁决：PDFium（Apache-2.0）**；MuPDF 因 AGPL 否决；商用 SDK 留 Provider 接缝作企业版升级位。集成：`pdfium-render` crate + bblanchon 预编译库。**线程模型见 ADR-005（句柄池 + 单写线程）**。

- 能力映射：调研 03 §7 全表（渲染/Ink 注解/页面管理/扁平化/文本层/书签）。
- 批注 = 混合方案 C：编辑时自定义图层；导出时标准兼容走 Ink Annotation，精确还原（压感变宽）走 Path Object 写内容流，最后 `FPDFPage_Flatten()`。
- 大文档：tile 渲染（512×512、4 级 LoD）+ 位图 LRU（3-6 页）+ 视口裁剪；指标目标 100 页滚动 58FPS / 内存 <100MB，**M3 以 PDFium 实测校准**（ADR-005）。
- 文本层：`FPDFText_*` 支持选择/复制/全文搜索；搜索接入全局 FTS（PDF 文本入库索引）。

---

## 5. 文件管理、对象与同步

### 5.1 信息架构

树形无限嵌套文件夹 + 收藏 + 标签（多对多）+ QuickNote + 未归档缓冲区 + 全局搜索（SQLite FTS5；中文经 jieba 分词插件；手写内容 OCR 索引留远期接口）。ER 模型见调研 04 §6.1：`Folder / Notebook / Page / Tag / SyncRecord / SyncLog / NoteVersion`。

### 5.2 页面对象模型（v2.0 补全：文本框/图片/模板）

页面上除笔画外的对象统一为 `PageObject`（与笔画同层、参与选择与 undo）：

```rust
enum PageObject {
    TextBox { id, rect, rich_text: Vec<TextSpan>, font: FontRef, size, color },
    Image { id, rect, asset_path: String, transform },
    Link  { id, rect, target: LinkTarget },  // PageRef(page_id) | Url(String)
}
```

- **文本框**：Flutter overlay 承担编辑态（输入法/光标/选区交给平台），确认后光栅化元数据存对象树，渲染态由 wgpu 文本管线（`glyphon`，cosmic-text 生态）绘制；导出 PDF 时优先写为真文本对象（`FPDFPageObj_NewTextObj` + CJK 字体子集嵌入）。
- **图片**：粘贴/拖入/拍照三路入口 → 写入 `.nbk/assets/`（WebP，>2MB 原图另存原件）→ 页面内自由变换对象。
- **页面模板/背景**：`background: Solid(color) | Template(TemplateId) | Pdf(page_index)`；模板 = SVG 描述（横线/方格/点阵/康奈尔），渲染为第 0 层；模板本身是内容插件，模板市场远期开放。
- **超链接**：文本框内链接 + 独立链接热区对象；PDF 导入文档保留原链接。
- **大纲/书签**：PDF 原生书签只读呈现；用户书签为独立实体（`Bookmark{page_id, y_offset, title}`），导出时合并写回 PDF outline。

### 5.3 `.nbk` 文档格式与版本迁移

```
note.nbk (ZIP)
├── manifest.json        # version, 页面顺序, 背景, 图层, 对象索引
├── document.pdf         # 导入的原始 PDF（嵌入，非引用）
├── pages/page_001.json  # 页面元数据 + 对象树
│       page_001.strk    # 二进制笔画（~14B/点）
├── undo_log.bin         # 撤销日志（checkpoint 机制）
├── thumbnails/          # 1x/2x WebP 渐进缩略图
└── assets/  audio/
```

**版本迁移**：`manifest.version` 单调递增；迁移器为内部模块 `nbk-migrate`，注册 `Vec<Migration{from, to, fn}>` 链式执行（v1→v2→v3）；打开旧版文件时先复制备份（`note.nbk.bak`）再就地迁移；**禁止旧代码打开新格式**（version 大于已知上限 → 只读模式打开 + 升级提示）。同步场景下未迁移副本与已迁移副本视为冲突双版本。

### 5.4 自动保存与崩溃恢复（v2.0 新增，P0）

触发策略（取先达者）：笔画提交后 3s 空闲 / 距上次保存 30s / 页面切换 / 应用退后台 / 手动 Ctrl+S。保存 = 仅写变更页文件 + manifest + undo checkpoint（增量）。异常退出判定：`manifest.json` 中 `clean_shutdown` 标志——启动置 false，正常退出置 true；启动见 false → 提示恢复，重放 undo_log 到最近 checkpoint。编辑中产生的临时态（未提交笔画）不可恢复，属可接受损失。

### 5.5 WebDAV 同步（sync-webdav 预装插件）

- 客户端：评审指出 `reqwest_dav` 成熟度存疑（star 极少）→ 裁决为 **`reqwest` + 自实现 WebDAV 方法子集**（PROPFIND/GET/PUT/DELETE/MKCOL/MOVE，约 300 行，XML 用 `quick-xml`），消除供应链风险。
- 算法：六态状态机（调研 04 §4.3 全量伪代码）；`etag + mtime` 双校验、`If-Match` 条件写入（412→冲突）。
- 冲突：LWW（**毫秒级**时间戳 + 设备 ID 作 tie-breaker；检测到本地时钟回拨 >5min 时禁用 LWW 退化为保留双版本）+ 冲突副本 `note (conflict 2026-10-12 16-38 dev-A).nbk`，文件库内横幅提示用户手工合并。
- 墓碑 30 天；坚果云并发 ≤4、Nextcloud 分块 4MB、群晖写后校验 size。
- 调度：iOS BGTaskScheduler / Android WorkManager；WiFi 限定、低电量降级、蜂窝仅元数据。
- Provider 启停在设置页操作；正在执行的原子操作（单次 PUT）完成后才切换，切换期间同步状态横幅展示。

### 5.6 协作远期路线

SyncLog 操作日志从 v1.0 建表埋点 → 协作期接 Yjs → 版本历史/分支期评估 Automerge 3.x。CRDT 永远以插件身份进入。协作 UI（远端光标 overlay）在 ink-engine 预留 `OverlayLayer` 渲染通道。

---

## 6. 手写笔适配层

厂商 SDK 现状与能力矩阵见调研 05：标准 API 覆盖 60-70% 基础能力；小米 Pencil Engine（预测/一笔成形/识别/侧旋）、华为 Pen Kit、Apple Pencil Pro（Squeeze/侧旋/触觉/悬停）为插件增量；三星 Remote SDK 边缘化（S25 起砍蓝牙），仅标准 API 兜底。

统一抽象（§调研 05 §4.3 全量接口）：`trait StylusDriver: Service`（vendor / capabilities 位图 / 输入流 / 按键流 / smart features Option）。

- `stylus-generic`（预装）：标准 API 兜底；
- 厂商适配器（`stylus-xiaomi` 等）：编译进主二进制（ADR-003），运行时按设备探测**激活**并替换 generic provider——依赖方（预测、一笔成形 UI）经 inject 响应式自动重载（§2.2 不变量 3 的核心应用场景）；
- UI 一律查询 capabilities 位图显隐功能入口，不判断具体厂商。

---

## 7. 外部扩展（WASM）与插件开发规范

### 7.1 WASM 沙箱与 Host Capability Interface（M-1 修复）

wasmtime 嵌入；内存上限（默认 64MB）+ fuel 计量 + manifest 声明式权限（fs/net/clipboard）。WASM 无法触达 GPU/平台 API，一切能力经 **Host Function ABI**：

```rust
// host 注入的 capability 函数（示例）
ink_register_brush(name_ptr, name_len, wgsl_ptr, wgsl_len) -> u32   // 提交 WGSL 着色器
ink_on_stroke_committed(callback_fn)                                 // 订阅事件
storage_read(path_ptr, path_len, out_ptr) -> i32                     // 权限内文件读
stylus_capabilities() -> u64                                          // 能力位图
```

ABI 版本化（`inkweave_abi_v1`），扩展 manifest 声明所需 ABI 版本，不兼容拒绝加载。ABI 设计是 M6 显式工作项（约 2-3 周），M0–M5 期间外部扩展通道冻结，第三方需求以内部模块形式先行验证。

### 7.2 插件清单（内部模块与外部扩展共用 schema，形态字段不同）

```toml
[plugin]
name = "shape-recognizer"
version = "0.1.0"
kind = "wasm"                    # wasm | internal
entry = "shape_recognizer.wasm"
abi = "inkweave_abi_v1"

[inject]
required = ["stylus", "ink"]

[permissions]
network = false
filesystem = "none"

[contributes]
commands = [{ id = "shape.toggle", title = "一笔成形" }]
events_listen = ["ink/stroke-committed"]
```

插件守则红线：禁止绕过 ctx 持有其他插件实例；`apply()` 禁重活；waterfall 观察后必须 `next()`；一切注册经 ctx 记账。完整可运行的 WASM 扩展示例（激光笔）见 `docs/examples/laser-pen/`（M6 交付物）。

---

## 8. 插件管理与运行时治理（v2.1 新增 · 六项要求唯一落点）

> 本章合入六项新要求，一一对应：**添加插件入口**（§8.4）·**插件热加载与关闭**（§8.2 / ADR-007）·**统一控制面板**（§8.3 / ADR-008）·**Material Design**（ADR-006 / §8.3）·**手写笔与 WebDAV 预置插件**（§8.7）·**后台机制与内存管理**（§8.5 / §8.6）。

### 8.1 插件形态、装载边界与新增服务

新需求落地前，先钉死"哪些插件能动态进出"：

| 形态 | 编译期 | 运行期能力 | 分发 |
|---|---|---|---|
| 内部模块（第一方 crate） | 由 `Cargo feature` 决定是否编入 `libinkweave` | 启 / 停（挂载 / 卸载 Fiber） | 随宿主发版 |
| **原生动态库（cdylib）** | 不参与编译 | **装入 / 卸载 / 启 / 停** | 应用内「添加插件」导入 `.so` / `.dll` / `.dylib`（ADR-003 v2.3/v2.4） |
| WASM 外部扩展（第三方） | 不参与编译 | **装入 / 卸载 / 启 / 停** | 应用内「添加插件」导入 `.wasm` / `.inkext` |

- **第一方功能（ink / pdf / storage / sync / stylus / plugin-manager）默认启用，但不再要求"必须编译进宿主"**：笔记功能（ink-engine）按用户要求改为**用户导入并启用**的形态，交付为原生动态库（见 ADR-003 v2.3 修订）；平台适配等无法动态加载的部件仍保持静态链接。**第三方 WASM 扩展**的按需装入策略不变。
- **动态分发有两条通道，适用面不同**：WASM 通道承接需强沙箱的纯逻辑扩展（M6）；原生通道承接渲染型能力（WASM 触达不了 GPU，§7.1）。两者**共用同一套四动词与同一个控制面板**，卡片上标注通道——原生通道**无沙箱**，这一点必须对用户可见（ADR-003 v2.4）。
- **新增 Definition 层服务 `PluginRegistry`**（`inkweave-api`）：

```rust
#[async_trait]
pub trait PluginRegistry: Service {
    fn snapshot(&self) -> Vec<PluginInfo>;                 // 控制面板数据源（无锁读 ArcSwap 快照）
    fn state(&self, id: PluginId) -> FiberState;
    async fn enable(&self, id: PluginId) -> Result<()>;
    async fn disable(&self, id: PluginId) -> Result<()>;
    async fn install(&self, pkg: ExtensionPackage) -> Result<PluginId>;   // WASM / 原生
    async fn uninstall(&self, id: PluginId) -> Result<()>;                // WASM / 原生
    fn subscribe(&self) -> Stream<FiberStateChange>;       // 经 Lifecycle Bridge 推给 Dart
}

pub struct PluginInfo {
    pub id: PluginId, pub name: String, pub version: String,
    pub kind: PluginKind,            // Internal | Wasm | Native
    pub state: FiberState,
    pub capabilities: StylusCapabilities,   // 能力位图（UI 据此显隐入口）
    pub permissions: Permissions,           // fs/net/clipboard
    pub memory_budget_mb: u32, pub memory_used_mb: u32,
    pub background_tasks: u32,
}
```

`PluginRegistry` 是 **Definition**：Provider 是内核的 registry 服务，Consumer 是 `plugin-manager` 模块与设置页。其它插件**只读消费**，不得直接改他人状态。

### 8.2 生命周期与「热加载 / 关闭」契约（ADR-007）

统一四动词，映射到既有 Fiber 六态（§2.1）：

| 动词 | 语义 | Fiber 迁移 | 触发 |
|---|---|---|---|
| **Load** | 装入（内部模块=启动即装入；WASM=从扩展目录读取并挂载） | `Pending → Loading → Active` | 宿主启动 / 「添加插件」 |
| **Enable** | 运行时启用（可能从 `Pending` 等依赖就绪） | `Pending/Disposed → Loading → Active` | 控制面板开关 |
| **Disable** | 运行时停用（保留扩展包，可再启用） | `Active → Unloading → Disposed` | 控制面板开关 |
| **Unload** | 卸载并删除扩展包（仅 WASM） | `Disposed` + 删除 `extensions/<id>/` | 控制面板「卸载」 |

**契约细则**：

1. **即时生效、无需重启宿主**：`Enable/Disable` 走 `ctx.plugin()` / `fiber.dispose()`，UI 不进入 loading 遮罩之外的阻塞态。
2. **停用/卸载的强制回收清单**（内核保证，插件作者不手写）：全部 `ctx.effect()` disposer（LIFO）→ 事件监听、服务注册、命令/菜单贡献 → 关联的后台任务（§8.5）→ 依赖方级联重启（§2.2 不变量 3）→ 释放 WASM 实例内存。
3. **失败隔离**：任一插件进入 `Failed` 只影响自身；控制面板以错误卡片 + 重试呈现（§8.3），主进程与兄弟插件不受影响（§0 崩溃率指标）。
4. **不做代码热替换**：换版本 = `Unload → Load` 新包，新 Fiber 从干净状态启动，不做状态迁移（与 HMR 既有结论一致，ADR-007）。
5. **UI 侧配合 Lifecycle Bridge**（§2.4）：用户点「停用」→ 立即进入 `Unloading` 展示态 → 待 `Stream<FiberStateChange>` 确认 `Disposed` 才落定为已停用。

### 8.3 统一控制面板（ADR-008）

`plugin-manager` 内部模块贡献的面板是**唯一**插件治理入口（Material 3，ADR-006）。

**信息架构**：

```
控制面板（plugin-manager 贡献的 M3 视图）
├── 顶部：已启用/已装入统计 · 全局搜索 · 「添加插件」FAB
├── 分组：预置插件（内部模块） | 已装入扩展（WASM） | 已停用
└── 插件卡片（M3 Card）
     ├── 名称 · 版本 · 来源标签（预置 / 第三方）· 形态（内部模块 / WASM）
     ├── 状态徽标：Pending / Loading / Active / Failed / Unloading / Disposed
     ├── 开关 Switch（Enable / Disable，即时生效）
     ├── 能力位图：有则显示（预测 / 一笔成形 / 识别 …），无则隐藏对应入口
     ├── 权限清单：网络 / 文件 / 剪贴板（第三方扩展可点开查看与撤销）
     ├── 资源观测：内存占用(上限) · 后台任务数 · 最近错误
     ├── 界面入口：该插件注册的命令 / 设置项（由宿主以 M3 组件渲染）
     └── 操作：启用 / 停用 / 卸载（WASM）/ 查看日志
```

**约束**：

- **不得自建管理界面**：任何插件（含第三方）都不产出自己的管理 UI，只声明贡献点，由宿主统一渲染 → 保证 M3 一致性与"统一"语义。
- **自我降级**：`plugin-manager` 自身可被停用，但此时保留一个最小只读视图，防止用户把自己锁死（停用面板后仍可重新启用）。
- **快照取数**：卡片数据来自 `PluginRegistry::snapshot()`（ArcSwap 无锁读快照，ADR-004），状态变更经 `subscribe()` 增量刷新，面板冷开不做重活（§0 指标：< 300ms）。
- **危险操作二次确认**：卸载（删除扩展包）走 `AlertDialog` 确认；停用同步类插件时提示"将取消其后台任务"。

### 8.4 「添加插件」入口（面向 WASM 与原生外部扩展）

入口位置：控制面板 FAB（§8.3）。**导入流程**（按选中文件扩展名分流，两条通道共享同一套权限确认与挂载路径）：

1. 文件选择（Flutter `file_picker`）：`.wasm` / `.inkext` → WASM 通道；`.so` / `.dll` / `.dylib` → 原生通道。宿主对扩展名做权威校验，选择器侧不做 MIME 过滤（Android 的扩展名→MIME 映射不可靠，被拒的选中好过打不开的选择器）。
2. **取得 manifest 并二次确认**：
   - WASM：读包内 `manifest.toml`（§7.2 schema），校验 `kind = "wasm"`、`abi` 与 `inkweave_abi_v1` 兼容、`version` 合法、签名有效（M6 起强制，M0–M5 开发期放行并标注"未签名"）。
   - 原生：**从二进制内部**探测 manifest（ADR-003 v2.4）——此步会运行库的初始化段但不调用任何生命周期函数；对话框据插件**真实申请**的权限填表，并同时标注"同进程运行，无沙箱"。
   - 两个通道都必须让用户逐项确认 `permissions`（network / filesystem / clipboard）；**未勾选项即拒绝装入**，而不是"以更窄的授权继续运行"。
3. 落入宿主私有存储 `extensions/<id>/`：原生通道还要在挂载前**复核副本与第 2 步清单逐字段一致**（ADR-003 v2.4 第 3 步）。
4. 调用 `PluginRegistry::install()` → `ctx.plugin()` 挂载 → 状态入 `Active`（或 `Pending` 等依赖）。**失败必须回滚**：不留卡片、不留磁盘残留。
5. 控制面板实时出现该扩展卡片，可立即停用或卸载；卡片标注通道（WASM 扩展 / 原生扩展）。

**拒绝条件**（任一命中即拒，给明确原因，不落盘）：manifest 缺失/非法、ABI 版本不兼容、签名校验失败（强制期）、权限未被全部接受、副本与确认时不一致、内存预算超过上限、`id` 与已装入冲突。

**内部模块不走此入口**：它们编译期已组合，其"启用/停用"在控制面板内完成，不涉及文件导入。

### 8.5 后台机制（BackgroundScheduler 统一层）

**唯一通道原则**：插件（内部模块与 WASM 扩展一律）**禁止**自行创建线程池、`AlarmManager`、定时器；一切后台工作经宿主 `BackgroundScheduler` 服务。

- **底层实现**：Android `WorkManager`（`sync-webdav` 已用，§5.5）；iOS `BGTaskScheduler`（`background-fetch` / `background-processing`，~30s 预算）；桌面为进程内调度器。三者由平台适配层统一封装。
- **声明式请求**：任务须声明 唯一名、约束（仅 Wi-Fi / 低电量禁跑 / 充电中 / 蜂窝仅元数据）、退避策略、是否加急。
- **与 Fiber 绑定**：`Disable` / `Unload` 时内核**自动取消**该插件名下全部任务（按 `plugin:<id>` 标记）——杜绝"僵尸任务"，属 §8.2 回收清单的一项。
- **长任务**：大文件同步等超过后台预算者转**前台服务 + 常驻通知**，通知渠道由宿主统一创建。
- **观测**：控制面板显示每插件的后台任务数（`PluginInfo.background_tasks`）。
- WASM 扩展的后台意图同样只登记为 `BackgroundScheduler` 请求（经 Host Function ABI），**扩展自身不得触达平台调度 API**。

### 8.6 内存管理（MemoryGovernor 统一层）

**预算**：`PluginInfo.memory_budget_mb`——内部模块按 manifest 声明；WASM 扩展默认上限 **64MB**（§7.1），超限即触发扩展内 GC 或终止其 Fiber（`Failed`）。

**压力广播**：宿主把系统内存压力映射并广播给全部活动插件：

| 压力源 | 广播 | 插件应做 |
|---|---|---|
| Android `onTrimMemory(RUNNING_LOW)` / iOS memory warning | `MemPressure::Low` | 释放可重建缓存（tile 位图、缩略图、索引页） |
| `TRIM_MEMORY_UI_HIDDEN` / App 退后台 | `MemPressure::Background` | 停动画、释放画布 offscreen 与预览 |
| `TRIM_MEMORY_COMPLETE` / LowMemory | `MemPressure::Critical` | 释放一切可重建资源；**宿主卸载空闲且非用户常驻的插件** |

**具体回收对象**（既有设计复用，不新增机制）：ink 只渲染视口 + LOD（§3.1）；PDF tile 位图 LRU，低内存档位缓存页数 2、常规 4–6（§4）；缩略图 L1/L2/L3 分级（§5 / 调研 04 §3）。

**观测**：`PluginInfo.memory_used_mb` 由 MemoryGovernor 采样汇总，控制面板呈现；宿主堆占用作为面板顶部汇总项。

### 8.7 预置插件清单（可开关）

预置插件 = 随宿主分发的内部模块，**在控制面板中与第三方扩展同权同列**，均可运行时启停：

| 插件 id | 形态 | 默认 | 启停效果 | 依据 |
|---|---|---|---|---|
| `plugin-manager` | 内部模块 | 启用 | 停用 → 保留最小只读视图（§8.3） | ADR-008 |
| `stylus-generic` | 内部模块 | 启用 | 停用 → 移除输入监听、释放画布缓存 | §6 |
| `sync-webdav` | 内部模块 | 启用 | 停用 → 取消全部同步任务、清内存中的凭据 | §5.5 |
| `ink-engine` / `pdfium-engine` / `storage-sqlite` | 内部模块 | 启用 | 依赖方级联（§2.2 不变量 3） | §3/§4/§5 |

**手写笔系统（预置插件）**：`stylus-generic` 是 `trait StylusDriver: Service` 的兜底实现，标准 API 覆盖 60–70% 能力（调研 05）。厂商适配器（`stylus-xiaomi` 等）编译进主二进制，**运行时按设备能力位图激活并替换 generic provider**——这正是 §2.2 不变量 3「inject 响应式」的核心应用场景。UI 一律查能力位图显隐入口，不判断厂商。停用后宿主不注册笔输入监听、释放画布位图缓存。

**WebDAV 同步（预置插件）**：`sync-webdav` 实现六态状态机（`IDLE → CHECK_LOCAL → CHECK_REMOTE → DIFF_CALC → {UPLOAD|DOWNLOAD|CONFLICT} → APPLY → VERIFY → CLEANUP → IDLE`），`etag + mtime` 双校验、`If-Match` 条件写（`412` → 冲突）；并发 ≤ 4（坚果云 4 线程起 429）；墓碑保留 30 天；调度经 §8.5（Wi-Fi 限定）。凭据存平台安全存储（Android Keystore / iOS Keychain），停用时清内存副本。详见 §5.5 与调研 04 §4。

### 8.8 验收与测试（本章）

| 项 | 手段 | 通过线 |
|---|---|---|
| 启停/装卸正确性 | cordis-rs 单测 + disposer 审计（§9.2） | 卸载后注册表、监听器、后台任务、内存全部归零 |
| 热加载时延 | 控制面板操作端到端计时 | 内部模块 < 200ms；WASM 装入/卸载 < 800ms（§0） |
| 失败隔离 | 注入 `apply()` panic / WASM trap | 宿主与兄弟插件不受影响，面板显示 `Failed` + 重试可用 |
| 权限门禁 | 未声明权限的扩展调用受限 Host Function | 返回 `Err`，不 panic，不越权 |
| 内存治理 | 内存压力注入 | 缓存回收生效；`Critical` 下空闲插件被卸载 |
| 后台取消 | 停用同步插件 | 名下 WorkRequest 全数取消 |
| 面板无障碍/暗色 | M3 检查表 + 动态字号 | 触控 ≥48dp、语义标签齐全、暗色画布可读 |

---

## 9. 工程规范（v2.0 新增）

### 9.1 monorepo 结构

```
inkweave/
├── crates/
│   ├── cordis-rs/          # 微内核（Context/Fiber/Effect/事件）
│   ├── inkweave-api/       # Definition 层：全部服务 trait + 数据类型
│   ├── ink-engine/         # wgpu 渲染 + 笔画管线 + 工具（笔/橡皮/选择）
│   ├── pdfium-engine/      # PDF 句柄池 + tile 渲染 + 编辑 + 导出
│   ├── storage-sqlite/     # SQLite+FTS5 + .nbk 读写 + undo log
│   ├── sync-webdav/        # WebDAV Provider
│   ├── stylus-drivers/     # generic + 厂商适配器
│   ├── plugin-manager/     # 统一控制面板 + PluginRegistry Provider（§8.3）
│   ├── ext-loader/         # wasmtime 扩展加载器 + ABI
│   └── nbk-migrate/        # 格式版本迁移
├── app/                    # Flutter 应用（UI 模块、PlatformView 桥）
│   ├── lib/modules/        # Dart 侧 UI 模块（library/, editor/, settings/）
│   └── plugins/            # flutter_rust_bridge 生成绑定
├── extensions/             # 第一方 WASM 扩展示例（laser-pen/）
├── docs/
│   ├── adr/                # ADR-001~005+ 持续登记
│   └── examples/
└── research/               # 调研底稿（本包）
```

### 9.2 测试策略

| 层 | 手段 |
|---|---|
| cordis-rs 单测 | 插件装/卸/依赖等待/失败隔离/级联重启；**disposer 审计**：卸载后注册表、监听器、内存全部归零 |
| 算法单测 | One Euro/重采样/网格化/碰撞检测，含 property-based（proptest） |
| 渲染正确性 | **golden image**：固定 StylusData 序列离屏渲染 → 像素 diff（容差 ≤1%，抗锯齿实现差异）；CI 三后端（Vulkan/Metal/DX12 via lavapipe 等软渲染） |
| 集成测试 | POC 延迟测量脚本化；PDF 导出后用独立解析器回读验证 |
| 同步测试 | 本地起 mock WebDAV 服务 + 故障注入（断网/412/慢速）场景矩阵 |
| 性能回归 | CI 基准：10000 点笔迹渲染、100 页 PDF 滚动、FTS 查询；超基线 15% 报警 |

### 9.3 CI/CD（发布前补全）

构建矩阵 6 平台 × debug/release；PDFium 用预编译库按平台缓存；Android APK/AAB 签名、iOS TestFlight、桌面安装包（MSIX/dmg/AppImage）统一版本号（semver，Cargo workspace 与 pubspec 同步脚本）。

### 9.4 可观测性

`tracing` 结构化日志（插件名/target 隔离，热路径编译剥离）；崩溃上报 Sentry（Rust panic + Flutter error + WASM trap 三路）；debug 构建内置帧率/延迟 HUD（ink-engine overlay 渲染，P2）。

### 9.5 已知技术债务（明示不隐瞒）

1. Web 端为降级模式：无 wgpu 画布时退 Canvas 2D + perfect-freehand 算法，延迟目标豁免。
2. 移动端无**代码级**插件热重载（HMR），调试扩展靠桌面端宿主先行。**注意：这不等于「插件热加载/关闭」缺失——运行时装/卸 + 启停由 §8.2（ADR-007）保证，两者是不同概念**。
3. inject 响应式不含存储热切换（ADR 化降级）。
4. 撤销日志跨设备不同步（每台设备独立 undo 栈）。
5. PDFium iOS 上架合规未实测，M3 预检。

---

## 10. 里程碑路线图（v2.1 修订：POC 前置 + P0 功能补齐 + 插件治理）

```
        ┌─────────┐
        │ Spike-0 │  延迟 POC（ADR-002 门禁，2 周）：PlatformView 输入路径实测
        └────┬────┘
             ▼
   ┌──────────────┐
   │ M0 骨架(5周)  │ cordis-rs(ArcSwap 版) + frb 桥 + Flutter 壳 + 遥测
   └──────┬───────┘
          ├──────────────┐
          ▼              ▼
   ┌────────────┐  ┌────────────┐
   │ M1 能写(6周)│  │ M2 能管(4周)│  （并行）
   │ 笔迹+压感   │  │ SQLite文件库 │
   │ +撤销+橡皮擦│  │ +.nbk+模板   │
   │ +选择工具   │  │ +文本框+图片 │
   └─────┬──────┘  └─────┬──────┘
         ▼               ▼
   ┌────────────┐  ┌────────────┐
   │ M3 PDF(6周) │  │ M4 同步(4周)│
   │ 句柄池+tile │  │ WebDAV 插件 │
   │ +导出+图层  │  │ +墓碑+冲突   │
   └─────┬──────┘  └────────────┘
         ▼
   M5 笔生态（持续）→ M6 WASM ABI + 扩展示例（持续）
```

| 阶段 | 关键交付 | 验收标准（新增项加粗） |
|---|---|---|
| **Spike-0**（2 周） | 最小 App 双端延迟实测 | Android/iOS touch-to-pixel ≤ 预算 130%；否则切换备选架构 B 并重估 M1 |
| **M0**（5 周） | cordis-rs（ADR-004 无锁版）+ 生命周期桥 + **`plugin-manager` 控制面板骨架（可启停预置插件，M3）** | 单测全绿 + disposer 审计零泄漏 + frb stream 状态同步正确 + **控制面板启停预置插件正确且耗时达标（§0）** |
| **M1**（6 周） | 笔/荧光笔/**橡皮擦(双模式)**/**撤销重做**/**套索选择** + 压感 + 预测 | 延迟达标（设备矩阵 §0）；**undo/redo 200 步跨会话恢复；笔画擦除命中率 100%（测试笔画集）** |
| **M2**（4 周） | 文件库 + `.nbk` + 缩略图缓存 + **页面模板 + 文本框 + 图片插入 + 自动保存/崩溃恢复** | 千本列表 60FPS；**崩溃恢复重放一致；FTS 50 万条 <50ms** |
| **M3**（6 周） | PDF 句柄池 + tile + 批注/页面管理 + 扁平化导出 + **图层系统** | 100 页滚动 ≥52FPS（58 的 90% 底线）内存 <100MB；导出件 Acrobat 可见笔迹；**线程竞争 sanitizer 零告警** |
| **M4**（4 周） | WebDAV 插件 + 后台调度 | 三家服务端回归通过；断网-恢复零丢失；**冲突副本正确生成**；**停用同步插件后其后台任务全数取消** |
| **M5**（持续） | stylus-xiaomi/apple + 一笔成形/预测 | 能力位图降级正确；小米 Pad 7 成形可用 |
| **M6**（持续） | WASM ABI v1 + 权限沙箱 + 激光笔示例 + **「添加插件」装入/卸载入口（§8.4）** | 纯 WASM 扩展**经控制面板装入/卸载正常、权限门禁生效** |

遗漏→归属明细：撤销/橡皮/选择(M1)、文本框/图片/模板/自动保存(M2)、图层(M3)、无障碍适配(M2 UI 模块同步要求)、i18n 框架(M2)、暗色模式画布适配(M1 末)、导入导出矩阵(M4)、FTUE(M4 末)。**v2.1 新增归属：统一控制面板与预置插件启停(M0)、后台调度统一层(M4)、内存治理层(M3–M4 随引擎落地)、「添加插件」WASM 装入/卸载(M6)**。

---

## 11. 风险登记册（v2.1 更新）

| 风险 | 等级 | 缓解 |
|---|---|---|
| PlatformView 输入/合成延迟不达标 | 高 | **Spike-0 门禁前置**；备选架构 B（原生底+Flutter 透明叠加） |
| PDFium 线程模型下性能不达标 | 中 | ADR-005 方案 B（单渲染线程）兜底；M3 实测校准指标 |
| wgpu 个别设备兼容 | 中 | GL 后端回退 + 启动 capability 探测 |
| PDFium iOS 上架合规 | 中 | 官方预编译库 + App Store 扫描预检（M3） |
| 厂商 SDK 政策变动 | 中 | capabilities 抽象，无 SDK 不失核心体验 |
| frb 桥控制面开销 | 低 | 笔迹走共享 ring buffer，frb 仅控制面 |
| WASM ABI 设计返工 | 低 | ABI 冻结前只做第一方示例验证 |
| Rust 升级破坏 ABI（内部模块） | 已消除 | ADR-003 静态链接，同 toolchain 构建 |

---

## 12. 附录

### 12.1 调研底稿索引

| 文件 | 内容 |
|---|---|
| `research/01-cordis.md` | Cordis 机制、22 条借鉴、Rust 移植模式 |
| `research/02-ink-engine.md` | 延迟预算、算法伪代码、后端对比 |
| `research/03-pdf.md` | 引擎对比、PDFium API 映射、格式细节 |
| `research/04-files-sync.md` | ER 图、同步状态机、服务端兼容 |
| `research/05-stylus-arch.md` | 厂商 SDK 结论、StylusPlugin 接口、决策矩阵 |
| `research/review-1-architecture.md` | v1.0 架构评审（4 致命 + 5 重大 + 15 遗漏） |
| `research/review-2-usability.md` | v1.0 可用性评审（23 遗漏 + 一致性） |

### 12.2 ADR 索引

| # | 决策 | 状态 |
|---|---|---|
| ADR-001 | Flutter UI + PlatformView 画布 | 接受（Spike-0 验证中） |
| ADR-002 | 输入路径原生直捕获，不经 Flutter | 接受；**测量方法见 v2.2 补充**（摄像机非唯一手段；当前状态：真机标定未做，风险已接受） |
| ADR-003 | 第一方插件 = 静态 crate，弃 dylib | **v2.3 修订**：保留「不用 Rust ABI」，允许原生 cdylib（只走 C ABI）以支持用户导入笔记功能；UI 仍不可动态加载。**v2.4 补充**：原生插件「装入契约」（探测先于确认 / 复制后复核 / 挂载已确认的字节 / 失败回滚） |
| ADR-004 | ArcSwap 无锁服务解析 + Arc 互持 | 接受 |
| ADR-005 | PDFium 句柄池 + 单写线程 | 接受 |
| ADR-006 | UI 设计语言 = Material Design 3 | 接受（v2.1） |
| ADR-007 | 插件「热加载/关闭」= 运行时装/卸 + 启停，非代码热替换 | 接受（v2.1） |
| ADR-008 | 统一控制面板 = 预装内部模块 `plugin-manager` | 接受（v2.1） |

> 本文档是**活文档**：每个 ADR 变更必须回写并升版本号。
