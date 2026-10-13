# Inkweave 开发指南 · 架构评审报告

> 评审人：架构评审（洁癖型）
> 评审日期：2026-10-12
> 评审对象：`inkweave-dev-guide.md` v1.0
> 参照调研：`01-cordis.md` ~ `05-stylus-arch.md`

---

## ① 致命缺陷（必须修，否则项目有结构性风险）

### F-1：Rust 无稳定 ABI——dylib 插件方案从根基上就不成立

**问题**：指南 §2.4 将第一方性能插件定义为「Rust dylib，进程内」，第三方插件为 WASM。但 **Rust 没有稳定的 ABI**（截至 rust-lang/rfcs#600 仍未稳定）。这意味着：

1. `libink_engine.so` 用 Rust 1.82 编译，`libpdfium_engine.so` 用 Rust 1.85 编译，两者在同一进程内通过 `dlopen` 加载时，**结构体布局、trait object 表示、panic  unwind 约定可能不同**，轻则段错误，重则静默数据损坏。
2. 即使锁定同一编译器版本，`-C opt-level` 或 `panic = "abort"` vs `"unwind"` 的配置差异也会导致 ABI 不兼容。
3. 指南声称 `plugin-loader` 支持 `dylib` 和 `WASM` 两种形态——但 dylib 形态在 Rust 生态中**没有可落地的方案**，除非所有 dylib 都严格用同一 toolchain + 同一 profile 编译，且仅暴露 `extern "C"` 接口。

**依据**：
- Rust RFC 600（`stable_abi` trait）状态：Deferred，无时间表
- `abi_stable` crate（crates.io 上唯一成熟方案）要求所有类型都实现 `StableAbi` trait，与指南中 `trait Service: Send + Sync + 'static` 的定义**完全不兼容**——`Service` 使用 `dyn Service` trait object，而 `abi_stable` 要求用 `DynTrait<RBox<()>, Trait_TO>` 替代
- 来源：https://github.com/Aaron1011/abi_stable_crates / https://github.com/rust-lang/rfcs/pull/600

**修法**：
- **方案 A（推荐）**：第一方插件放弃 dylib，改为 **静态链接的 Rust crate**，通过 Cargo feature flag 控制启用/禁用。仅第三方插件走 WASM。
- **方案 B**：如果坚持 dylib，必须引入 `abi_stable` crate 重写全部跨 crate 接口，`Service` trait 需改为 `#[sabi_trait]` 标注，代价是放弃标准 Rust trait 人体工学。
- **方案 C**：dylib 仅暴露 `extern "C"` 函数指针表（vtable 手动管理），类似 Lua C API 模式。开发成本高但 ABI 稳定。

---

### F-2：热路径输入事件穿越 PlatformView 边界——延迟目标 28ms 可能无法达成

**问题**：指南 §1 ADR-001 的架构是：Flutter UI → PlatformView 承载原生 Surface → Rust/wgpu 独占渲染。但**输入事件的路径**没有充分论证：

1. **Android 路径**：`MotionEvent` → Android View 系统 → PlatformView 的原生 Surface → （如何到达 Rust？）→ ink-engine 处理 → wgpu 渲染。如果事件先到 Flutter 引擎再转发给原生 Surface，则调研 02 §1.4 明确指出的 Flutter iOS `SmoothPointerDataDispatcher` 额外 8-16ms 延迟**依然存在**。
2. **iOS 路径**：`UITouch` → `UIView`（PlatformView 的原生视图）→ 需要某种方式将事件传给 Rust。如果 Rust 渲染在独立的 `CAMetalLayer` 上，而事件在 PlatformView 的 `UIView` 上，两者之间的同步需要额外机制。
3. **核心矛盾**：指南声称「帧管线不经 Flutter 引擎」，但**输入事件**是否能绕过 Flutter 引擎？在 PlatformView 架构下，Android 的 `MotionEvent` 分发走 Android View 层级（不经过 Flutter），但 iOS 的 `UITouch` 在 `UIView` 层级处理——这部分确实可以不经过 Flutter。**但指南没有明确区分输入路径和渲染路径的独立性**。

**依据**：
- 调研 02 §1.4：Flutter iOS 的 `SmoothPointerDataDispatcher` 将 99% 的 pointer packet 压到下一个 VSync，增加 8-16ms
- Flutter 3.44 引入 HCPP（Hybrid Composition++），使用 Vulkan `SurfaceControl` 事务实现零拷贝合成（来源：https://flutter.dev/blog/whats-new-in-flutter-3-44），但这只解决**渲染合成**问题，不解决**输入事件路由**问题
- PlatformView TLHC 模式在 Pixel 7 上 GPU 单帧耗时 3.4ms vs 纯 Flutter 2.1ms（来源：CSDN PlatformView 性能实测），增加了 ~1.3ms 合成开销
- Android `GLFrontBufferedRenderer` 要求渲染在 `SurfaceView` 上（来源：调研 02 §1.2.1），而 PlatformView 在 Android 上的实现是 `AndroidSurfaceView`——两者能否复用同一个 EGL Context 需要验证

**修法**：
- 必须在 M1 之前做一个 **spike/POC**，测量完整的 touch-to-pixel 延迟路径，明确：
  - Android：`MotionEvent` 是否能在 PlatformView 的原生侧直接传给 Rust（通过 JNI → FFI），不经过 Flutter Dart 层？
  - iOS：`UITouch` 在 PlatformView 的 `UIView` 子类中捕获后，能否直接在主线程调用 Rust FFI？
  - 如果答案是否定的（必须经过 Flutter），则延迟预算需要重新计算，28ms 目标可能不成立
- 备选方案：放弃 PlatformView，改为**全屏原生 Surface + Flutter 叠加在上层（透明背景）**。这牺牲了 Flutter 与原生画布的布局集成便利性，但彻底消除输入路径的 Flutter 依赖。GoodNotes/Notability 均非 Flutter 应用，不存在此问题。

---

### F-3：PDFium 线程不安全——多页并行 tile 渲染方案存在数据竞争

**问题**：指南 §4 声称「tile-based 渲染（512×512 tile、4 级 LoD）+ LRU 缓存（内存 3-6 页）」，目标「100 页 PDF 滚动 58FPS」。但 **PDFium 是线程不安全的**：

1. PDFium 官方文档明确声明：`FPDF_DOCUMENT`、`FPDF_PAGE` 等句柄**不可跨线程共享**，同一文档的并发渲染需要每个线程加载独立的 `FPDF_DOCUMENT`。
2. `pdfium-render` crate 在 Rust 侧通过 `unsafe` FFI 封装，其 `PdfDocument` 类型实现了 `Send` 但**不实现 `Sync`**（来源：pdfium-render 源码 `impl !Sync for PdfDocument`），这意味着不能多线程并发渲染同一文档。
3. 指南的 tile 渲染方案如果要在多个线程上并行渲染不同页面的 tile，必须为每个渲染线程创建独立的 `FPDF_DOCUMENT` 实例——每个实例会独立解析整个 PDF 的交叉引用表，**内存开销是页面数 × 文档大小**。

**依据**：
- PDFium 源码 `fpdfview.h` 注释：`// Each FPDF_DOCUMENT is associated with one file and should not be used from multiple threads simultaneously.`
- 调研 03 §4.1 提到 iOS `CATiledLayer` 方案，但 iOS 原生方案使用 `CGPDFDocument`（Core Graphics），它是线程安全的（`CGPDFDocument` 使用内部锁）。PDFium 没有等价机制。
- pdfium-render 的 `PdfPage::render()` 方法签名要求 `&mut self`，进一步证实非线程安全

**修法**：
- **方案 A（推荐）**：实现 **PDFium 文档句柄池**——为 N 个渲染线程各分配一个独立的 `FPDF_DOCUMENT`，通过 `FPDF_LoadMemDocument` 或 `FPDF_LoadDocument` 加载同一文件的独立副本。LRU 缓存管理的是**渲染后的 tile 位图**，而非 PDFium 句柄。
- **方案 B**：单线程渲染 + 异步调度器。所有 PDFium 调用串行化到一个专用渲染线程，通过任务队列调度。牺牲并行度但避免数据竞争。对于 100 页文档，如果单页渲染 <17ms（60FPS 预算），单线程即可满足。
- 无论哪种方案，**必须在指南中显式声明 PDFium 的线程安全约束**，否则开发者会踩坑。

---

### F-4：cordis-rs Context 树 + Weak<Fiber> + RwLock 的并发模型存在死锁风险

**问题**：指南 §2.1 的 Context 定义：
```rust
pub struct Context {
    parent: Option<Arc<Context>>,
    services: RwLock<HashMap<ServiceKey, ServiceEntry>>,
    fiber: Weak<Fiber>,
}
```

1. **服务解析的锁链**：`get_service()` 需要 `self.services.read()` → 未命中 → `self.parent.as_ref()?.get_service(name)` → 父节点 `services.read()`。如果此时另一个线程正在 `provide()` 服务（需要 `self.services.write()`），则**读锁等待写锁释放，写锁等待读锁释放**——在 `std::sync::RwLock` 中，写锁饥饿是可能的（取决于平台实现）。更严重的是：如果服务解析路径上有任何一个节点正在被写入，**整条父链的读锁都会被阻塞**。
2. **Fiber 状态转换与服务注册的竞态**：Fiber 从 `PENDING → LOADING` 需要检查所有 `inject` 依赖是否就绪。如果依赖检查（读锁）和依赖注册（写锁）在不同线程并发执行，且 Fiber 状态转换也需要锁（如 `fiber.state` 的原子读写），则可能形成锁环。
3. **Weak<Fiber> 的升级失败**：`fiber: Weak<Fiber>` 意味着 Context 不拥有 Fiber。如果 Fiber 被 dispose 后 Context 仍在被使用（如事件回调中），`fiber.upgrade()` 返回 `None`，但指南没有定义这种情况下的行为——是 panic 还是静默忽略？

**依据**：
- 调研 01 §2.1 的 Cordis 原版是 TypeScript（单线程事件循环），不存在并发问题。移植到 Rust 的多线程模型时，必须重新审视锁策略。
- Rust `std::sync::RwLock` 不保证公平性（来源：https://doc.rust-lang.org/std/sync/struct.RwLock.html），写锁可能饥饿
- `parking_lot::RwLock` 提供更好的性能保证，但仍有死锁可能

**修法**：
- 服务解析路径改为**无锁设计**：使用 `ArcSwap<HashMap<ServiceKey, ServiceEntry>>` 替代 `RwLock<HashMap>`。读操作完全无锁（`ArcSwap::load()` 只读原子指针），写操作通过 `ArcSwap::store()` 替换整个 HashMap。对于服务注册频率远低于服务查询频率的场景（典型：注册在启动期，查询在运行期），这是最优方案。
- 或者：限制服务解析只在 Fiber 的 `LOADING` 阶段执行（单线程初始化序列），运行期通过预解析的 `Arc<dyn Service>` 直接访问，不再走 `get_service()` 动态查找。
- `Weak<Fiber>` 改为 `Arc<Fiber>`（Context 和 Fiber 互相持有），或明确定义 `fiber.upgrade() == None` 时的行为契约。

---

## ② 重大疑虑（建议修，不修会在后期付出更大代价）

### M-1：WASM 插件如何调用需要 GPU/平台能力的服务？

指南 §2.4 将第三方插件定义为 WASM 模块（wasmtime 嵌入），并声明「能力白名单（声明式权限：fs/net/clipboard 需在 manifest 申请）」。但**没有说明 WASM 插件如何调用 GPU 渲染或平台特定服务**。

- 笔刷插件需要注册自定义 shader（指南 §3.3：`fn fragment_shader(&self) -> &str`）。WASM 插件如何提供 WGSL/GLSL 着色器字符串？wasmtime 的 `Linker` 可以注入 host function，但着色器需要直接提交到 wgpu 的 `RenderPipeline`——WASM 模块无法直接访问 wgpu 的 `Device`。
- 手写笔厂商插件需要访问平台 SDK（如小米 Pencil Engine 的 Java/Kotlin API）。WASM 模块无法直接调用 JNI/ObjC 桥。

**修法**：定义 **Host Capability Interface**——一组预定义的 host function，WASM 插件通过 import 声明调用。例如：
- `ink_register_shader(name: &str, wgsl_source: &str)` → host 侧提交到 wgpu
- `stylus_get_capabilities() -> Capabilities` → host 侧查询平台 SDK
- 这需要在 cordis-rs 中设计一套 **WASM ABI**，是额外 2-3 周的工作量，应在 M6 里程碑中显式列出。

### M-2：Flutter UI 壳插件与 Rust Fiber 生命周期的统一

指南 §2.4 将第一方 UI 插件定义为「Dart 壳 + Flutter widget」，但没有说明 Dart 侧的 UI 插件如何与 Rust 侧的 Fiber 状态机同步。

- 当一个 Rust Fiber 从 `ACTIVE → UNLOADING` 时，对应的 Dart UI 插件需要卸载其 Widget。但 Dart 和 Rust 运行在不同的执行环境中，如何保证 UI 卸载在 Flutter 的 UI 线程上执行？
- 反过来，Dart 侧的用户操作（如点击「禁用插件」）需要触发 Rust 侧的 `fiber.dispose()`。这是异步操作，期间 UI 应显示什么状态？

**修法**：定义 **Plugin Lifecycle Bridge**——Rust Fiber 状态变更通过 `Stream<FiberStateChange>` 推送到 Dart 侧，Dart 侧的 `PluginHostWidget` 监听并响应。需要明确：
- 状态变更通知的时序保证（at-least-once? exactly-once?）
- UI 线程安全性（Flutter Widget 操作必须在 UI 线程）
- 中间状态（`UNLOADING`）的 UI 表现（loading spinner? 灰度遮罩?）

### M-3：撤销/重做（Undo/Redo）栈——指南完全遗漏

指南 §3 的渲染管线和 §5 的文档格式都没有提及撤销/重做机制。这是手写笔记应用的**核心用户功能**：

- GoodNotes、Notability、Xournal++ 全部支持多步撤销/重做
- 手写场景的撤销粒度是**笔画级**（而非字符级），需要维护笔画的插入/删除/修改操作栈
- 与 cordis-rs 的 Effect 系统有天然的对应关系（每个笔画是一个 effect，disposer 负责删除笔画），但指南没有建立这个映射

**修法**：在 §3 增加 **Ink Undo/Redo 设计**：
- 使用 Command Pattern：`InkCommand::AddStroke(StrokeId)` / `InkCommand::RemoveStroke(StrokeId)` / `InkCommand::ModifyStroke(StrokeId, Diff)`
- 与 cordis-rs 的 Effect 系统集成：每个笔画提交自动 push 到 Fiber 的 effect 栈，撤销 = 调用 disposer
- 存储：操作日志持久化到 `.nbk` 的 `undo_log.strk` 文件，支持跨会话撤销

### M-4：橡皮擦工具——指南完全遗漏

指南 §3 的笔刷系统列出了钢笔/铅笔/荧光笔/激光笔，但没有橡皮擦。橡皮擦在手写笔记中是**一等公民**：

- **笔画擦除**（Stroke Eraser）：整条笔画被擦除——需要笔画级别的碰撞检测
- **像素擦除**（Pixel Eraser）：只擦除笔尖经过的像素——需要 stencil buffer 或 clip path
- **Xournal++ 方案**：支持两种模式切换
- **Apple Pencil Pro 双击切换**：笔/橡皮擦双击切换是用户期望的交互

**修法**：在 §3.3 笔刷系统中增加 `EraserBrush`，实现 `BrushPlugin` trait。笔画擦除需要 `ink-engine` 提供空间索引（R-Tree 或 Grid）加速碰撞检测。

### M-5：里程碑工作量估计偏乐观，缺少关键依赖

| 阶段 | 指南估计 | 评审意见 |
|------|---------|---------|
| M0 骨架 | 4 周 | 偏紧。cordis-rs 的 Rust 移植（含 Effect 栈、Context 树、Fiber 状态机、事件总线）+ 测试，且需要解决 F-4 的并发设计，4 周可能不够。建议 5-6 周。 |
| M1 能写 | 6 周 | 合理，但**依赖 F-2 的 POC 结论**。如果 PlatformView 延迟不达标，需要切换架构，M1 工作量翻倍。 |
| M3 PDF | 5 周 | 偏紧。需包含 F-3 的线程安全方案实现 + tile 渲染 + 缓存策略 + 扁平化导出。建议 6-7 周。 |
| M5 笔生态 | 持续 | 缺少明确的 v1 交付标准。「小米 Pad 7 上预测/成形可用」需要与小米 SDK 团队对接，时间不可控。 |

**遗漏的关键工作项**：

| 工作项 | 重要性 | 建议阶段 |
|--------|--------|---------|
| 撤销/重做栈 | P0（核心功能） | M1 |
| 橡皮擦工具 | P0（核心功能） | M1 |
| 文本工具（打字文本框） | P1 | M2 |
| 图层系统（多图层显隐/锁定/排序） | P1 | M2 |
| 页面模板（网格/横线/空白/自定义） | P1 | M2 |
| 选择工具（套索/矩形选区 + 移动/缩放/旋转） | P1 | M3 |
| 无障碍（Accessibility） | P1（App Store 审核要求） | M2 |
| 本地化（i18n/l10n 框架） | P2 | M2 |
| 遥测/崩溃上报 | P2 | M0 |
| 导入/导出（.nbk ↔ GoodNotes/Notability/Xournal++ 格式转换） | P2 | M4 |
| WASM 插件 ABI 设计 | P2 | M6 |

---

## ③ 遗漏项清单

| # | 遗漏项 | 严重性 | 说明 |
|---|--------|--------|------|
| 1 | **撤销/重做** | 致命 | 笔记应用核心功能，完全未提及 |
| 2 | **橡皮擦** | 致命 | 手写场景一等公民，笔刷系统中缺失 |
| 3 | **文本工具** | 重大 | 所有竞品（GoodNotes/Notability/Xournal++）均支持在笔记中插入文本框 |
| 4 | **图层系统** | 重大 | manifest.json 中有 `layers` 字段但指南未描述图层 UI 交互（显隐切换、锁定、排序、合并） |
| 5 | **页面模板** | 重大 | 空白页/网格/横线/自定义模板是笔记应用标配 |
| 6 | **选择工具** | 重大 | 套索选择 + 移动/缩放/旋转/删除选中笔画 |
| 7 | **无障碍 (a11y)** | 重大 | iOS VoiceOver / Android TalkBack 适配是 App Store 审核要求 |
| 8 | **本地化 (i18n)** | 中等 | 国际化框架、RTL 支持、CJK 字体回退 |
| 9 | **遥测/崩溃上报** | 中等 | 插件崩溃隔离后需要上报机制，否则无法诊断 |
| 10 | **导入/导出格式兼容** | 中等 | 与 GoodNotes/Notability/Xournal++ 的文件互操作 |
| 11 | **墨迹形状识别（一笔成形）** | 中等 | 小米/华为均提供 SDK，指南提到但未排入里程碑 |
| 12 | **手写识别（OCR for Ink）** | 低 | 远期功能但应预留接口 |
| 13 | **多光标/协作编辑 UI** | 低 | CRDT 远期规划但未提及 UI 层面的光标渲染 |
| 14 | **暗色模式** | 低 | Flutter 原生支持但需在画布渲染中适配 |
| 15 | **性能监控/帧率 HUD** | 低 | 开发期调试工具，对延迟优化至关重要 |

---

## ④ 与调研证据矛盾处

| # | 指南声称 | 调研证据 | 矛盾性质 |
|---|---------|---------|---------|
| 1 | §1「帧管线不经 Flutter 引擎，规避 Flutter iOS 指针调度器额外 8-16ms 延迟」 | 调研 02 §1.4 指出 Flutter iOS `SmoothPointerDataDispatcher` 增加延迟。但指南的 PlatformView 方案中，**渲染路径**确实可以不经过 Flutter，**输入路径**是否经过 Flutter 未论证。 | **不完整论证**——渲染和输入是两条独立路径，指南只解决了渲染路径 |
| 2 | §2.4「第一方性能插件：Rust dylib，进程内」 | Rust 无稳定 ABI（RFC 600 Deferred）。调研 01 §5.2 的 Rust 移植方案使用 `Arc<dyn Service>` trait object，隐含了所有插件编译到同一 binary 的假设。 | **与 Rust 语言现实矛盾**——dylib 插件在 Rust 中不可行（见 F-1） |
| 3 | §4「100 页 PDF 滚动 58FPS、内存 <100MB」 | 调研 03 §4.1 的 58FPS 数据来自 iOS `CATiledLayer` 方案（使用 Core Graphics 的线程安全 `CGPDFDocument`）。PDFium 不是线程安全的，直接迁移该指标需要额外工程。 | **指标来源与引擎选型不匹配**——58FPS 数据基于 CG 而非 PDFium |
| 4 | §3.1 延迟预算「应用处理 <2ms（无 GC、无对象分配，全部对象池）」 | 调研 02 §2.3 One Euro Filter 实现使用 `numpy.array`（Python 示例），Rust 移植需要确保零分配。但指南的 `StylusData` 结构体包含 `f32` × 6 = 24 bytes，每个采样点都需要创建——对象池需要预分配 `Vec<StylusData>` 并在帧边界回收。 | **可行但需要显式设计**——指南提到了对象池但未给出具体方案 |
| 5 | §5.3「sync-webdav 使用 reqwest_dav」 | 调研 04 §4.2 列出 `reqwest_dav` 和 `rustydav` 两个选项，但 `reqwest_dav` 的 GitHub 仓库（niuhuan/reqwest_dav）star 数极少（<100），维护活跃度存疑。 | **选型依据不足**——建议使用更成熟的 `reqwest` + 手动实现 WebDAV 方法，或评估 `rustydav` |
| 6 | §0「包体积：核心 APK < 40MB」 | 调研 03 §1.1 PDFium ~3-8MB + 调研 02 §4.2 wgpu ~2-5MB + Flutter 引擎 ~8-10MB + Rust stdlib ~2MB = 15-25MB 基线。加上 cordis-rs、SQLite、字体文件、平台 SDK，40MB 目标偏紧但可行。 | **偏紧但可行**——需要严格控制依赖树 |
| 7 | §2.2「inject 响应式：storage provider 被替换时，依赖方自动 Unloading → Loading → Active，无需重启应用」 | 调研 01 §2.7 明确描述了 Cordis 的响应式共效。但笔记应用中「存储后端热切换」的实际场景极其罕见（用户不会在运行中切换本地存储到 WebDAV），为此引入复杂的生命周期级联重启是否值得？ | **过度设计嫌疑**——建议将「热切换」降级为「重启后生效」，大幅简化实现 |

---

## ⑤ 值得肯定的设计

1. **混合方案 C（PDF 批注架构）**：编辑时自定义图层、导出时双路径（Ink Annotation + Path Object），兼顾性能与标准兼容性。与 GoodNotes/Notability 的做法一致（调研 03 §3.3），是正确的工程判断。

2. **`.nbk` ZIP 容器格式**：按页拆分文件实现增量保存、嵌入 PDF 而非外部引用（吸取 Xournal++ 教训）、二进制笔迹格式（~14B/点）紧凑高效。格式设计成熟。

3. **笔刷插件化**：将笔刷定义为 `BrushPlugin` 而非枚举，第三方可注册新笔刷，与 Cordis「一切皆插件」的理念一致。荧光笔正片叠底混合防叠色（参考 Saber）是正确的细节决策。

4. **能力位图抽象（StylusDriver::capabilities）**：不假设所有设备都有相同能力，UI 根据能力位图降级显隐。这是对厂商 SDK 碎片化的正确应对。

5. **风险登记册**：显式列出 PlatformView 掉帧、wgpu 兼容性、PDFium iOS 合规等风险，并给出缓解方案。这种工程纪律值得肯定。

6. **调研覆盖度**：五份调研文档覆盖了 Cordis 框架、笔迹引擎、PDF 引擎、文件/同步、手写笔 SDK 全部关键领域，引用来源丰富，为指南提供了扎实的证据基础。

---

## 附录：评审结论摘要

| 类别 | 数量 |
|------|------|
| 致命缺陷 | **4** |
| 重大疑虑 | **5** |
| 遗漏项 | **15** |
| 与调研矛盾 | **7** |

**核心判断**：指南的**设计理念和方向正确**（Flutter + Rust 混合、Cordis 插件化、PDFium 底座），但在**工程落地层面存在 4 个致命缺陷**——Rust ABI 不稳定导致 dylib 插件方案不可行、PlatformView 输入路径延迟未验证、PDFium 线程安全约束被忽略、cordis-rs 并发模型有死锁风险。这些问题不解决，M1 里程碑的 28ms 延迟目标和 M3 的 58FPS 目标都无法可靠达成。

**建议优先级**：F-1（dylib → crate/WASM）> F-2（延迟 POC）> F-3（PDFium 线程安全）> F-4（锁策略）> 遗漏项补全 > M-1~M-5。
