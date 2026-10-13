# 04 - 笔记软件文件管理系统设计与同步方案调研

> 调研日期：2026-10-12
> 目标：为跨平台笔记软件的文件管理系统与同步方案提供技术决策依据

---

## 目录

1. [信息架构：GoodNotes 6 & Notability 文件管理模型](#1-信息架构goodnotes-6--notability-文件管理模型)
2. [本地存储引擎选型](#2-本地存储引擎选型)
3. [缩略图/预览生成与缓存策略](#3-缩略图预览生成与缓存策略)
4. [WebDAV 客户端实现与同步方案](#4-webdav-客户端实现与同步方案)
5. [移动端 IO 性能与平台差异](#5-移动端-io-性能与平台差异)
6. [综合建议与数据模型设计](#6-综合建议与数据模型设计)

---

## 1. 信息架构：GoodNotes 6 & Notability 文件管理模型

### 1.1 GoodNotes 6 文件管理模型

GoodNotes 6 采用**多层嵌套文件夹**结构，信息层级为：

```
文库 (Library)
├── 文稿 (Documents)        ← 主标签页
│   ├── 文件夹 A
│   │   ├── 子文件夹 A-1
│   │   │   ├── 笔记本 A-1-a
│   │   │   └── 笔记本 A-1-b
│   │   └── 笔记本 A-2
│   └── 笔记本 B（顶层）
├── 收藏夹 (Favorites)       ← 虚拟集合，非物理位置
├── 搜索 (Search)            ← 全局搜索（含手写识别 OCR）
├── 共享 (Shared)            ← 协作/分享文档
└── 市集 (Marketplace)       ← 模板/素材商店
```

**核心特性**：
- **文件夹嵌套**：支持 10+ 层深度嵌套（实测至少 10 层无问题）
- **QuickNote**：快速笔记入口，无需指定文件夹
- **颜色标签**：文件夹/笔记本可设置颜色标签，子项继承父级色系
- **批量操作**：多选模式支持批量重命名、移动、导出、复制、删除
- **全文搜索**：支持手写识别搜索，可搜索所有笔记本内容
- **默认排序**：按日期、名称排序
- **默认导入位置**：可设置默认导入路径，自动归类新内容
- **Flashcards（学习卡片）**：GoodNotes 6 新增功能

> 来源：[GoodNotes 6 全面使用教程 - CSDN](https://m.blog.csdn.net/m0_73640344/article/details/147260315)；[Goodnotes vs Notability - Paperlike](https://paperlike.com/blogs/paperlikers-insights/app-review-goodnotes-vs-notability)；[GoodNotes 怎么快速整理笔记本 - php.cn](https://m.php.cn/faq/2529116.html)

### 1.2 Notability 的 Subject/Divider 模型

Notability 采用**分割条 (Divider) → 主题 (Subject) → 笔记 (Note)** 的两级分类结构：

```
侧边栏
├── 分割条 A (Divider)       ← 最大号"文件夹"
│   ├── 主题 1 (Subject)     ← 次级"文件夹"
│   │   ├── 笔记 a
│   │   └── 笔记 b
│   └── 主题 2
│       └── 笔记 c
├── 分割条 B
│   └── ...
└── 未归档 (Unfiled)         ← 临时存放区
```

**核心特性**：
- **分割条 (Divider)**：最大号分类容器，可嵌套最多 5 层
- **主题 (Subject)**：次级分类，按科目/项目/类别组织
- **未归档 (Unfiled)**：未分类笔记的临时存放区，避免遗漏
- **侧边栏导航**：一次性查看所有 Divider/Subject/Note
- **拖拽操作**：支持在 Subject 间拖拽移动笔记
- **颜色分类**：每个 Subject 可设置不同颜色
- **录音同步**：Notability 独有功能，录音与笔记内容同步回放

> 来源：[iPad 上使用 Notability 的专业指南 - CSDN](https://blog.csdn.net/m0_73640344/article/details/147260337)；[notability 整理归档 - CSDN](https://blog.csdn.net/weixin_39678304/article/details/111728468)；[Goodnotes 6 vs Notability 2026 - affine.pro](https://affine.pro/blog/goodnotes-vs-notability-tips)

### 1.3 信息架构共性归纳

| 维度 | GoodNotes 6 | Notability | 共性 |
|------|-------------|------------|------|
| 主结构 | 多层嵌套文件夹 | Divider → Subject 两级 | **树形文件夹** |
| 快速访问 | 收藏夹 + QuickNote | 收藏夹 + 未归档 | **收藏/快捷入口** |
| 搜索 | 全局搜索（含手写 OCR） | 全局搜索 | **全局搜索** |
| 最近 | 按日期排序 | 按日期排序 | **最近使用** |
| 共享 | 共享标签页 | 链接分享 | **协作/分享** |
| 标签 | 颜色标签 | 颜色 + 自定义封面 | **视觉标签系统** |
| 未分类 | 无（强制归类） | 未归档区 | **缓冲/暂存** |

**共性总结**：**树形文件夹 + 标签/收藏 + 全局搜索** 是笔记软件信息架构的三大支柱。

### 1.4 GoodNotes 文档格式

`.goodnotes` 文件本质上是一个 **ZIP 压缩包**，内含：
- **PDF 数据**：每页笔记渲染为 PDF 页面
- **元数据**：笔迹、手写数据、页面布局等
- **录音文件**：如有录音，以 `.m4a` 格式存储
- **图片/附件**：嵌入的图片和其他资源

`.goodnotes.zip` 是完整资料库备份格式，包含文件夹结构。

> 来源：[GoodNotes 导入文件支持 - support.goodnotes.com](https://support.goodnotes.com/hc/zh-cn/articles/7353717816463)；[导出 GoodNotes 录音 - CSDN](https://devpress.csdn.net/v1/article/detail/129461612)；[GoodNotes 导出文件 - support.goodnotes.com](https://support.goodnotes.com/hc/zh-cn/articles/7353742824975)

---

## 2. 本地存储引擎选型

### 2.1 技术选型对比表

| 特性 | SQLite + FTS5 | Isar | ObjectBox | 纯文件系统 + JSON |
|------|--------------|------|-----------|-----------------|
| **类型** | 关系型嵌入式 DB | NoSQL 嵌入式 DB | 面向对象嵌入式 DB | 文件级存储 |
| **查询能力** | SQL + FTS5 全文检索 | 类型安全 QueryBuilder | QueryBuilder（编译时检查） | 需自行实现 |
| **全文搜索** | ✅ FTS5 原生支持 | ⚠️ 需自行集成 | ❌ 无内置 | ❌ 需自行实现 |
| **ACID 事务** | ✅ 完整支持 | ✅ 支持 | ✅ 支持 | ❌ 需自行保证 |
| **跨平台一致性** | ✅ 全平台一致（C 库） | ⚠️ Flutter 为主 | ⚠️ 主要 Android/JVM | ✅ 全平台 |
| **性能（写入）** | 中等 | 快（10 万条 ~3.6s） | 极快（10 万条 ~3.3s） | 取决于实现 |
| **性能（读取）** | 中等 | 极快（10 万条 <500ms） | 快（10 万条 ~4.7s） | 取决于实现 |
| **包体积增量** | ~50KB（Room 封装） | 中等 | 1-1.5MB | 0 |
| **开源** | ✅ 公共领域 | ✅ Apache 2.0 | ❌ 闭源 | ✅ |
| **Web 支持** | ✅ wa-sqlite (WASM) | ⚠️ 有限 | ❌ 无 Web 支持 | ✅ |
| **CRDT 集成** | ✅ 可通过 sqlite-crdt | ⚠️ 需自行实现 | ⚠️ 需自行实现 | ⚠️ 需自行实现 |
| **适用场景** | 需要标准 SQL、FTS、跨平台 | Flutter 项目、高性能 NoSQL | Android 高性能场景 | 极简场景 |

### 2.2 推荐方案

**主选：SQLite + FTS5**

理由：
1. **跨平台一致性最佳**：SQLite 是唯一在 Android/iOS/Desktop/Web 全平台都有成熟支持且行为一致的嵌入式数据库
2. **FTS5 全文搜索**：原生支持全文索引，50 万条记录 MATCH 查询仅需 ~0.03s
3. **CRDT 生态**：`sqlite-crdt` (CRDT-sqlite) 项目已实现 SQLite + CRDT 集成
4. **标准 SQL**：查询灵活，调试方便，工具生态丰富
5. **包体积极小**：Room 封装仅 ~50KB

> 来源：[SQLite vs ObjectBox vs Isar - Ente Photos](https://ente.io/blog/tech/sqlite-objectbox-isar/)；[iOS/Android SQLite 全文检索 FTS - Baidu Wenku](https://wenku.baidu.com/view/58aa2a63e618964bcf84b9d528ea81c758f52e27)；[端侧 RAG 实战指南 - 掘金](https://juejin.cn/post/7613971395928408091)

**Flutter 项目备选：Isar**
- 如果项目使用 Flutter，Isar 是优秀的 NoSQL 替代方案
- 读取性能极佳，API 现代
- 但 Web 支持有限，且 FTS 需额外集成

### 2.3 CRDT 用于协作同步的价值

| 特性 | Yjs | Automerge | Loro |
|------|-----|-----------|------|
| **语言** | JavaScript/TypeScript | JavaScript/Rust | Rust |
| **设计** | 状态基础型 CRDT | 操作基础型 CRDT | CRDT |
| **富文本** | ✅ Yjs + TipTap/ProseMirror 成熟 | ✅ Peritext 算法（3.x） | ⚠️ loro-prosemirror |
| **版本历史** | ⚠️ 需自行实现 | ✅ 原生支持（hash + diff + cherry-pick） | ⚠️ 早期 |
| **离线支持** | ✅ 原生 | ✅ 原生 | ✅ 原生 |
| **内存效率** | 较优 | 较高（存储元数据多） | 较优 |
| **生态成熟度** | ⭐⭐⭐⭐⭐ 最成熟 | ⭐⭐⭐⭐ 成熟 | ⭐⭐ 早期 |
| **WebSocket 集成** | y-websocket / Hocuspocus / Liveblocks | automerge-repo | 手动集成 |
| **SQLite 集成** | ⚠️ 需桥接 | ✅ Automerge 3.x columnar storage | ✅ SQLite 原生 |

**建议**：
- **近期**：使用 SQLite + 操作日志表实现本地同步，暂不引入 CRDT
- **中期（协作功能）**：引入 **Yjs** 作为协作引擎，与 TipTap/ProseMirror 集成最成熟
- **远期（版本历史 + 分支合并）**：考虑 **Automerge 3.x**，其文档模型原生支持 fork/merge/diff/cherry-pick

> 来源：[Yjs vs Automerge vs Loro 2026 - PkgPulse](https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026)；[CRDT 数据结构与算法 - tool.lu](https://tool.lu/fr_FR/topic/iZ/CRDT/)；[History of CRDTs 2026 - Taskade](https://www.taskade.com/blog/crdt-history)；[基于 CRDT 的协同编辑实践 - CSDN](https://blog.csdn.net/Zaninho14/article/details/163864670)

---

## 3. 缩略图/预览生成与缓存策略

### 3.1 缩略图生成策略

```
┌─────────────────────────────────────────────────────┐
│                  缩略图生成流水线                       │
├─────────────────────────────────────────────────────┤
│                                                     │
│  笔记保存/修改 ──→ 异步生成缩略图（后台线程）             │
│       │                                             │
│       ├──→ 生成低分辨率预览（~200px 宽，WebP 格式）     │
│       ├──→ 存储至缓存目录（按 noteId_pageIndex 命名）   │
│       └──→ 更新 SQLite 元数据表（thumbnail_path）      │
│                                                     │
│  列表加载时：                                         │
│       │                                             │
│       ├──→ 缓存命中 → 直接显示                        │
│       ├──→ 缓存未命中 → 显示占位符 → 异步生成/下载      │
│       └──→ 渐进式加载：低分辨率 → 高清（交叉淡入）       │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### 3.2 缓存层级

| 层级 | 存储位置 | 生命周期 | 用途 |
|------|---------|---------|------|
| L1 - 内存缓存 | RAM | 应用运行期 | 当前可见 + 预加载项 |
| L2 - 磁盘缓存 | App Sandbox/Caches | 系统自动清理 | 缩略图文件 |
| L3 - 持久存储 | App Sandbox/Documents | 永久 | 原始笔记数据 |

### 3.3 虚拟列表实现

**核心原则**：只渲染可视区域 + 少量预加载区域的列表项

```
┌──────────────────────────────┐
│     预加载缓冲区（上方）        │  ← 提前渲染，滚动时无延迟
├──────────────────────────────┤
│                              │
│     可视区域（Viewport）       │  ← 实际渲染的列表项
│                              │
├──────────────────────────────┤
│     预加载缓冲区（下方）        │  ← 提前渲染，滚动时无延迟
└──────────────────────────────┘
│     未渲染区域（回收/延迟）      │  ← 组件回收，数据解绑
```

**各平台实现**：
- **Flutter**：`ListView.builder` + `CachedNetworkImage` + `VisibilityDetector`
- **iOS (SwiftUI)**：`LazyVStack` / `LazyVGrid`
- **Android (Compose)**：`LazyColumn` / `LazyVerticalGrid`
- **Web (React)**：`react-window` / `react-virtualized` + `IntersectionObserver`
- **鸿蒙**：`LazyForEach` + `List` 组件

**加载体验优化组合**：
1. 骨架屏占位（shimmer 效果）
2. 低分辨率预览（先加载小图）
3. 渐进式高清加载（交叉淡入效果）
4. 内存压力时释放离屏资源

> 来源：[鸿蒙 App 列表滚动卡顿优化 - 华为云](https://bbs.huaweicloud.com/blogs/472403)；[Flutter 图片内存优化指南 - CSDN](https://blog.csdn.net/2501_94417424/article/details/155946530)；[实操图片流页面体验优化 - 腾讯云](https://cloud.tencent.com/developer/article/2446865)

---

## 4. WebDAV 客户端实现与同步方案

### 4.1 WebDAV 协议要点

#### 核心 HTTP 方法

| 方法 | 用途 | 关键 Header |
|------|------|------------|
| `PROPFIND` | 获取目录列表/文件属性 | `Depth: 0/1/infinity`，请求 XML body |
| `GET` | 下载文件内容 | `Range`（断点续传） |
| `PUT` | 上传/覆盖文件 | `If-Match: <etag>`（条件写入） |
| `DELETE` | 删除文件/目录 | 要求无锁 |
| `MKCOL` | 创建目录 | - |
| `COPY` | 复制文件 | `Destination`, `Overwrite` |
| `MOVE` | 移动/重命名 | `Destination`, `Overwrite` |
| `LOCK` | 加锁（独占/共享） | XML body（lockinfo） |
| `UNLOCK` | 解锁 | `Lock-Token` |
| `PROPPATCH` | 修改文件属性 | XML body |

#### ETag 与冲突检测

- **ETag**：服务器为每个资源生成的唯一标识符，内容变化时 ETag 变化
- **条件写入**：`PUT` 时附带 `If-Match: "<etag>"` 头，若服务端 ETag 不匹配则返回 `412 Precondition Failed`
- **Last-Modified**：资源最后修改时间，作为 ETag 的补充校验手段

> 来源：[WebDAV 概述与原理 - 51CTO](https://blog.51cto.com/u_15851118/14692182)；[WebDAV 基础知识 - CSDN](https://blog.csdn.net/zlllc/article/details/136102593)；[WebDAV 与协作 - CSDN](https://m.blog.csdn.net/gogzf/article/details/78434561)；[冲突检测 - Microsoft](https://msdn.microsoft.com/ZH-CN/library/cc264293)

### 4.2 可用 WebDAV 库

#### Rust 生态

| 库名 | 特点 | 链接 |
|------|------|------|
| **reqwest_dav** | 基于 tokio + reqwest 的异步客户端，支持 Basic/Digest 认证，GET/PUT/MV/CP/DELETE/MKCOL/LIST | [github.com/niuhuan/reqwest_dav](https://github.com/niuhuan/reqwest_dav) |
| **rustydav** (async) | 基于 reqwest，支持 GET/PUT/DELETE/MKCOL/MV/LIST/UNZIP | [github.com/dbian/rustydav_async](https://github.com/dbian/rustydav_async) |
| **hyperdav** | 基于 hyper 的 WebDAV 客户端 | crates.io |
| **dav-server-rs** | Rust WebDAV **服务端**库（messense） | [github.com/messense/dav-server-rs](https://www.github-zh.com/projects/443671503-dav-server-rs) |

#### JavaScript/TypeScript 生态

| 库名 | 特点 | 链接 |
|------|------|------|
| **webdav** (perry-mitchell) | 最成熟的 JS WebDAV 客户端，TypeScript 编写，支持 Node.js + 浏览器，多种认证方式 | [npm: webdav](https://www.npmjs.com/package/webdav)；[GitHub](https://github.com/perry-mitchell/webdav-client) |
| **webdav-client-uniapp** | webdav 的 uni-app 适配 fork | [github.com/kytrun/webdav-client-uniapp](https://github.com/kytrun/webdav-client-uniapp) |
| **webdavjs** | 轻量级 JS WebDAV 实现，支持同步/异步模式 | [gitee.com/mirrors/webdavjs](https://gitee.com/mirrors/webdavjs) |

> 来源：[reqwest_dav - GitHub](https://github.com/niuhuan/reqwest_dav)；[webdav NPM](https://www.npmjs.com/package/webdav)；[awesome-webdav - GitHub](https://github.com/fstanis/awesome-webdav)

### 4.3 增量同步算法设计

#### 双向同步状态机

```
                    ┌──────────┐
                    │   IDLE   │ ← 初始/同步完成状态
                    └────┬─────┘
                         │ 触发同步
                         ▼
                  ┌──────────────┐
                  │  CHECK_LOCAL │ ← 扫描本地变更（对比本地 etag/mtime）
                  └──────┬───────┘
                         │
                         ▼
                  ┌──────────────┐
                  │ CHECK_REMOTE │ ← PROPFIND 获取远端属性（etag/mtime）
                  └──────┬───────┘
                         │
                         ▼
                  ┌──────────────┐
                  │  DIFF_CALC   │ ← 计算差异集合
                  └──────┬───────┘
                         │
            ┌────────────┼────────────┐
            ▼            ▼            ▼
     ┌───────────┐ ┌──────────┐ ┌──────────┐
     │ UPLOAD    │ │ DOWNLOAD │ │ CONFLICT │
     │ (本地新)  │ │ (远端新) │ │ (双方改) │
     └─────┬─────┘ └────┬─────┘ └────┬─────┘
           │            │            │
           └────────────┼────────────┘
                        ▼
                 ┌──────────────┐
                 │   APPLY      │ ← 应用变更到本地 DB + 文件系统
                 └──────┬───────┘
                        │
                        ▼
                 ┌──────────────┐
                 │   VERIFY     │ ← 校验同步结果（etag 一致性）
                 └──────┬───────┘
                        │
                        ▼
                 ┌──────────────┐
                 │   CLEANUP    │ ← 处理墓碑删除、清理临时文件
                 └──────┬───────┘
                        │
                        ▼
                    ┌──────────┐
                    │   IDLE   │
                    └──────────┘
```

#### 同步算法伪代码

```python
# ===== 数据结构 =====
class SyncRecord:
    id: str              # 笔记/文件夹唯一 ID
    local_etag: str      # 本地已知的 etag
    remote_etag: str     # 远端最新 etag
    local_mtime: int     # 本地修改时间
    remote_mtime: int    # 远端修改时间
    is_deleted: bool     # 墓碑标记
    deleted_at: int      # 删除时间戳
    sync_status: enum    # SYNCED / DIRTY / CONFLICT / PENDING

# ===== 核心同步算法 =====
async def sync(remote_client: WebDAVClient, local_db: SQLiteDatabase):
    
    # Phase 1: 收集本地变更
    local_changes = local_db.query(
        "SELECT * FROM sync_records WHERE sync_status IN ('DIRTY', 'PENDING')"
    )
    
    # Phase 2: 获取远端状态
    remote_listing = await remote_client.propfind(
        path="/notes/",
        depth="infinity",
        props=["getetag", "getlastmodified", "getcontentlength"]
    )
    remote_map = {item.href: item for item in remote_listing}
    
    # Phase 3: 差异计算
    to_upload = []     # 本地新增/修改 → 上传
    to_download = []   # 远端新增/修改 → 下载
    to_delete_local = []   # 远端删除 → 本地删除
    to_delete_remote = []  # 本地删除 → 远端删除
    conflicts = []     # 双方都修改 → 冲突
    
    for record in local_changes:
        remote = remote_map.get(record.remote_path)
        
        if remote is None:
            if record.is_deleted:
                pass  # 双方都已删除
            else:
                to_upload.append(record)  # 远端不存在，上传
                
        elif record.is_deleted:
            if record.deleted_at > remote.mtime:
                to_delete_remote.append(record)  # 本地删除时间更晚
            else:
                conflicts.append(record)  # 远端有更新但本地已删除
                
        elif record.local_etag != remote.etag:
            # 远端也有变化 → 冲突
            conflicts.append((record, remote))
        else:
            to_upload.append(record)  # 远端未变，上传本地
    
    for remote_item in remote_listing:
        local = local_db.get_by_path(remote_item.href)
        if local is None:
            to_download.append(remote_item)  # 本地不存在，下载
        elif local.remote_etag != remote_item.etag and local not in local_changes:
            to_download.append(remote_item)  # 远端变化且本地未修改
    
    # Phase 4: 执行同步
    
    # 4a: 上传（带条件写入防冲突）
    for record in to_upload:
        data = local_db.get_content(record.id)
        try:
            await remote_client.put(
                path=record.remote_path,
                data=data,
                headers={"If-Match": f'"{record.local_etag}"'}  # 条件写入
            )
            new_etag = get_response_etag()
            local_db.update_sync_status(record.id, "SYNCED", new_etag)
        except HTTP_412:
            conflicts.append(record)  # 条件写入失败 → 冲突
    
    # 4b: 下载
    for item in to_download:
        data = await remote_client.get(item.href)
        local_db.save_content(item.href, data, item.etag)
    
    # 4c: 远端删除
    for record in to_delete_remote:
        await remote_client.delete(record.remote_path)
        local_db.mark_deleted(record.id)
    
    # 4d: 本地删除
    for record in to_delete_local:
        local_db.delete_content(record.id)
    
    # Phase 5: 冲突解决
    for conflict in conflicts:
        strategy = resolve_conflict(conflict)  # LWW / 三路合并 / 用户选择
        apply_resolution(strategy)

# ===== 冲突解决策略 =====
def resolve_conflict(conflict):
    """
    策略优先级：
    1. LWW (Last Write Wins) - 以最后修改时间为准
    2. 三路合并 - 如果有 CRDT/操作日志
    3. 保留两版本 - 生成冲突副本供用户选择
    """
    local, remote = conflict
    
    # LWW: 比较修改时间
    if local.local_mtime > remote.mtime:
        return "UPLOAD_LOCAL"
    elif remote.mtime > local.local_mtime:
        return "DOWNLOAD_REMOTE"
    else:
        # 时间戳相同但内容不同 → 保留两个版本
        return "KEEP_BOTH"
```

#### 墓碑删除机制

```
删除操作流程：
1. 本地标记 is_deleted = true, deleted_at = now()
2. 同步时：
   - 若远端 etag 未变 → DELETE 远端文件 → 本地物理删除
   - 若远端 etag 已变 → 冲突（远端有更新但本地已删除）
3. 墓碑保留期：7-30 天（可配置），过期后物理清除
4.  tombstone 表结构：
   ┌────────┬──────────┬────────────┬──────────────┐
   │   id   │ path     │ deleted_at │ synced       │
   ├────────┼──────────┼────────────┼──────────────┤
   │ uuid-1 │ /a/b.md  │ 1697100000 │ false        │
   │ uuid-2 │ /c/d.md  │ 1697000000 │ true         │
   └────────┴──────────┴────────────┴──────────────┘
```

### 4.4 常见服务端兼容性问题

| 服务端 | 已知问题 | 应对策略 |
|--------|---------|---------|
| **坚果云** | ① 并发请求限制（4-8 线程以上出现 429）；② 无增量同步协议，只能对比 mtime+size；③ WebDAV 目录与客户端同步目录是两套逻辑 | 控制并发线程数 ≤4；使用 mtime+size 双重校验；明确告知用户目录映射关系 |
| **Nextcloud** | ① 大文件 PUT 可能失败（PHP 配置限制）；② 升级后 WebDAV 接口偶发 broken 告警；③ 资源占用高（System Load 飙升）；④ 日志信息有限 | 调整 PHP `upload_max_filesize`/`post_max_size`；检查 nginx 配置 `client_max_body_size`；分块上传 |
| **群晖 (Synology)** | ① WebDAV 套件偶发需修复；② 旧版内核兼容性问题；③ Cloud Sync 的 WebDAV 模式偶发写入大小为 0 | 保持 DSM 和 WebDAV 套件更新；使用 WebDAV 最新版套件；验证写入完整性 |

> 来源：[坚果云 WebDAV 适配指南 - CSDN](https://m.blog.csdn.net/weixin_28700411/article/details/166109837)；[Nextcloud WebDAV broken Issue #39549](https://github.com/nextcloud/server/issues/39549)；[群晖挂载 WebDAV 解决方法](https://wp.gxnas.com/10383.html)；[吐槽 Nextcloud 体验](https://www.codewoody.com/posts/22580/)

### 4.5 增量同步与版本历史设计

```
版本历史存储模型：

notes 表:
┌──────┬────────┬───────────┬────────────┬──────────────┐
│  id  │ title  │ content   │ version    │ parent_id    │
├──────┼────────┼───────────┼────────────┼──────────────┤
│ n-1  │ 笔记A  │ <blob>    │ 5          │ null         │
└──────┴────────┴───────────┴────────────┴──────────────┘

note_versions 表:
┌──────┬──────────┬───────────┬────────────┬──────────────┬──────────┐
│  id  │ note_id  │ version   │ content    │ created_at   │ etag     │
├──────┼──────────┼───────────┼────────────┼──────────────┼──────────┤
│ v-1  │ n-1      │ 1         │ <blob>     │ 1697000000   │ "abc1"   │
│ v-2  │ n-1      │ 2         │ <blob>     │ 1697100000   │ "abc2"   │
│ v-3  │ n-1      │ 3         │ <blob>     │ 1697200000   │ "abc3"   │
│ v-4  │ n-1      │ 4         │ <blob>     │ 1697300000   │ "abc4"   │
└──────┴──────────┴───────────┴────────────┴──────────────┴──────────┘

sync_log 表（操作日志，用于增量同步和审计）:
┌──────┬──────────┬──────────┬───────────┬────────────┬──────────┐
│  id  │ note_id  │ action   │ device_id │ timestamp  │ checksum │
├──────┼──────────┼──────────┼───────────┼────────────┼──────────┤
│ l-1  │ n-1      │ CREATE   │ dev-A     │ 1697000000 │ sha256.. │
│ l-2  │ n-1      │ UPDATE   │ dev-A     │ 1697100000 │ sha256.. │
│ l-3  │ n-1      │ UPDATE   │ dev-B     │ 1697200000 │ sha256.. │
│ l-4  │ n-1      │ DELETE   │ dev-A     │ 1697300000 │ null     │
└──────┴──────────┴──────────┴───────────┴────────────┴──────────┘
```

**版本保留策略**：
- 最近 10 个版本：永久保留
- 1 天内的版本：每小时保留 1 个
- 7 天内的版本：每天保留 1 个
- 超过 7 天：仅保留最早和最新版本

---

## 5. 移动端 IO 性能与平台差异

### 5.1 iOS 文件权限模型

**App Sandbox 架构**：
```
应用沙盒目录结构：
├── AppName.app/          ← 应用包（只读，签名验证）
├── Documents/            ← 用户数据（iCloud 备份 ✅）
├── Library/
│   ├── Preferences/      ← NSUserDefaults（iCloud 备份 ✅）
│   └── Caches/           ← 缓存（系统可自动清理 ❌）
├── Application Support/  ← 应用支持文件
└── tmp/                  ← 临时文件（重启可能清除 ❌）
```

**关键限制**：
- 严格的沙盒隔离，应用间不能直接访问文件
- 跨应用共享需通过 **App Groups**（需 Apple Developer 后台配置）
- 后台执行受限：不能随意在后台运行同步任务，只能通过 **BGTaskScheduler** 请求系统调度
- 后台同步需声明 `background-fetch` 或 `background-processing` 模式
- 系统决定何时执行后台任务，开发者无法保证时效性

### 5.2 Android 文件权限模型

**Scoped Storage（Android 11+）**：
```
存储访问模型：
├── 应用私有目录
│   └── /Android/data/<package_name>/   ← 完全隔离
├── 共享媒体
│   └── 通过 MediaStore API 访问（需权限）
├── 其他应用文件
│   └── 通过 Storage Access Framework (SAF) 访问
└── 下载目录
    └── 通过系统文件选择器
```

**关键特性**：
- Android 11+ 引入 Scoped Storage，应用无法直接访问任意文件路径
- **SAF (Storage Access Framework)**：通过系统文件选择器获取用户授权的文件访问权限
- 后台执行相对自由：可使用 **WorkManager** 调度后台同步任务
- 支持 **Foreground Service** 进行长时间后台同步（需显示通知）
- 权限模型从安装时授权 → 运行时动态授权（Android 6+）

### 5.3 iOS vs Android 对比

| 维度 | iOS | Android |
|------|-----|---------|
| **沙盒** | 严格沙盒，App Sandbox | Linux UID 隔离 + Scoped Storage |
| **文件访问** | 仅 Documents/Library/tmp | 私有目录 + SAF + MediaStore |
| **后台同步** | BGTaskScheduler（系统调度，不可控） | WorkManager / Foreground Service（更灵活） |
| **跨应用共享** | App Groups（需配置） | Content Provider / Intent / SAF |
| **断点续传** | NSURLSession 支持 | OkHttp / HttpURLConnection 支持 |
| **权限请求** | 运行时即时请求，不可检测是否被拒 | 运行时请求，可检测授权状态 |

### 5.4 后台同步策略

```
┌─────────────────────────────────────────────────┐
│               后台同步策略矩阵                     │
├─────────────────────────────────────────────────┤
│                                                 │
│  触发条件：                                       │
│  ├── 应用前台恢复 → 立即同步                       │
│  ├── 网络状态变化（WiFi 连接）→ 触发同步            │
│  ├── 定时触发（iOS: BGTask / Android: WorkMgr）   │
│  └── 用户手动触发 → 立即同步                       │
│                                                 │
│  约束条件：                                       │
│  ├── 电量：低电量模式下降级/暂停同步                │
│  ├── 流量：仅 WiFi 下执行大文件同步                 │
│  ├── 计费网络：仅同步元数据，不下载内容             │
│  └── 后台时限：iOS ~30s / Android Foreground 无限 │
│                                                 │
│  断点续传：                                       │
│  ├── 大文件分块上传（4MB/block）                   │
│  ├── 记录已上传块索引 → 恢复时跳过已完成块          │
│  └── 使用 HTTP Range 头 / 自定义分块协议           │
│                                                 │
└─────────────────────────────────────────────────┘
```

### 5.5 电量/流量策略

| 策略 | 实现方式 |
|------|---------|
| **WiFi 限定** | 检测网络类型，蜂窝网络下仅同步元数据（标题/标签/目录结构） |
| **低电量降级** | 监听 `BatteryManager`（Android）/ `ProcessInfo.batteryLevel`（iOS），低电量时暂停非关键同步 |
| **后台流量预算** | iOS: 系统分配后台执行时间预算；Android: WorkManager 的 `Constraints` 设置网络类型 |
| **分块传输** | 大文件切分为 4MB 块，逐块上传/下载，支持断点续传 |
| **差量同步** | 仅传输变更的块（rsync 算法），而非整个文件 |

> 来源：[iOS vs Android Security 2026 - appsecsanta](https://appsecsanta.com/mobile-security-tools/ios-vs-android-security)；[Android/iOS 应用沙箱对比 - CSDN](https://blog.csdn.net/o5p6q/article/details/155118174)；[移动端同步失败原因及解决方案 - CSDN](https://wenku.csdn.net/column/44teqdk7r6)；[iOS 运行与 Android 沙盒对比 - CSDN](https://m.blog.csdn.net/weixin_29001419/article/details/117541461)

---

## 6. 综合建议与数据模型设计

### 6.1 推荐信息架构（ER 简图）

```
┌─────────────────────────────────────────────────────────────────┐
│                        数据模型 ER 图                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────┐       ┌──────────────┐       ┌──────────────┐    │
│  │  Folder   │       │   Notebook   │       │    Page      │    │
│  ├──────────┤       ├──────────────┤       ├──────────────┤    │
│  │ id (PK)  │◄──┐   │ id (PK)      │◄──┐   │ id (PK)      │    │
│  │ name     │   │   │ folder_id(FK)│   │   │ notebook_id  │    │
│  │ parent_id│───┘   │ title        │   │   │ page_index   │    │
│  │ color    │       │ cover_path   │   │   │ content_blob │    │
│  │ icon     │       │ template_id  │   │   │ thumbnail    │    │
│  │ position │       │ created_at   │   │   │ checksum     │    │
│  │ is_star  │       │ updated_at   │   │   │ created_at   │    │
│  │ sort_order│      │ is_starred   │   │   └──────────────┘    │
│  └──────────┘       │ is_deleted   │   │                       │
│       │             │ sync_status  │   │   ┌──────────────┐    │
│       │             │ local_etag   │   │   │   Tag        │    │
│       │             │ remote_etag  │   │   ├──────────────┤    │
│       │             │ version      │   │   │ id (PK)      │    │
│       │             └──────────────┘   │   │ name         │    │
│       │                    │           │   │ color        │    │
│       │                    │           │   └──────┬───────┘    │
│       │                    ▼           │          │            │
│       │             ┌──────────────┐   │          │            │
│       │             │  QuickNote   │   │   ┌──────▼───────┐    │
│       │             ├──────────────┤   │   │  NoteTag     │    │
│       │             │ id (PK)      │   │   ├──────────────┤    │
│       │             │ content      │   │   │ note_id (FK) │    │
│       │             │ created_at   │   │   │ tag_id  (FK) │    │
│       │             │ updated_at   │   │   └──────────────┘    │
│       │             │ is_deleted   │   │                       │
│       │             └──────────────┘   │   ┌──────────────┐    │
│       │                                │   │  SyncRecord  │    │
│       │                                │   ├──────────────┤    │
│       └────────────────────────────────┘   │ entity_id    │    │
│                                            │ entity_type  │    │
│                                            │ local_etag   │    │
│                                            │ remote_etag  │    │
│                                            │ sync_status  │    │
│                                            │ last_synced  │    │
│                                            │ is_deleted   │    │
│                                            │ deleted_at   │    │
│                                            └──────────────┘    │
│                                                                 │
│  ┌──────────────┐       ┌──────────────┐                       │
│  │  SyncLog     │       │  NoteVersion │                       │
│  ├──────────────┤       ├──────────────┤                       │
│  │ id (PK)      │       │ id (PK)      │                       │
│  │ entity_id    │       │ note_id (FK) │                       │
│  │ action       │       │ version      │                       │
│  │ device_id    │       │ content_blob │                       │
│  │ timestamp    │       │ created_at   │                       │
│  │ checksum     │       │ etag         │                       │
│  └──────────────┘       │ device_id    │                       │
│                         └──────────────┘                       │
└─────────────────────────────────────────────────────────────────┘

关系说明：
  Folder.parent_id → Folder.id         (自引用，树形嵌套)
  Notebook.folder_id → Folder.id       (笔记本归属文件夹)
  Page.notebook_id → Notebook.id       (页面归属笔记本)
  NoteTag.note_id → Notebook.id        (多对多关联)
  NoteTag.tag_id → Tag.id
  SyncRecord: 多态关联，entity_type 区分 Folder/Notebook/Page
  NoteVersion.notebook_id → Notebook.id (版本历史)
```

### 6.2 技术选型总结

| 层面 | 推荐方案 | 备选方案 |
|------|---------|---------|
| **本地数据库** | SQLite + FTS5 | Isar（Flutter 项目） |
| **全文搜索** | SQLite FTS5 + 中文分词（jieba/SCWS） | 自建倒排索引 |
| **协作同步（远期）** | Yjs（近期） / Automerge 3.x（版本历史） | - |
| **Web 端存储** | wa-sqlite (WASM) + OPFS | IndexedDB |
| **WebDAV 客户端（Rust）** | reqwest_dav | rustydav |
| **WebDAV 客户端（JS/TS）** | webdav (perry-mitchell) | webdav-client-uniapp |
| **缩略图缓存** | 三级缓存（内存 → 磁盘 → 持久存储） | - |
| **虚拟列表** | 平台原生 Lazy 组件 | react-window (Web) |
| **同步策略** | 增量同步 + etag 条件写入 + LWW 冲突解决 | CRDT 三路合并（远期） |
| **后台同步** | iOS: BGTaskScheduler / Android: WorkManager | - |
| **断点续传** | 分块传输（4MB/block）+ 块索引记录 | - |

### 6.3 关键设计决策

1. **信息架构**：采用树形文件夹 + 收藏 + 标签 + 全局搜索的四维组织方式，参考 GoodNotes 的多层嵌套 + Notability 的未归档缓冲
2. **文档格式**：采用类 `.goodnotes` 的 ZIP 包格式（PDF + 元数据 + 资源），便于导出/导入兼容
3. **存储引擎**：SQLite 作为核心存储，FTS5 提供全文搜索，操作日志表支持增量同步
4. **同步方案**：WebDAV 作为主要同步通道，etag + mtime 双重校验，LWW 冲突解决，墓碑删除机制
5. **性能优化**：三级缩略图缓存 + 虚拟列表 + 分块传输 + 差量同步
6. **平台适配**：统一 SQLite 数据层，平台特定层处理文件权限/后台任务/网络约束

---

## 参考来源汇总

### 信息架构
- [GoodNotes 6 全面使用教程 - CSDN](https://m.blog.csdn.net/m0_73640344/article/details/147260315)
- [Goodnotes vs Notability - Paperlike](https://paperlike.com/blogs/paperlikers-insights/app-review-goodnotes-vs-notability)
- [GoodNotes 整理笔记本技巧 - php.cn](https://m.php.cn/faq/2529116.html)
- [iPad 使用 Notability 专业指南 - CSDN](https://blog.csdn.net/m0_73640344/article/details/147260337)
- [Notability 整理归档 - CSDN](https://blog.csdn.net/weixin_39678304/article/details/111728468)
- [GoodNotes 导入文件 - support.goodnotes.com](https://support.goodnotes.com/hc/zh-cn/articles/7353717816463)
- [GoodNotes 导出文件 - support.goodnotes.com](https://support.goodnotes.com/hc/zh-cn/articles/7353742824975)
- [导出 GoodNotes 录音 - CSDN](https://devpress.csdn.net/v1/article/detail/129461612)

### 存储引擎
- [SQLite vs ObjectBox vs Isar - Ente Photos](https://ente.io/blog/tech/sqlite-objectbox-isar/)
- [iOS/Android SQLite FTS 全文检索 - Baidu Wenku](https://wenku.baidu.com/view/58aa2a63e618964bcf84b9d528ea81c758f52e27)
- [端侧 RAG 实战指南 - 掘金](https://juejin.cn/post/7613971395928408091)
- [ObjectBox vs SQLite - CSDN](https://m.blog.csdn.net/gitblog_00126/article/details/155119775)

### CRDT
- [Yjs vs Automerge vs Loro 2026 - PkgPulse](https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026)
- [CRDT 数据结构与算法 - tool.lu](https://tool.lu/fr_FR/topic/iZ/CRDT/)
- [History of CRDTs 2026 - Taskade](https://www.taskade.com/blog/crdt-history)
- [基于 CRDT 的协同编辑 - CSDN](https://blog.csdn.net/Zaninho14/article/details/163864670)
- [Yjs 终极指南 - CSDN](https://blog.csdn.net/gitblog_00846/article/details/153153160)

### WebDAV
- [WebDAV 概述与原理 - 51CTO](https://blog.51cto.com/u_15851118/14692182)
- [WebDAV 基础知识 - CSDN](https://blog.csdn.net/zlllc/article/details/136102593)
- [WebDAV 与协作 - CSDN](https://m.blog.csdn.net/gogzf/article/details/78434561)
- [reqwest_dav - GitHub](https://github.com/niuhuan/reqwest_dav)
- [webdav NPM - perry-mitchell](https://www.npmjs.com/package/webdav)
- [awesome-webdav - GitHub](https://github.com/fstanis/awesome-webdav)
- [冲突检测 - Microsoft](https://msdn.microsoft.com/ZH-CN/library/cc264293)
- [企业云盘同步机制深度解析 - 掘金](https://juejin.cn/post/7647389207657431103)
- [坚果云 WebDAV 适配指南 - CSDN](https://m.blog.csdn.net/weixin_28700411/article/details/166109837)
- [Nextcloud WebDAV Issue #39549](https://github.com/nextcloud/server/issues/39549)
- [群晖 WebDAV 解决方法](https://wp.gxnas.com/10383.html)
- [note-gen WebDAV 同步测评 - CSDN](https://m.blog.csdn.net/gitblog_00347/article/details/151276683)
- [davynci WebDAV 同步工具 - CSDN](https://wenku.csdn.net/doc/3cqh3h2pza)

### 移动端 IO
- [iOS vs Android Security 2026 - appsecsanta](https://appsecsanta.com/mobile-security-tools/ios-vs-android-security)
- [Android/iOS 应用沙箱对比 - CSDN](https://blog.csdn.net/o5p6q/article/details/155118174)
- [iOS 运行与 Android 沙盒对比 - CSDN](https://m.blog.csdn.net/weixin_29001419/article/details/117541461)
- [移动端同步失败原因 - CSDN](https://wenku.csdn.net/column/44teqdk7r6)

### 缩略图/虚拟列表
- [鸿蒙 App 列表滚动优化 - 华为云](https://bbs.huaweicloud.com/blogs/472403)
- [Flutter 图片内存优化指南 - CSDN](https://blog.csdn.net/2501_94417424/article/details/155946530)
- [实操图片流页面体验优化 - 腾讯云](https://cloud.tencent.com/developer/article/2446865)
