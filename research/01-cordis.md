# Cordis 插件化框架调研报告

> **调研目标**：深入分析 koishijs/cordis（cordiverse/cordis）的设计哲学与实现机制，为跨平台笔记软件的插件架构设计提供指导。
>
> **调研日期**：2026-10-12

---

## 目录

1. [Cordis 概述与起源](#1-cordis-概述与起源)
2. [核心概念详解](#2-核心概念详解)
   - 2.1 Context（上下文）
   - 2.2 Service（服务）
   - 2.3 Plugin（插件）
   - 2.4 Lifecycle（生命周期：Fiber 状态机）
   - 2.5 事件模型（Events）
   - 2.6 Effect Scope / 响应式 Dispose
   - 2.7 插件依赖与依赖注入（inject）
   - 2.8 插件热重载机制（HMR）
3. [「一切皆为插件」的设计哲学](#3-一切皆为插件的设计哲学)
4. [与类似架构的横向对比](#4-与类似架构的横向对比)
5. [非 Node 环境移植方案](#5-非-node-环境移植方案)
6. [最新版本 API 概览](#6-最新版本-api-概览)
7. [对笔记软件的借鉴清单](#7-对笔记软件的借鉴清单)
8. [设计模式总结](#8-设计模式总结)
9. [参考来源](#9-参考来源)

---

## 1. Cordis 概述与起源

### 1.1 项目信息

| 属性 | 详情 |
|------|------|
| **仓库** | [cordiverse/cordis](https://github.com/cordiverse/cordis)（原 koishijs/cordis） |
| **Stars** | 9.1k（截至 2026-08） |
| **语言** | TypeScript（编译为 ESM JavaScript） |
| **最新版本** | v4.0.0-rc.8（上游）；@deepseek-ai/cordis@4.0.1（DeepSeek vendor 版） |
| **许可证** | MIT |
| **论文** | *A Programming Paradigm for Spatiotemporal Composability*（arXiv:2608.25512） |
| **文档** | [cordis-primer](https://deepseek-harness.github.io/deepseek-harness/reference/cordis-primer) |

### 1.2 历史脉络

- **2020-01**：Koishi（跨平台聊天机器人框架）发布首个版本，作者 Shigma
- **2022-04**：Cordis 作为独立 npm 包发布，从 Koishi 核心层抽出
- **2023**：进入 3.x 版本线，Shigma 发表《可逆的插件系统》设计文章（论文雏形）
- **2024-11**：Cordis 4.0 预发布，引入基于 fiber 的生命周期体系
- **2026-08**：DeepSeek Harness 发布，以 vendor 方式内嵌 Cordis，配套论文发布

> **来源**：[GitHub cordiverse/cordis](https://github.com/cordiverse/cordis)；[redreamality.com 深度分析](https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/)

### 1.3 定位

Cordis 自称 **Meta-Framework of Spatiotemporal Composability**（时空可组合性的元框架）。它不是：
- 机器人框架（Koishi 才是）
- DI 容器（传统 DI 回答"谁创建谁"，Cordis 回答"谁在什么时候活着"）
- 应用框架（不预设任何业务领域）

它是**规定"副作用如何组合、依赖如何解析"的运行时元框架**，上层框架（Koishi、DeepSeek Harness）在其上叠加领域语义。

---

## 2. 核心概念详解

Cordis 的全部语义可浓缩为**五个概念**：插件（Plugin）、上下文（Context）、注入（Inject）、事件（Event）、可逆副作用（Revertible Effect）。

> **来源**：[DeepSeek Harness 背后的"心脏"：Cordis 到底是什么](https://new.qq.com/rain/a/20260904A0BKC100)

### 2.1 Context（上下文）

Context 是 Cordis 的核心容器，但它**不是普通对象——而是一个 Proxy（代理）**。

```typescript
// 源码概念示意 (core/context.ts)
class Context {
  constructor() {
    // 用 Proxy 包装自身，所有属性读取都经过 ReflectService.handler
    const self = new Proxy<this>(this, ReflectService.handler)
    this.root = self
  }
}
```

**关键特性**：

- **服务按需查找**：当写 `ctx.tools` 时，触发 Proxy 拦截，去服务注册表查找名为 `'tools'` 的服务实例
- **层级结构**：Context 是有层级的，每个插件运行在自己的**子 Context** 中，子 Context 继承父 Context 的服务，但可以有隔离的服务实例
- **方法 mixin**：核心服务的方法被 mixin 到 ctx 上

```typescript
// Context 聚合了所有核心能力
this.mixin('reflect',  ['get', 'set', 'provide', 'accessor', 'mixin'])
this.mixin('fiber',    ['runtime', 'effect'])
this.mixin('registry', ['inject', 'plugin'])
this.mixin('events',   ['on', 'once', 'parallel', 'emit', 'serial', 'bail', 'waterfall'])
```

**服务解析流程**：

```
读取 ctx.database
  ↓
Context 是否本身拥有该属性？
  ├─ 是：直接返回
  └─ 否：将 database 视为服务名
       ↓
    在当前插件 Fiber 中查找
       ↓
    沿父 Fiber 向上查找
       ↓
    找到对应作用域内的服务后返回
```

> **来源**：[DeepSeek Harness 系列（02）：万物皆插件——Cordis 核心设计深度解读](https://m.blog.csdn.net/chendongqi2007/article/details/164881421)；[DeepSeek Harness 的基石：从源码理解 Cordis](https://www.juejin.cn/post/7675174962013257791)

### 2.2 Service（服务）

Service 是挂到 `ctx` 上的命名能力，让别的插件按名字取用。

```typescript
import { Service, type Context } from 'cordis'

// 方式一：Service 子类（需要对外暴露服务时用）
export class GreeterService extends Service {
  constructor(ctx: Context) {
    super(ctx, 'greeter') // 注册为 ctx.greeter
  }
  
  greet(name: string) {
    return `Hello, ${name}!`
  }
}

// 方式二：ctx.provide（轻量注册）
export function apply(ctx: Context) {
  ctx.provide('greet', (name: string) => `Hello, ${name}!`)
}
```

**类型安全通过声明合并实现**：

```typescript
declare module 'cordis' {
  interface Context {
    greeter: GreeterService
  }
}
```

**服务即上下文属性**：消费方不需要 `import` 提供方，通过 `ctx.<key>` 在运行时通过代理解析，提供方可任意替换。

> **来源**：[Cordis 入门 01：从零跑起第一个插件](https://m.blog.csdn.net/qq_41914181/article/details/164507546)

### 2.3 Plugin（插件）

Cordis 接受**三种形态**的插件：

```typescript
import { Service, type Context } from 'cordis'

// 形式一：函数插件（最常用）
export function apply(ctx: Context) {
  console.log('插件已加载')
}

// 形式二：对象插件
export const myPlugin = {
  name: 'my-plugin',
  inject: ['tools'],       // 声明依赖
  apply(ctx: Context) { /* ... */ },
}

// 形式三：Service 子类（需要对外暴露服务时用）
export class MyService extends Service {
  static inject = ['database']
  constructor(ctx: Context) {
    super(ctx, 'myService') // 注册为 ctx.myService
  }
}
```

**插件配置**：

```typescript
// 插件可接收配置（第二个参数）
function greetPlugin(ctx: Context, config: { name: string }) {
  console.log(`hello ${config.name}`)
}

await ctx.plugin(greetPlugin, { name: 'cordis' })
```

**插件清单（manifest）**：声明式配置文件中描述插件树：

```yaml
# cordis.yml
- name: './hello.ts'
  # 或 github:user/plugin，或 npm 包名
```

> **来源**：[DeepSeek Harness 桌面端来了！带你拆解核心机制](https://baijiahao.baidu.com/s?id=1878658585645002537)

### 2.4 Lifecycle（生命周期：Fiber 状态机）

每个被加载的插件实例都有一个对应的 **Fiber**（纤程），是 Cordis 追踪插件生命周期的核心对象。

**六态状态机**：

```typescript
// core/fiber.ts
export const enum FiberState {
  PENDING,   // 等待所需服务就绪
  LOADING,   // apply() 正在执行
  ACTIVE,    // 加载完成，正常运行中
  FAILED,    // apply() 或配置校验抛出了异常
  UNLOADING, // disposer 正在执行清理
  DISPOSED,  // 已完全卸载，不可再启动
}
```

**状态转换**：

```
PENDING → LOADING → ACTIVE → UNLOADING → DISPOSED
              ↘ FAILED
```

**完整生命周期流程**：

```
parent context mounts plugin
  → registry creates fiber and child context
  → runtime resolves declared dependencies and computes target
  → once satisfied, apply(ctx) runs
  → ctx.effect(), event listeners, services, and child plugins belong to fiber
  → fiber becomes ACTIVE
  → dispose request or dependency change starts coordinated recovery
```

**关键语义**：
- `PENDING`：等待依赖就绪，不是错误状态
- `FAILED`：已积累的 effect 会被回收，failed fiber 不再提供 coeffects
- 卸载 = **逆序启动、并发回收**总账中的全部 disposer
- 父子级联：父卸载递归卸载子树

> **来源**：[redreamality.com 深度分析](https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/)；[Cordis 框架超深度源码分析](https://m.blog.csdn.net/zhonglinzhang/article/details/164012094)

### 2.5 事件模型（Events）

Cordis 提供**五种分发模式**，每种模式有严格的语义契约：

| 模式 | 调用方式 | 是否 await | 是否返回值 | 语义 | 典型场景 |
|------|----------|------------|------------|------|----------|
| **emit** | `ctx.emit(name, ...args)` | 否 | 否 | 广播通知 | 状态变更、inbox 插入 |
| **parallel** | `await ctx.parallel(name, ...args)` | 是 | 否 | 并发观察 | session/flush |
| **serial** | `await ctx.serial(name, ...args)` | 是 | 是 | 顺序决策，首个非 null 值胜出 | turn-stopping |
| **bail** | `ctx.bail(name, ...args)` | 否 | 是 | serial 的同步版 | 权限检查 |
| **waterfall** | `await ctx.waterfall(name, ...args, next)` | 否* | 是 | 环绕中间件链，可短路 | pre-execute、request |

> *waterfall 官方文档标为"不 await"，但监听器通常是异步函数，实践中普遍 `await ctx.waterfall(...)`

#### emit：广播

```typescript
// 触发方
ctx.emit('my-plugin/ready', { id: 'worker-1' })

// 监听方（卸载时自动移除）
ctx.on('my-plugin/ready', ({ id }) => {
  console.log(`${id} is ready`)
})
```

#### bail：短路

```typescript
// 监听方：命中拦截条件就返回，否则继续
ctx.on('some-check', (input) => {
  if (shouldBlock(input)) return 'blocked'
  // 返回 null/false/undefined 表示「我没意见」
})

// 分发方
const result = ctx.bail('some-check', input)
```

#### waterfall：流水线（最重要的扩展机制）

```typescript
// 分发方：初始值传给第一个监听器
const output = await ctx.waterfall('my-plugin/transform', input, async () => input)

// 监听方：必须调用 next()，拿到下游结果后加工再返回
ctx.on('my-plugin/transform', async (_input, next) => {
  const downstream = await next()
  // 不调用 next() 直接返回 = 有意短路（否决/拦截）
  return downstream.trim()
})
```

**waterfall 铁律**：只观察/标注的监听器**必须调用 `next()`**；不调用 `next()` 直接返回 = 有意短路。

#### 类型安全的事件

```typescript
// 通过 TypeScript 声明合并注册事件
declare module 'cordis' {
  interface Events {
    'my-plugin/ready': (payload: { id: string }) => void
    'my-plugin/check': (input: string) => boolean | undefined
    'my-plugin/transform': (input: string, next: () => Promise<string>) => Promise<string>
  }
}
```

**事件命名约定**：`namespace/action`（如 `agent/step`、`tools/result`）

> **来源**：[DeepSeek Harness 事件系统：emit/bail/serial/waterfall](https://www.runoob.com/deepseek-harness/deepseek-harness-events.html)；[Cordis 插件引擎拆解](https://www.xujun.org/note-2098246928097841154.html)

### 2.6 Effect Scope / 响应式 Dispose

**核心原语：`ctx.effect()`**

Cordis 中所有对上下文的变更，最终都归结为 `ctx.effect()` 这一个原语。提供服务、挂载插件、注册监听器，全是它的特例。

```typescript
ctx.effect(() => {
  // 正向副作用（注册时执行）
  const timer = setInterval(() => { /* ... */ }, 30_000)
  
  // 返回 disposer（卸载时执行）
  return () => clearInterval(timer)
})
```

**支持的返回类型**（5 种）：
1. 同步 disposer 函数 `() => void`
2. 异步 disposer `() => Promise<void>`
3. `Disposable` 对象（含 `.dispose()` 方法）
4. `Promise<Disposable>`
5. `undefined`（无需清理）

**内置 API 本身就是 effect**：
- `ctx.on()` 的监听器随插件卸载自动移除
- `ctx.plugin()` 的子插件随父插件递归卸载
- 服务注册随提供方消失自动撤销

**Recovery Accumulator（恢复累加器）**：

Fiber 维护一个 composed record of inverses（逆操作组合记录）。卸载时按 LIFO（后进先出）逆序执行所有 disposer。

```
注册顺序: effect_A → effect_B → effect_C
卸载顺序: dispose_C → dispose_B → dispose_A
```

> **来源**：[DeepSeek Harness 底层探秘：Cordis 元框架](https://m.blog.csdn.net/aftvc_xwq/article/details/164375343)；[redreamality.com](https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/)

### 2.7 插件依赖与依赖注入（inject）

**声明式依赖**：

```typescript
export const inject = ['tools', 'llm']

export function apply(ctx: Context) {
  // 只有当 tools 和 llm 都就绪后，apply 才会被执行
  ctx.tools.register(/* ... */)
  ctx.llm.registerAdapter(/* ... */)
}
```

**响应式共效（Reactive Coeffects）**：

当依赖的服务提供方发生变化时，依赖它的插件会被**自动停用和重新激活**：

```
mount weather-tool                  → PENDING
provide tools only                  → still PENDING
provide llm-A                       → LOADING → ACTIVE
replace llm-A with llm-B            → UNLOADING → LOADING → ACTIVE
dispose weather-tool                → tool unregistered, timer cleared
```

**依赖解析不是 in-place 变量赋值**：消费者完成旧 episode、回收 effects、再针对新 provider 激活。

**类型安全**：

```typescript
// 通过声明合并让 inject 有类型提示
declare module 'cordis' {
  interface Context {
    tools: ToolsService
    llm: LLMService
  }
}
```

> **来源**：[深入 DeepSeek Harness 系列(四）：Cordis 工作原理解析](https://baijiahao.baidu.com/s?id=1875460779452889187)

### 2.8 插件热重载机制（HMR）

**HMR（Hot Module Replacement）流程**：

```
文件变更检测
  → 分类受影响模块
  → 定位 stale entries
  → 撤回旧 fibers 及其 effects
  → 导入新代码
  → 重新建立受影响的子图
```

**事务性语义**：如果新代码导入失败，恢复模块缓存和旧 fibers，**绝不进入半重载状态**。

**重要限制**：
- HMR **不是状态迁移**：旧 fiber 被回收，新 fiber 从干净状态启动
- 私有内存状态会消失，除非放在更长生命周期的服务中
- 与 Dynamic Software Updating (DSU) 不同

**Loader 配置协调**：

```
old entry tree + edited configuration
  → keyed comparison by stable id
  → unload/update/load affected fibers
  → new running fiber tree
```

Loader 比较期望的插件树与运行中的 fiber 树，只 mount/update/disable/remove 实际变更的部分。

> **来源**：[redreamality.com](https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/)；[Cordis 框架超深度源码分析](https://m.blog.csdn.net/zhonglinzhang/article/details/164012094)

---

## 3. 「一切皆为插件」的设计哲学

### 3.1 核心本身也是插件

在 Cordis/DHS 中，没有"特权核心"：

> "There is no privileged core to patch: you extend dsh by mounting a plugin beside the others."
> — DeepSeek Harness architecture.md

| 组件 | 是否插件 |
|------|----------|
| 模型适配器 | ✅ 插件 |
| 工具注册表 | ✅ 插件 |
| 会话日志 | ✅ 插件 |
| Agent Loop | ✅ 插件（`AgentLoop extends Service`） |
| 存储层 | ✅ 插件 |
| 沙箱 | ✅ 插件 |
| UI | ✅ 插件 |

### 3.2 服务即上下文属性

```typescript
// ❌ 传统做法：直接 import，强耦合
import { ToolsService } from './tools-service'
const tools = new ToolsService()

// ✅ Cordis 做法：通过 ctx 查找，解耦合
ctx.tools.register(myTool)
```

**隔离与组合的方式**：

1. **空间隔离**：每个插件拥有独立子 Context，子 Context 继承父 Context 的服务但可有隔离实例
2. **生命周期隔离**：每个插件有独立 Fiber，失败不影响兄弟节点
3. **依赖驱动组合**：通过 `inject` 声明依赖，运行时自动解析，无需手动编排启动顺序
4. **事件松耦合**：插件间通过类型化事件通信，不需要知道对方的存在

### 3.3 能力接缝（Capability Seams）三层模式

```
Service Definition  →  定义稳定的词汇表、请求/结果类型、行为契约
       ↓
Service Provider    →  实现契约（如本地 shell、沙箱 shell）
       ↓
Consumer            →  使用抽象服务（如暴露为模型可调用的工具）
```

Provider 和 Consumer 依赖 Definition，不依赖彼此。替换 backend 可以让多个 consumers 迁移到另一个执行环境而不改 import。

> **来源**：[DeepSeek Harness 源码深度分析：像 VS Code 一样插件化](https://juejin.cn/post/7673799351681286194)；[mazhen.tech 深度解析](https://mazhen.tech/p/deepseek-harness-深度解析一切皆插件的-agent-运行时/)

---

## 4. 与类似架构的横向对比

### 4.1 对比矩阵

| 维度 | Cordis | VS Code Extensions | Obsidian Plugins | Eclipse OSGi | Webpack/Tapable |
|------|--------|-------------------|------------------|--------------|-----------------|
| **卸载语义** | Fiber 六态状态机；disposer 逆序回收并递归卸载子插件 | `context.subscriptions` 统一 dispose；多数扩展需 reload window | `onunload()` 手动清理；无自动级联 | bundle 停止自动注销服务/监听器；classloader 泄漏是老问题 | 无运行时卸载概念 |
| **依赖管理** | 声明式+响应式：缺依赖则 PENDING；provider 变化自动级联 | `extensionDependencies` 弱语义 + `activationEvents` 懒激活 | `manifest.json` 声明但无自动解析 | Import-Package 静态解析 + DS 动态 bind/unbind | 无，插件互不感知 |
| **可逆性保证** | 一等设计目标：revertible effect + 元定理 | 机制具备但场景弱："有撤销机制、无热卸载场景" | 手动，无框架保证 | 部分：注册项自动回收，代码/资源需手写 | 零 |
| **核心可否替换** | **无特权核心**：连 agent loop 都是插件 | 不可：编辑器内核离开插件仍可用 | 不可：核心编辑功能不可替换 | 不可：system bundle 不可卸载 | 不可：Compiler 核心不可替换 |
| **通信机制** | 类型化事件 + 5 种分发模式（emit/waterfall/parallel/serial/bail） | Contribution Points + Commands + Events API | `app.workspace` 等全局对象直接访问 | Extension Points + Extensions | Tap/Hook 链 |
| **热重载** | 事务性 HMR，不进入半重载状态 | 需 reload window | 需手动 disable/enable | 支持 bundle 热更新但有限 | 构建期概念，非运行时 |
| **沙箱隔离** | 无（进程内，信任代码） | Extension Host 进程隔离 | 无沙箱（直接访问 app 对象） | ClassLoader 隔离 | N/A |
| **语言** | TypeScript | TypeScript/JavaScript | TypeScript/JavaScript | Java | JavaScript |

### 4.2 历史谱系

"注册即可撤销"不是 Cordis 的发明，有四条独立血统：

1. **OSGi**（2000s）：bundle 停止时框架自动注销其注册的全部服务与监听器
2. **VS Code Disposable**：扩展把注册产物 push 进 `context.subscriptions`
3. **React useEffect**：effect 返回清理函数
4. **Effect-TS Scope**：scope 关闭时 finalizer 逆序执行

> Cordis 论文自述：动态组合的"形式化基础尚不完善"（formal foundations remain underdeveloped），把自身创新定位在**形式化而非机制发明**。

### 4.3 对笔记应用最值得借鉴的思想

| 来源 | 可借鉴思想 | 笔记应用适用性 |
|------|-----------|---------------|
| **Cordis** | 可逆副作用 + 依赖驱动生命周期 | ⭐⭐⭐⭐⭐ 插件卸载时自动清理注册内容 |
| **Cordis** | waterfall 事件（中间件链） | ⭐⭐⭐⭐⭐ 笔记保存/渲染/导出管线拦截 |
| **VS Code** | Contribution Points 声明式扩展 | ⭐⭐⭐⭐ 菜单、视图、命令的声明式注册 |
| **VS Code** | Extension Host 进程隔离 | ⭐⭐⭐⭐ 第三方插件不拖垮主进程 |
| **Obsidian** | 渐进式 API 暴露 | ⭐⭐⭐⭐ 基础功能先开放，高级能力需申请权限 |
| **OSGi** | 服务注册自动回收 | ⭐⭐⭐ 服务停止时自动注销 |
| **Webpack** | Tapable 钩子系统 | ⭐⭐⭐ 编译/构建管线的 hook 设计 |

> **来源**：[mazhen.tech 深度解析 §9.1.1](https://mazhen.tech/p/deepseek-harness-深度解析一切皆插件的-agent-运行时/)；[CSDN 插件架构对比](https://m.blog.csdn.net/lza_csdn2019/article/details/157772606)

---

## 5. 非 Node 环境移植方案

### 5.1 现状

- Cordis 核心是**纯 TypeScript**，运行在 Node.js 上
- 论文讨论了语言无关的实现可能性（Rust traits、过程宏、动态库、WebAssembly），但**无官方 Rust 实现**
- DeepSeek Harness 有 Python SDK 和 C-based Landlock launcher，但 Cordis 核心不变

### 5.2 最小可复用的设计模式（非代码移植）

对于 Rust 核心 / 移动端的笔记应用，应移植的是**设计模式**而非代码：

#### 模式 1：Context 树 + Proxy 服务解析

```rust
// Rust 概念示意
trait Service: Send + Sync + 'static {}

struct Context {
    parent: Option<Arc<Context>>,
    services: RwLock<HashMap<String, Arc<dyn Service>>>,
    fibers: RwLock<Vec<Arc<Fiber>>>,
}

impl Context {
    /// 类似 Proxy 拦截：先查自身，再沿父链查找
    fn get_service<T: Service>(&self, name: &str) -> Option<Arc<T>> {
        if let Some(svc) = self.services.read().get(name) {
            return svc.clone().downcast().ok();
        }
        self.parent.as_ref()?.get_service(name)
    }
    
    /// 创建子上下文（隔离作用域）
    fn scope(&self) -> Context {
        Context {
            parent: Some(Arc::new(self.clone())),
            services: RwLock::new(HashMap::new()),
            fibers: RwLock::new(Vec::new()),
        }
    }
}
```

**移植要点**：
- 用 trait object（`dyn Service`）代替 TypeScript 的 Proxy 动态属性
- 用 `Arc<Context>` 实现层级引用
- 服务解析：自身 → 父链 → 返回 None

#### 模式 2：依赖注入 + 响应式生命周期

```rust
// Rust 概念示意
#[async_trait]
trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn inject(&self) -> &[&str]; // 声明依赖的服务名
    async fn apply(&self, ctx: &Context) -> Result<Box<dyn Disposer>>;
}

trait Disposer: Send + Sync {
    fn dispose(&self) -> Pin<Box<dyn Future<Output = ()> + '_>>;
}

// Fiber 状态机
enum FiberState {
    Pending,   // 等待依赖
    Loading,   // apply() 执行中
    Active,    // 正常运行
    Failed,    // 异常
    Unloading, // 清理中
    Disposed,  // 已卸载
}
```

**移植要点**：
- `inject` 作为 trait 方法返回依赖列表
- 依赖就绪 → 调度 `apply()`；依赖消失 → 触发 `dispose()`
- 用 Rust 的 `Drop` trait 作为兜底，但主要靠显式 disposer

#### 模式 3：可逆副作用（Effect System）

```rust
// Rust 概念示意
struct EffectStack {
    disposers: Vec<Box<dyn Disposer>>,
}

impl EffectStack {
    fn register<D: Disposer + 'static>(&mut self, disposer: D) {
        self.disposers.push(Box::new(disposer));
    }
    
    /// LIFO 逆序执行所有 disposer
    async fn recover(&mut self) {
        while let Some(disposer) = self.disposers.pop() {
            disposer.dispose().await;
        }
    }
}
```

**移植要点**：
- 与 Rust RAII/Drop 互补：Drop 管词法作用域，EffectStack 管动态组件生命周期
- 所有经 Context 的注册都进 EffectStack
- 卸载 = `pop()` + `dispose()` 直到空

#### 模式 4：类型化事件 + 多分发模式

```rust
// Rust 概念示意
enum DispatchMode {
    Emit,      // 广播，不等结果
    Parallel,  // 并发 await
    Serial,    // 顺序 await，首个非 None 胜出
    Bail,      // serial 的同步版
    Waterfall, // 中间件链
}

struct EventBus {
    hooks: HashMap<String, Vec<Hook>>,
}

impl EventBus {
    /// Waterfall 示意
    async fn waterfall<T, F>(&self, name: &str, mut value: T, default: F) -> T
    where
        F: Fn() -> Pin<Box<dyn Future<Output = T>>>,
    {
        let hooks = self.hooks.get(name).cloned().unwrap_or_default();
        let mut remaining = hooks.into_iter().peekable();
        
        // 构建链式 next
        // 每个 hook 接收 (value, next) 并可修改 value 或短路
        // ...
        todo!()
    }
}
```

**移植要点**：
- Rust 的泛型 + trait 可提供比 TS 更强的类型安全
- Waterfall 可用闭包链或 `Box<dyn Fn>` 实现
- 考虑用 `tokio::spawn` 实现 parallel 模式

#### 模式 5：插件清单 + 声明式组合

```toml
# plugin-manifest.toml（概念示意）
[plugin]
name = "markdown-renderer"
version = "1.0.0"
entry = "libmarkdown_renderer.so"  # 或 .wasm / .dylib

[dependencies]
editor-core = ">=1.0"
storage = ">=1.0"

[contributes]
commands = ["render-markdown", "export-pdf"]
events = ["editor/before-save", "editor/after-load"]
```

### 5.3 移动端特殊考量

| 考量 | 方案 |
|------|------|
| **内存限制** | 插件用 WASM 沙箱，限制内存上限 |
| **无热重载需求** | 移动端可简化：去掉 HMR，只保留 install/uninstall |
| **安全沙箱** | WASM 天然隔离，或用 iOS/Android 的进程隔离 |
| **插件分发** | 应用内市场 + 签名校验，替代 npm |
| **性能** | 核心服务用原生代码，插件通过 FFI/WASM 桥接 |

### 5.4 最小移植优先级

| 优先级 | 模式 | 理由 |
|--------|------|------|
| **P0** | Effect/Disposer 栈 | 没有它，插件卸载必然泄漏 |
| **P0** | Context 层级 + 服务解析 | 解耦的基础 |
| **P1** | Fiber 状态机 | 管理插件生命周期 |
| **P1** | inject 依赖声明 | 自动编排启动顺序 |
| **P2** | 类型化事件（emit + waterfall） | 插件间通信 |
| **P3** | HMR | 桌面端可选，移动端可省 |
| **P3** | 声明式 Loader/配置协调 | 初期可手动管理 |

> **来源**：[redreamality.com §13-14](https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/)；[系统级 AGI Agent Harness 语言选型方案](https://m.toutiao.com/article/7677639721505260086/)

---

## 6. 最新版本 API 概览

### 6.1 包结构（v4.0.0-rc.8）

```
packages/
├── core/              # 核心运行时
│   ├── context.ts     # Context 类（Proxy 代理 + extend/isolate/intercept）
│   ├── registry.ts    # RegistryService（ctx.plugin / ctx.inject）、Plugin 类型
│   ├── fiber.ts       # Fiber 类（插件生命周期状态机 + effect 系统）★ 最核心
│   ├── events.ts      # EventsService（事件总线、5 种分发模式）、内置事件表
│   ├── service.ts     # Service 抽象基类（服务注册、可调用服务、配置合并）
│   ├── reflect.ts     # ReflectService（ctx.get/set/provide/mixin，服务解析）
│   ├── logger.ts      # 结构化日志门面（Exporter 可插拔）
│   ├── utils.ts       # 基础设施（DisposableList、traceable proxy、composeError）
│   └── index.ts       # 统一出口
├── loader/            # 插件加载器
│   └── config/
│       └── isolate.ts # 服务隔离机制
├── include/           # 配置文件加载
├── hmr/               # 热模块替换
├── timer/             # 定时器服务
├── logger-console/    # 控制台日志
└── group/             # 分组包
```

### 6.2 核心 API 速查

#### Context API

```typescript
// 创建根上下文
const ctx = new Context()

// 挂载插件
const fiber = ctx.plugin(MyPlugin, config?)
await fiber // 等待插件进入 ACTIVE

// 注册副作用
ctx.effect(() => {
  // setup
  return () => { /* cleanup */ }
})

// 事件
ctx.on('event-name', handler)
ctx.once('event-name', handler)
ctx.emit('event-name', ...args)
await ctx.parallel('event-name', ...args)
await ctx.serial('event-name', ...args)
ctx.bail('event-name', ...args)
await ctx.waterfall('event-name', ...args, defaultNext)

// 服务
ctx.get('serviceName')       // 读取（无则 undefined）
ctx.provide('name', value)   // 注册服务
ctx.mixin('service', ['method1', 'method2']) // 将服务方法混入 ctx
```

#### Plugin 规范

```typescript
// 函数插件
export const name = 'my-plugin'
export const inject = ['tools', 'database']

export function apply(ctx: Context, config: Config) {
  // 注册能力
  ctx.on('some-event', handler)
  ctx.effect(() => { /* ... */ return () => { /* cleanup */ } })
  ctx.plugin(SubPlugin) // 挂载子插件（随父卸载）
}

// 带 Schema 校验
export const Config = Schema.object({
  apiKey: Schema.string().required(),
  timeout: Schema.number().default(5000),
})
```

#### Service 基类

```typescript
export class MyService extends Service {
  static inject = ['database'] // 依赖
  
  constructor(ctx: Context) {
    super(ctx, 'myService') // 注册为 ctx.myService
  }
  
  // 服务方法
  async query(id: string) {
    return this.database.get(id) // 通过 inject 访问依赖
  }
}
```

#### Fiber API

```typescript
// 获取 fiber
const fiber = ctx.plugin(MyPlugin)

// 等待就绪
await fiber

// 手动卸载
await fiber.dispose()

// 状态查询
fiber.state // FiberState enum
fiber.ctx   // 子 Context
```

### 6.3 依赖关系图

```
context ──→ registry / events / reflect / logger / fiber
registry ──→ fiber
fiber    ──→ context / reflect
reflect  ──→ context / fiber / utils
```

> **来源**：[Cordis 框架超深度源码分析](https://m.blog.csdn.net/zhonglinzhang/article/details/164012094)；[GitHub cordiverse/cordis](https://github.com/cordiverse/cordis)

---

## 7. 对笔记软件的借鉴清单

### 7.1 架构层面

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 1 | **一切皆插件** | 编辑器核心、渲染引擎、同步模块、导出器、搜索索引都作为插件挂载，而非硬编码进核心。这样可以在不修改核心的情况下替换 Markdown 渲染器（如从 marked 换到 markdown-it）。 |
| 2 | **Context 树 + 作用域隔离** | 为每个笔记窗口/标签页创建子 Context，继承全局服务（存储、同步），但可以有独立的编辑器状态、插件实例。实现"每个窗口可以有不同插件配置"。 |
| 3 | **无特权核心** | 定义清晰的 Service 接口（如 `EditorEngine`、`StorageBackend`、`SyncProvider`），核心只消费接口不消费实现。这样可以从本地 SQLite 切换到 CRDT 云同步而不改业务代码。 |
| 4 | **能力接缝三层模式** | 对每个核心能力（如文件存储）定义：①接口（`StorageService`）②提供方（`LocalStorageProvider`、`CloudStorageProvider`）③消费方（编辑器、导出器）。替换存储后端不影响消费方。 |

### 7.2 插件生命周期

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 5 | **Fiber 状态机** | 插件有明确的 PENDING→ACTIVE→DISPOSED 生命周期。UI 可以根据插件状态显示"加载中"/"已就绪"/"加载失败"。失败的插件不影响其他插件运行。 |
| 6 | **可逆副作用（Effect/Disposer）** | 插件注册的命令、菜单项、快捷键、侧边栏视图、编辑器扩展全部进入 disposer 栈。卸载插件时自动移除所有注册内容，无需插件作者手写清理代码。 |
| 7 | **inject 依赖声明** | 插件声明 `inject: ['editor', 'storage']`，框架等待依赖就绪才激活。"PDF 导出"插件依赖"编辑器"服务，编辑器未初始化时它自动等待，无需手动编排启动顺序。 |
| 8 | **响应式依赖（Provider 替换）** | 用户从"本地存储"切换到"云同步"时，所有依赖 storage 的插件自动卸载→重新激活到新 provider 上。无需重启应用。 |

### 7.3 事件与通信

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 9 | **类型化事件** | 定义 `note/before-save`、`note/after-save`、`editor/before-render` 等事件，通过声明合并提供类型安全。拼错事件名编译期报错。 |
| 10 | **waterfall 中间件链** | 笔记保存管线：拼写检查插件→格式规范化插件→自动标签插件→最终保存。每个插件可以修改内容或拦截保存（如"内容未变更则跳过"）。 |
| 11 | **bail 短路决策** | 权限检查：`ctx.bail('note/can-delete', noteId)` → 第一个返回 `false` 的插件即阻止删除（如"笔记已锁定"/"无权限"）。 |
| 12 | **parallel 扇出** | 笔记保存后并行通知：同步插件开始上传、搜索索引插件更新索引、分析插件更新字数统计。 |

### 7.4 插件开发体验

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 13 | **三种插件形态** | 简单功能用函数插件（一个 `apply` 函数）；需要配置用对象插件；需要暴露服务给其他插件用 Service 子类。降低入门门槛。 |
| 14 | **声明式清单** | `plugin-manifest.json` 声明名称、版本、依赖、贡献点（命令、菜单、视图）。应用根据清单自动处理依赖顺序和冲突检测。 |
| 15 | **配置 Schema 校验** | 插件定义配置的 JSON Schema，框架自动校验用户输入。配置错误时给出明确报错而非运行时崩溃。 |
| 16 | **HMR（桌面端）** | 开发模式下，插件代码保存后自动重载，不需要重启整个笔记应用。大幅提升插件开发效率。 |

### 7.5 安全与隔离

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 17 | **进程隔离（参考 VS Code）** | 第三方插件运行在独立 Worker/进程，崩溃不影响主应用。通过消息传递通信，而非直接访问内存对象。 |
| 18 | **渐进式权限（参考 Obsidian）** | 基础 API（读写笔记、注册命令）默认开放；高级能力（网络请求、文件系统访问、剪贴板）需显式申请权限并在清单中声明。 |
| 19 | **Context 介导的副作用边界** | 明确划界：经框架 API 的操作可追踪/可撤销；直接访问 OS 资源（文件、网络）的副作用不在框架管理范围内，需插件自行用 `ctx.effect()` 登记。 |

### 7.6 跨平台移植

| # | 借鉴点 | 如何应用到笔记软件 |
|---|--------|-------------------|
| 20 | **Rust 核心 + WASM 插件** | 核心运行时用 Rust 实现（Context 树、Fiber 状态机、Effect 栈、事件总线），插件编译为 WASM 模块，天然沙箱隔离。 |
| 21 | **最小移植集** | P0 先实现：Effect/Disposer 栈 + Context 层级 + 服务解析。P1 再加：Fiber 状态机 + inject。P2 加事件系统。避免一次性全量移植。 |
| 22 | **移动端简化** | 移动端去掉 HMR（不需要开发时热重载），简化 Loader（应用内安装而非 npm），用 WASM 沙箱替代进程隔离。 |

---

## 8. 设计模式总结

### 8.1 Cordis 的核心设计模式

```
┌─────────────────────────────────────────────────────────────┐
│                    Cordis 设计模式金字塔                      │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Layer 4: 声明式组合（Configuration as Composition） │   │
│  │  cordis.yml / manifest.json 描述期望的插件树          │   │
│  └─────────────────────────────────────────────────────┘   │
│                          ▲                                   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Layer 3: 可逆副作用（Revertible Effects）            │   │
│  │  ctx.effect() → disposer; LIFO 回收                  │   │
│  └─────────────────────────────────────────────────────┘   │
│                          ▲                                   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Layer 2: 响应式共效（Reactive Coeffects）            │   │
│  │  inject 声明 → 依赖就绪自动激活 → provider 变化自动重载 │   │
│  └─────────────────────────────────────────────────────┘   │
│                          ▲                                   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Layer 1: Context 中介（Context-Mediated Access）     │   │
│  │  Proxy 拦截 → 服务解析 → 层级继承                      │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 8.2 关键不变量

1. **一切经 ctx 的操作都是 effect**：提供服务、挂载插件、注册监听器，内部都走 `ctx.effect()`
2. **注册即记账，卸载即回滚**：没有组件享有豁免权
3. **身份 = 不变量集合**：替换一个组件不改变系统身份，只要不变量集合保持
4. **acquisition 可逆，emission 需补偿**：框架内注册可自动撤销；已发出的网络请求/文件写入不可回滚

### 8.3 反模式警示

| 反模式 | 问题 | 正确做法 |
|--------|------|----------|
| 直接 `import` 具体实现 | 强耦合，无法替换 | 通过 `ctx.<key>` 查找服务 |
| 在 `apply()` 中做重活 | 阻塞加载序列 | `apply()` 只注册，重活交给 effect/事件 |
| 绕过 ctx 直接操作全局状态 | 无法追踪/撤销 | 所有副作用经 `ctx.effect()` 登记 |
| 忘记 waterfall 中调用 `next()` | 静默吞掉下游 | 只有有意短路时才不调用 |
| 每个简单功能都拆成 Definition+Provider+Consumer | 过度工程 | 只在能力确实需要独立演化时才拆分 |

---

## 9. 参考来源

### 官方资源

| 来源 | URL |
|------|-----|
| GitHub 仓库 | https://github.com/cordiverse/cordis |
| 论文 (arXiv) | https://arxiv.org/abs/2608.25512 |
| 论文仓库 | https://github.com/cordiverse/paper |
| 官方文档 (cordis-primer) | https://deepseek-harness.github.io/deepseek-harness/reference/cordis-primer |
| DeepSeek Harness 架构文档 | https://github.com/deepseek-ai/deepseek-harness/blob/main/docs/architecture.md |

### 深度分析文章

| 来源 | URL |
|------|-----|
| Cordis Explained (Remy Blog, 英文, 最全面) | https://redreamality.com/blog/cordis-spatiotemporal-composability-deepseek-harness/ |
| DeepSeek Harness 深度解析 (mazhen.tech) | https://mazhen.tech/p/deepseek-harness-深度解析一切皆插件的-agent-运行时/ |
| Cordis 框架超深度源码分析 (CSDN) | https://m.blog.csdn.net/zhonglinzhang/article/details/164012094 |
| 万物皆插件：DeepSeek Harness 底层揭秘 (掘金) | https://juejin.cn/post/7693225151681396746 |
| Cordis 核心设计深度解读 (CSDN) | https://m.blog.csdn.net/chendongqi2007/article/details/164881421 |
| Cordis 到底是什么 (腾讯新闻) | https://new.qq.com/rain/a/20260904A0BKC100 |
| Cordis 工作原理解析 (百度) | https://baijiahao.baidu.com/s?id=1875460779452889187 |
| 从源码理解 Cordis (掘金) | https://www.juejin.cn/post/7675174962013257791 |
| Cordis 插件引擎拆解 (xujun.org) | https://www.xujun.org/note-2098246928097841154.html |

### API 与教程

| 来源 | URL |
|------|-----|
| Cordis 入门 01 (CSDN) | https://m.blog.csdn.net/qq_41914181/article/details/164507546 |
| 事件系统 emit/bail/serial/waterfall (菜鸟教程) | https://www.runoob.com/deepseek-harness/deepseek-harness-events.html |
| DSH 文档合辑：Cordis 教程与 API (CSDN) | https://m.blog.csdn.net/zhonglinzhang/article/details/163952639 |
| dsh-plugin-dev-skill (Skills Directory) | https://www.skillsdirectory.com/skills/green-dalii-dsh-plugin-dev-skill |

### 对比与生态

| 来源 | URL |
|------|-----|
| 插件架构谱系对照 (mazhen.tech §9.1) | https://mazhen.tech/p/deepseek-harness-深度解析一切皆插件的-agent-运行时/ |
| Cordis 从开源到 DeepSeek (腾讯云) | https://cloud.tencent.com/developer/article/2728030 |
| 解构 Cordis (CSDN) | https://m.blog.csdn.net/gs80140/article/details/163752955 |
| AGI Agent Harness 语言选型 | https://m.toutiao.com/article/7677639721505260086/ |

---

*本报告基于 2026-10-12 的公开信息编写。Cordis v4 仍处于 RC 阶段，API 可能变化。*
