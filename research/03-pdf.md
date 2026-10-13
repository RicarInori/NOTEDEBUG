# 笔记应用内置 PDF 编辑器技术方案调研

> 调研日期：2026-10-12
> 调研目标：为跨平台笔记软件（Android/iOS/桌面/WASM）选型 PDF 引擎，覆盖查看、批注、手写叠加、页面管理、导出扁平化 PDF 全链路。

---

## 目录

1. [PDF 引擎选型对比](#1-pdf-引擎选型对比)
2. [PDFium 深度能力](#2-pdfium-深度能力)
3. [手写批注与 PDF 结合方案](#3-手写批注与-pdf-结合方案)
4. [大 PDF 渲染性能](#4-大-pdf-渲染性能)
5. [参考实现的文件格式](#5-参考实现的文件格式)
6. [推荐结论](#6-推荐结论)
7. [PDF 编辑 API 映射表（功能 → PDFium API）](#7-pdf-编辑-api-映射表)
8. [存储格式建议](#8-存储格式建议)

---

## 1. PDF 引擎选型对比

### 1.1 引擎总览对比表

| 维度 | **PDFium** | **MuPDF** | **Apache PDFBox** | **pdf-lib** | **pdf.js** | **PSPDFKit (Nutrient)** | **Apryse (PDFTron)** |
|------|-----------|-----------|-------------------|-------------|------------|------------------------|---------------------|
| **开发语言** | C++ | C | Java | TypeScript (纯 JS) | JavaScript | 多平台 SDK (原生+JS) | 多平台 SDK (原生+JS) |
| **许可证** | Apache-2.0 / BSD | AGPL / 商业双许可 | Apache-2.0 | MIT | Apache-2.0 | 商业闭源 | 商业闭源 |
| **渲染性能** | ★★★★★ (Chrome 内核) | ★★★★★ (极轻量) | ★★★ (中等) | N/A (不渲染) | ★★★★ (Web 标准) | ★★★★★ | ★★★★★ |
| **编辑能力** | ★★★★ (注解/页面/字体/内容流) | ★★★★ (完整编辑+转换) | ★★★★ (Java 全功能) | ★★★ (注解/页面操作) | ★★ (查看为主) | ★★★★★ | ★★★★★ |
| **Android** | ✅ (系统 PdfRenderer 基于此) | ✅ (JNI) | ✅ (PdfBox-Android) | ✅ (WASM) | ✅ (WASM) | ✅ 原生 SDK | ✅ 原生 SDK |
| **iOS** | ✅ (编译集成) | ✅ (Obj-C/Swift 绑定) | ❌ | ✅ (WASM) | ✅ (WASM) | ✅ 原生 SDK | ✅ 原生 SDK |
| **桌面 (Win/Mac/Linux)** | ✅ | ✅ | ✅ | ✅ (Node.js) | ✅ (Electron/Web) | ✅ | ✅ |
| **WASM** | ✅ (实验性但可用) | ✅ (mupdf.js 官方) | ❌ | ✅ (原生) | ✅ (原生) | ✅ | ✅ |
| **包体积** | ~3-8 MB (动态库) | ~2-4 MB (核心极轻量) | ~15 MB (含依赖) | ~1 MB (纯 JS) | ~2 MB (WASM) | ~30+ MB | ~30+ MB |
| **价格** | 免费 | AGPL 免费 / 商业联系 Artifex | 免费 | 免费 | 免费 | 商业定制（高） | $1,500+/年起 |

### 1.2 各引擎详细分析

#### PDFium（推荐首选）

- **背景**：原 Foxit 开发，2014 年捐赠给 Chromium 项目，Google 维护。Android 系统 `PdfRenderer` 即基于 PDFium。
- **许可**：Apache-2.0 或 BSD，对商业闭源完全友好。
- **优势**：
  - Chrome/Chromium 大规模验证，渲染兼容性极佳
  - 429+ 个 `FPDF_*` C API 函数，覆盖渲染/注解/页面操作/字体/表单/签名
  - 预编译二进制分发成熟（bblanchon/pdfium-binaries 每周自动构建）
  - 社区封装丰富：Flutter (pdfium_dart/pdfium_flutter)、Rust (pdfium-render)、Python (pypdfium2)、Go (go-pdfium)、.NET
  - WASM 支持（实验性但 @embedpdf/pdfium 已可用）
- **劣势**：
  - 编译配置复杂（需 GN + Ninja，依赖 Chromium 构建系统）
  - 不直接支持 PDF 内容流的自由编辑（如修改已有文本）
  - 无内置 OCR
- **来源**：https://github.com/chromium/pdfium | https://github.com/bblanchon/pdfium-binaries

#### MuPDF

- **背景**：Artifex Software 开发维护，C 语言实现，核心仅 ~2MB。
- **许可**：AGPL v3 / 商业双许可。**AGPL 对商业闭源项目有传染性**——整个应用需开源，除非购买商业许可。
- **优势**：
  - 极致轻量，启动速度极快（Zathura 冷启动 < 0.3s）
  - 内存占用低（同场景约为 Poppler 的 1/3）
  - 支持 PDF/XPS/EPUB/CBZ 多格式
  - 完整的注解、表单、数字签名、搜索、涂黑功能
  - mupdf.js 官方 WASM 绑定可用
  - PyMuPDF (Python) 性能极强
- **劣势**：
  - AGPL 许可对商业项目不友好
  - 商业许可需联系 Artifex 定价（不透明）
  - CMYK 色彩空间仅做近似转换，印刷校色场景有偏差
- **来源**：https://mupdf.com | https://gitee.com/alayi/mupdf (LICENSE 文件)

#### Apache PDFBox

- **背景**：Apache 基金会项目，Java 生态最成熟的 PDF 库。
- **许可**：Apache-2.0，完全自由。
- **优势**：完整的 PDF 创建/修改/渲染/文本提取/表单处理
- **劣势**：
  - 仅 Java 生态，移动端需使用 PdfBox-Android 移植版
  - 渲染性能中等（Android 测试 ~450ms/页 vs PDFium ~220ms/页）
  - 无 WASM 支持
  - 包体积较大
- **来源**：https://github.com/apache/pdfbox

#### pdf-lib（JS 生态）

- **背景**：纯 TypeScript 实现，专注 PDF 创建和轻量修改。
- **许可**：MIT，完全自由。
- **优势**：
  - 纯 JS 无原生依赖，浏览器/Node.js 通用
  - 创建/修改注解、页面操作、嵌入字体、合并文档
  - 通过 PDF incremental update 机制追加注解
- **劣势**：
  - **无渲染能力**——不能将 PDF 页面渲染为图像
  - 不适合复杂 PDF 解析
  - 大文件内存管理需手动优化
- **来源**：https://github.com/Hopding/pdf-lib (7.1k stars)

#### pdf.js（JS 生态）

- **背景**：Mozilla 开发，Web 端 PDF 渲染事实标准。
- **许可**：Apache-2.0。
- **优势**：浏览器端渲染质量高，文本层抽取支持选择/搜索
- **劣势**：
  - 编辑能力极弱（查看为主）
  - 注解创建需配合其他库（如 pdfAnnotate）
  - 不能写入修改后的 PDF
- **来源**：https://github.com/nicefail/pdf.js

#### PSPDFKit (Nutrient) / Apryse (PDFTron) — 商用 SDK

| 维度 | PSPDFKit (Nutrient) | Apryse (PDFTron) |
|------|--------------------|--------------------|
| 定位 | 全功能 PDF SDK | 企业级文档 SDK |
| 平台 | iOS/Android/Web/桌面 | iOS/Android/Web/桌面/服务器 |
| 手写/Ink | ✅ 完整的 Ink Annotation 创建 | ✅ 完整 |
| 扁平化 | ✅ Flatten annotations | ✅ |
| 实时协作 | ✅ | ✅ |
| 价格 | 商业定制（高，联系销售） | $1,500+/年起（模块化） |
| 特色 | Magic Ink 手绘形状识别、Instant JSON | 200+ 格式转换、深度标注协作 |

- **评估**：功能最全面，但价格高昂（年费数千到数万美元），适合预算充足的企业级产品。对于初创/中小团队，建议先用开源方案验证产品，后期按需升级。
- **来源**：https://www.nutrient.io/sdk/vs/apryse | https://apryse.com/ | https://segmentfault.com/a/1190000047916465

### 1.3 Rust 生态 PDF 库

| 库 | 定位 | 许可 | Stars | 能力 |
|---|------|------|-------|------|
| **pdfium-render** | PDFium 的高级 Rust 封装 | Apache-2.0 | 713 | 渲染/文本提取/注解/页面操作/字体/表单/WASM，绑定 429 个 FPDF_* API |
| **lopdf** | 纯 Rust PDF 操作库 | MIT | 2.8k | 底层 PDF 对象操作、创建/修改、合并，无渲染能力 |
| **printpdf** | PDF 生成库 | MIT | 980 | 专注 PDF 生成：文本/图形/图层/字体嵌入/Unicode，支持 WASM |
| **pdf-writer** | 纯写入 PDF 库 | Apache-2.0 | 593 | 极低层级的 PDF 构建，Typst 项目使用 |
| **mupdf (Rust 绑定)** | MuPDF 的 Rust 绑定 | AGPL | — | 通过 FFI 调用 MuPDF，受 AGPL 约束 |

- **来源**：https://github.com/ajrcarey/pdfium-render | https://github.com/J-F-Liu/lopdf | https://github.com/fschutt/printpdf | https://github.com/typst/pdf-writer

---

## 2. PDFium 深度能力

### 2.1 头文件模块一览

PDFium 通过以下头文件组织 API（共 24 个头文件，429+ 函数）：

| 头文件 | 功能域 | 关键能力 |
|--------|--------|---------|
| `fpdfview.h` | 核心视图 | 初始化库、加载/关闭文档、渲染页面 |
| `fpdf_edit.h` | 编辑 | 创建/修改页面、页面内容对象（文本/路径/图像）、字体加载 |
| `fpdf_annot.h` | 注解 | 创建/修改/删除注解（含 Ink Annotation）、设置外观流 |
| `fpdf_text.h` | 文本 | 文本提取、字符级信息、文本搜索 |
| `fpdf_formfill.h` | 表单 | 表单字段填充、交互 |
| `fpdf_ppo.h` | 页面操作 | 页面复制、跨文档页面导入 |
| `fpdf_save.h` | 保存 | 文档保存（全量/增量） |
| `fpdf_flatten.h` | 扁平化 | 将注解+表单渲染为页面内容（`FPDFPage_Flatten`） |
| `fpdf_transformpage.h` | 页面变换 | MediaBox/CropBox 设置、页面旋转 |
| `fpdf_attachment.h` | 附件 | 文件附件的创建/提取 |
| `fpdf_signature.h` | 签名 | 数字签名验证 |
| `fpdf_catalog.h` | 目录 | PDF 标签/目录操作 |
| `fpdf_thumbnail.h` | 缩略图 | 页面缩略图 |
| `fpdf_structtree.h` | 结构树 | 标签 PDF 结构 |
| `fpdf_doc.h` | 文档 | 书签/链接 |
| `fpdf_javascript.h` | JS | PDF 内嵌 JavaScript |
| `fpdf_sysfontinfo.h` | 系统字体 | 系统字体信息、字体回退 |

- **来源**：https://blog.csdn.net/m0_53768564/article/details/128866008

### 2.2 核心编辑能力

#### 注解操作（fpdf_annot.h）

```
FPDFPage_CreateAnnot()           → 创建注解（指定子类型：Ink/Text/Highlight/...）
FPDFPage_GetAnnot()              → 获取已有注解
FPDFPage_GetAnnotCount()         → 获取注解数量
FPDFPage_RemoveAnnot()           → 删除注解
FPDFAnnot_SetRect()              → 设置注解边界框
FPDFAnnot_SetColor()             → 设置颜色
FPDFAnnot_SetFlags()             → 设置标志（Print/NoZoom/NoRotate）
FPDFAnnot_SetStringValue()       → 设置任意字典键值
FPDFAnnot_SetBorder()            → 设置边框
FPDFAnnot_SetAP()                → 设置外观流
FPDFAnnot_GetAP()                → 获取外观流
```

#### Ink Annotation 专用（手写笔迹）

```
FPDFAnnot_AddInkStroke()         → 向 InkList 添加一条笔画（点数组）
FPDFAnnot_RemoveInkList()        → 清除 InkList
```

> 参考 ISO 32000-1:2008 Section 12.5.6.13：Ink annotation 的 `InkList` 数组存储多组 `(x1, y1, x2, y2, ..., xn, yn)` 坐标点。

#### 页面操作

```
FPDFPage_New()                   → 创建新页面
FPDF_DeletePage()                → 删除页面
FPDFPage_GetRotation()           → 获取旋转角度
FPDFPage_SetRotation()           → 设置旋转角度 (0/90/180/270)
FPDF_ImportPages()               → 跨文档导入页面 (fpdf_ppo.h)
FPDF_MovePages()                 → 移动页面位置（新版 API）
FPDFPage_GetBoundary()           → 获取页面边界 (MediaBox/CropBox/...)
FPDFPage_SetMediaBox()           → 设置页面边界
```

#### 字体嵌入

```
FPDFText_LoadFont()              → 加载 TrueType/OpenType 字体并嵌入
FPDFText_LoadStandardFont()      → 加载标准 14 字体
FPDFText_LoadCidType2Font()      → 加载 CID 字体（CJK 支持）
FPDFFont_GetFontData()           → 获取嵌入字体数据
FPDFFont_GetFamilyName()         → 获取字体族名
FPDF_AddInstalledFont()          → 注册系统字体
```

#### 页面内容对象

```
FPDFPageObj_NewTextObj()         → 创建文本对象
FPDFPageObj_NewImageObj()        → 创建图像对象
FPDFPageObj_CreateNewPath()      → 创建路径对象
FPDFPageObj_CreateNewRect()      → 创建矩形对象
FPDFPage_InsertObject()          → 向页面插入对象
FPDFPage_RemoveObject()          → 从页面移除对象
FPDFPage_GenerateContent()       → 生成/重建内容流
FPDFPageObj_SetFillColor()       → 设置填充色
FPDFPageObj_SetStrokeColor()     → 设置描边色
FPDFPageObj_SetLineWidth()       → 设置线宽
FPDFPageObj_Transform()          → 应用变换矩阵
```

#### 保存

```
FPDF_SaveAsCopy()                → 全量保存为副本
FPDF_SaveWithVersion()           → 指定 PDF 版本保存
// 通过 FPDF_FILEWRITE 回调支持自定义输出（内存/文件/网络）
```

#### 扁平化（Flatten）

```
FPDFPage_Flatten()               → 将注解和表单字段渲染为页面内容（不可再编辑）
// fpdf_flatten.h — 导出 PDF 前调用，使所有注解成为永久性页面内容
```

### 2.3 社区封装生态

| 项目 | 语言/平台 | 说明 |
|------|----------|------|
| [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) | 预编译二进制 | 每周自动构建，支持 Win/Linux/Mac/Android/iOS/WASM，NuGet/Conda 分发 |
| [paulocoutinho/pdfium-lib](https://github.com/paulocoutinho/pdfium-lib) | 多平台编译脚本 | 为 iOS/Android/macOS/WASM 编译 PDFium，MIT 许可 |
| [pdfium-render](https://github.com/ajrcarey/pdfium-render) | Rust | 高级 Rust 封装，713 stars，支持 WASM，绑定全部 429 个 API |
| [pypdfium2](https://pypi.org/project/pypdfium2/) | Python | Python ctypes 绑定，自动下载 pdfium-binaries |
| pdfium_dart / pdfium_flutter | Dart/Flutter | Flutter FFI 绑定，支持 iOS/Android |
| [printing (pub.dev)](https://pub.dev/packages/printing) | Flutter | 使用 pdfium-binaries 实现 PDF 渲染和打印 |
| [@embedpdf/pdfium](https://www.npmjs.com/package/@embedpdf/pdfium) | JS/WASM | PDFium WebAssembly 封装，浏览器端渲染/编辑 |
| PdfiumViewer / Pdfium.Net | C#/.NET | .NET 平台 PDFium 封装 |

- **来源**：https://github.com/bblanchon/pdfium-binaries | https://github.com/paulocoutinho/pdfium-lib | https://pub.dev/packages/printing

---

## 3. 手写批注与 PDF 结合方案

### 3.1 两种主流方案对比

| 维度 | **方案 A：PDF Ink Annotation** | **方案 B：自定义图层 + 导出扁平化** |
|------|-------------------------------|-----------------------------------|
| **存储方式** | 笔迹直接存为 PDF `/Ink` 注解（ISO 32000 InkList） | 笔迹存为独立数据（JSON/二进制），PDF 作为背景引用 |
| **标准兼容** | ✅ 任何 PDF 阅读器可见笔迹 | ❌ 仅本应用可见，需导出才可见 |
| **编辑灵活性** | ⚠️ 受 PDF 注解模型限制（InkList 为点序列） | ✅ 完全自由（图层/混合模式/撤销栈） |
| **性能** | ⚠️ 大量笔画时 PDF 文件膨胀、解析慢 | ✅ 渲染和存储解耦，各自优化 |
| **跨应用互操作** | ✅ 标准 PDF annotation | ❌ 需要专有格式 |
| **扁平化导出** | 已有 AP 外观流，可直接 flatten | 导出时将笔迹绘制到 PDF 页面内容流 |
| **代表应用** | 标准 PDF 注解工具（Adobe Acrobat） | GoodNotes、Notability、Xournal++ |

### 3.2 推荐方案：混合架构（方案 C）

**核心思路**：编辑时使用自定义图层（高性能、灵活），存储时同时维护 PDF Ink Annotation（标准兼容），导出时扁平化。

```
┌─────────────────────────────────────────────┐
│                 编辑层 (Runtime)              │
│  ┌─────────┐  ┌──────────┐  ┌────────────┐  │
│  │ PDF 背景 │  │ 笔迹图层  │  │ 文本/形状  │  │
│  │ (只读)   │  │ (自定义)  │  │  图层      │  │
│  └─────────┘  └──────────┘  └────────────┘  │
├─────────────────────────────────────────────┤
│              存储层 (Persist)                 │
│  ┌──────────────────────────────────────┐    │
│  │ 原生格式 (.xxx)                       │    │
│  │  - PDF 文件引用/嵌入                   │    │
│  │  - 笔迹数据 (JSON/binary strokes)     │    │
│  │  - 页面元数据                          │    │
│  └──────────────────────────────────────┘    │
├─────────────────────────────────────────────┤
│              导出层 (Export)                  │
│  ┌──────────────────────────────────────┐    │
│  │ PDF 扁平化导出                         │    │
│  │  - 笔迹 → PDF Ink Annotation 或        │    │
│  │    笔迹 → PDF 内容流 (path object)      │    │
│  │  - FPDFPage_Flatten() 或手动写入        │    │
│  └──────────────────────────────────────┘    │
└─────────────────────────────────────────────┘
```

### 3.3 GoodNotes / Notability 做法推断

#### GoodNotes

- **自有格式 `.goodnotes`**：本质是 ZIP 压缩包
  - 内含 `attachments/` 文件夹，图片以 JPEG2000 (`.jp2`) 格式存储
  - 使用**先进的向量墨水引擎**（官方描述），笔迹为矢量存储
  - PDF 导入后作为页面背景，笔迹独立存储
  - 支持 OCR 搜索手写内容
  - 页面可自由排列/删除，说明页面数据是独立管理的
- **推断**：GoodNotes 采用方案 B——PDF 为背景图层，手写笔迹以专有矢量格式独立存储，导出 PDF 时将笔迹烘焙到页面中。
- **来源**：https://m.bilibili.com/opus/715403829276835897 | https://support.goodnotes.com/hc/zh-cn/articles/4599403401359

#### Notability

- **自有格式 `.note`**：本质是 ZIP 压缩包，内含：
  ```
  template/
  ├── Session.plist        ← Apple 二进制 plist，含 curvespoints（笔迹坐标数据）
  ├── metadata.plist       ← 元数据
  ├── Recordings/
  │   └── library.plist    ← 音频索引
  ├── Images/              ← 图片资源
  ├── thumbnail, thumb2x, thumb3x, thumb6x  ← 缩略图
  └── *.pdf                ← 导入的 PDF 文件
  ```
  - `Session.plist` 中的 `curvespoints` 字段存储 base64 编码的笔画坐标二进制数据
  - 音频录制与笔迹时间戳关联
- **推断**：Notability 同样采用方案 B——PDF 和笔迹分离存储，plist 中保存笔画数据，导出时合成。
- **来源**：https://m.freebuf.com/news/167120.html | https://wenku.csdn.net/doc/6792u7t75r

### 3.4 PDF Ink Annotation 写入示例（PDFium）

```c
// 创建 Ink Annotation
FPDF_PAGE page = FPDF_LoadPage(doc, page_index);
FPDF_ANNOTATION annot = FPDFPage_CreateAnnot(page, FPDF_ANNOT_INK);

// 设置边界框
FS_RECTF rect = {left, top, right, bottom};
FPDFAnnot_SetRect(annot, &rect);

// 设置颜色
FPDFAnnot_SetColor(annot, FPDFANNOT_COLORTYPE_STROKE, 255, 0, 0, 255);

// 添加笔画（一组点坐标）
FS_POINTF points[] = {{100, 200}, {110, 210}, {120, 205}, ...};
FPDFAnnot_AddInkStroke(annot, points, sizeof(points)/sizeof(FS_POINTF));

// 设置 Print 标志
FPDFAnnot_SetFlags(annot, FPDF_ANNOT_FLAG_PRINT);

FPDFPage_CloseAnnot(annot);
```

---

## 4. 大 PDF 渲染性能

### 4.1 分块渲染（Tile-Based Rendering）

**核心思想**：将页面分割为固定大小的 tile（如 256×256 或 512×512），仅渲染可见区域的 tile。

| 策略 | 说明 | 效果 |
|------|------|------|
| **tile 尺寸** | 256×256（CPU 开销高但内存低）/ 512×512（推荐平衡）/ 1024×1024（单次压力大） | 512×512 为通用推荐 |
| **多级细节 (LoD)** | `levelsOfDetail = 4`，缩放时先显示低分辨率再加载高清 | 缩放平滑 |
| **动态质量** | 滚动时使用低分辨率 tile，停止后加载高清 | 兼顾流畅与清晰 |

**iOS CATiledLayer 示例**：
```swift
class PDFPageLayer: CATiledLayer {
    override init() {
        super.init()
        tileSize = CGSize(width: 512, height: 512)
        levelsOfDetail = 4
        levelsOfDetailBias = 4
        backgroundColor = UIColor.clear.cgColor
    }
}
```

**性能对比（iOS 实测）**：

| 指标 | 全量预加载 | Tile-Based |
|------|-----------|------------|
| 初始加载时间 | 1.8s | 0.6s |
| 最大内存占用 | 480 MB | 65 MB |
| 滚动帧率 | 38 FPS | 58 FPS |
| 支持最大页数 | ~20 页 | >100 页 |

- **来源**：https://blog.csdn.net/weixin_42360846/article/details/151765359

### 4.2 缓存策略

| 层级 | 存储内容 | 生命周期 | 容量 |
|------|---------|---------|------|
| **内存缓存** | 当前可见 tile/页面位图 | 会话期间 | LRU，建议 3-6 页 |
| **磁盘缓存** | 最近查看的渲染结果 | 跨会话 | 自动清理 |
| **预渲染队列** | 即将显示的区域 | 临时 | 前后各 1 页缓冲 |

**关键参数**：
- `maxCacheSize`：AndroidPdfViewer 建议大文件设为 2-4 页
- 低内存设备：`maxCacheSize = 2`
- 普通设备：`maxCacheSize = 4-6`
- 动态调整：根据运行时内存压力调整缓存大小

```java
// Android 动态缓存调整
if (isLowMemoryDevice()) {
    pdfView.setMaxCacheSize(2);
} else {
    pdfView.setMaxCacheSize(6);
}
```

- **来源**：https://m.blog.csdn.net/gitblog_00469/article/details/151667687 | https://m.blog.csdn.net/gitblog_00947/article/details/151645167

### 4.3 OOM 规避

| 策略 | 说明 |
|------|------|
| **按需加载** | 仅加载可视区域 ± 1 页缓冲 |
| **位图回收** | 及时 `recycle()` 不可见的 Bitmap |
| **流式处理** | 渲染一页 → 绘制一页 → 释放一页（长图场景） |
| **分页加载** | 不一次性加载整个文档到内存 |
| **缓存上限** | 限制内存缓存页数，LRU 淘汰 |
| **降低缩放** | 滚动时临时降低渲染 DPI |

**SumatraPDF 实测数据**（基于 MuPDF 引擎）：

| 指标 | SumatraPDF | Adobe Reader | Foxit Reader |
|------|-----------|-------------|-------------|
| 初始加载 | 1.2s | 3.5s | 2.8s |
| 滚动流畅度 | 58 FPS | 42 FPS | 35 FPS |
| 内存占用 | 85 MB | 210 MB | 180 MB |

- **来源**：https://m.blog.csdn.net/gitblog_07312/article/details/148942779 | https://m.blog.csdn.net/gitblog_07038/article/details/148941912

### 4.4 文本层抽取（选择/搜索）

PDFium 提供完整的文本层 API：

```
FPDFText_LoadPage()           → 加载页面文本
FPDFText_CountChars()         → 字符总数
FPDFText_GetUnicode()         → 获取 Unicode
FPDFText_GetCharBox()         → 获取字符边界框
FPDFText_GetBoundedText()     → 获取矩形区域内文本
FPDFText_FindStart()          → 开始搜索
FPDFText_FindNext()           → 搜索下一个
FPDFText_FindPrev()           → 搜索上一个
FPDFText_FindClose()          → 关闭搜索
```

文本层用于：
1. **文本选择**：通过 `FPDFText_GetCharBox()` 获取每个字符位置，实现精确选择
2. **全文搜索**：`FPDFText_Find*()` 系列 API
3. **OCR 叠加**：扫描件可通过外部 OCR 生成隐藏文本层

### 4.5 Outlines / 书签

```
FPDFBookmark_GetFirstChild()   → 获取子书签
FPDFBookmark_GetNextSibling()  → 获取下一个兄弟书签
FPDFBookmark_GetTitle()        → 获取书签标题
FPDFBookmark_GetDest()         → 获取书签目标
FPDFBookmark_GetCount()        → 获取子节点数量 (pdfium-render 0.8.19+)
```

---

## 5. 参考实现的文件格式

### 5.1 各应用格式对比

| 应用 | 格式 | 本质 | 笔迹存储 | PDF 处理 | 同步 |
|------|------|------|---------|---------|------|
| **GoodNotes** | `.goodnotes` | ZIP 包 | 专有矢量墨水引擎 | 作为页面背景，独立存储 | iCloud |
| **Notability** | `.note` | ZIP 包 | Apple binary plist (`Session.plist` 中 `curvespoints`) | 嵌入 ZIP 内 | iCloud |
| **Xournal++** | `.xopp` | gzip 压缩 XML | XML `<stroke>` 元素含坐标+压力 | **外部引用**（不嵌入） | 无内置 |
| **Saber** | `.sbn2` | 压缩 JSON | JSON 序列化笔画数据 | 嵌入/引用 | Nextcloud 等 |

### 5.2 格式详细分析

#### GoodNotes `.goodnotes`

```
notebook.goodnotes (ZIP)
├── attachments/
│   ├── image_001.jp2        ← JPEG2000 格式图片
│   ├── image_002.jp2
│   └── ...
├── pages/
│   ├── page_001.dat         ← 页面数据（笔迹+布局）
│   └── ...
├── metadata                 ← 笔记本元数据
└── ...
```

- 图片使用 JPEG2000 压缩（可通过替换为更低质量 jp2 再压缩来瘦身）
- 笔迹使用**向量墨水引擎**存储，分辨率无关
- 支持 OCR 索引手写内容
- **来源**：https://m.bilibili.com/opus/715403829276835897

#### Notability `.note`

```
template.note (ZIP)
├── Session.plist            ← Apple 二进制 plist
│   ├── curvespoints         ← base64 编码的笔画坐标（二进制浮点数组）
│   ├── curvestyle           ← 笔画样式（颜色/粗细/工具类型）
│   └── ...
├── metadata.plist           ← 文档元数据
├── Recordings/
│   ├── library.plist        ← 音频录制索引
│   ├── Recording 1.m4a      ← 音频文件
│   └── ...
├── Images/                  ← 嵌入图片
├── *.pdf                    ← 导入的 PDF 原文
├── thumbnail, thumb2x, ...  ← 多级缩略图
└── PDFIndex.zip             ← PDF 文本索引
```

- `Session.plist` 是核心，包含所有笔画数据
- 音频与笔画时间戳关联（录音回放时笔迹同步出现）
- **来源**：https://m.freebuf.com/news/167120.html

#### Xournal++ `.xopp`

```xml
<!-- .xopp = gzip 压缩的 XML -->
<xournal version="0.4.8">
  <title>我的笔记</title>
  <page width="595" height="842">
    <background type="solid" color="#ffffff"/>
    <!-- 或 <background type="pdf" uri="file:///path/to/doc.pdf" pageno="1"/> -->
    <layer>
      <stroke tool="pen" color="#000000" width="1.5">
        <point x="100" y="200" pressure="0.8"/>
        <point x="150" y="220" pressure="0.7"/>
        <!-- 更多点 -->
      </stroke>
      <text x="300" y="300" font="Sans 12">示例文本</text>
    </layer>
  </page>
</xournal>
```

**关键特点**：
- PDF **不嵌入**，仅通过文件路径引用（移动文件时需一起携带 PDF）
- 支持多图层（`<layer>`）
- 笔画含压力数据
- 兼容旧版 `.xoj` 格式
- 导出 PDF 时批注拍平到页面
- **来源**：https://m.toutiao.com/a7649946057387835910/ | https://m.blog.csdn.net/gitblog_00965/article/details/151628886

#### Saber `.sbn2`

- 基于 Flutter/Dart 开发
- 专有 SBN2 格式：
  - 压缩存储结构
  - 内容序列化为 JSON
  - 增量更新机制
  - 端到端加密支持（双密码系统）
- 支持导入/导出 PDF
- 笔画数据 JSON 序列化后压缩
- **来源**：https://m.blog.csdn.net/gitblog_00199/article/details/157239360 | https://blog.csdn.net/weixin_51215591/article/details/163954153

### 5.3 格式设计启示

| 设计决策 | 推荐做法 | 理由 |
|---------|---------|------|
| 容器格式 | ZIP 包 | 跨平台通用、支持压缩、易于扩展 |
| 笔迹数据 | JSON 或二进制序列化 | JSON 可读性好、调试方便；二进制性能更好 |
| PDF 存储 | 嵌入 ZIP 内（非外部引用） | Xournal++ 外部引用的方式导致文件分离，用户体验差 |
| 缩略图 | 多级 PNG（1x/2x/3x） | 快速预览，无需渲染整页 |
| 元数据 | JSON/plist | 页面顺序、创建时间、标签等 |
| 增量保存 | 仅修改变更的页面文件 | 避免每次保存整个文档 |

---

## 6. 推荐结论

### 6.1 引擎推荐：PDFium（主引擎）+ pdf-lib（Web 辅助）

**推荐理由**：

1. **许可证友好**：Apache-2.0/BSD，商业闭源无顾虑。MuPDF 的 AGPL 对商业项目有传染性风险。
2. **跨平台覆盖最全**：Android（系统级支持）/ iOS / Windows / macOS / Linux / WASM，一套引擎全平台统一。
3. **编辑能力充足**：429+ API 覆盖注解（含 Ink Annotation）、页面操作、字体嵌入、扁平化导出全部需求。
4. **社区生态成熟**：
   - 预编译二进制（bblanchon/pdfium-binaries）免去编译痛苦
   - Flutter 封装（pdfium_dart/pdfium_flutter）适合跨平台移动端
   - Rust 封装（pdfium-render）适合桌面/WASM
5. **性能优秀**：Chrome 内核级渲染引擎，渲染质量和速度均处于第一梯队。
6. **成本为零**：完全免费，无商业许可费用。

**不选 MuPDF 的原因**：AGPL 许可对商业闭源项目有传染性，除非购买商业许可（价格不透明）。

**不选商用 SDK 的原因**：PSPDFKit/Apryse 功能最全面但价格高昂（$1,500+/年起），适合后期产品验证成功后按需升级。

**Web/WASM 补充**：pdf.js 做渲染查看 + pdf-lib 做注解写入，或 PDFium WASM 统一方案。

### 6.2 架构推荐

```
┌──────────────────────────────────────────────────────────────┐
│                        应用层                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────┐  │
│  │ 查看器    │  │ 批注工具  │  │ 手写引擎  │  │ 页面管理器  │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └──────┬──────┘  │
│       │              │              │               │          │
│  ┌────┴──────────────┴──────────────┴───────────────┴──────┐  │
│  │                  PDFium 统一接口层                         │  │
│  │  FPDF_Render / FPDFAnnot_* / FPDFPage_* / FPDF_*Edit    │  │
│  └────────────────────────┬────────────────────────────────┘  │
│                           │                                    │
│  ┌────────────────────────┴────────────────────────────────┐  │
│  │              原生文件存储层                                │  │
│  │  .notebook (ZIP)                                         │  │
│  │  ├── document.pdf      (原始 PDF)                         │  │
│  │  ├── pages/            (每页元数据 JSON)                   │  │
│  │  ├── strokes/          (笔迹数据 binary/JSON)             │  │
│  │  ├── thumbnails/       (缩略图 PNG)                       │  │
│  │  └── manifest.json     (文档结构索引)                      │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │              导出层                                        │   │
│  │  笔迹 → PDF Ink Annotation (FPDFAnnot_AddInkStroke)       │   │
│  │  或 笔迹 → PDF Path Object (写入内容流)                    │   │
│  │  FPDFPage_Flatten() → 扁平化导出                           │   │
│  └──────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

---

## 7. PDF 编辑 API 映射表

### 功能 → PDFium API 对照

| 功能需求 | PDFium API | 头文件 | 备注 |
|---------|-----------|--------|------|
| **查看** | | | |
| 加载文档 | `FPDF_LoadDocument()` / `FPDF_LoadMemDocument()` | fpdfview.h | 支持文件路径/内存 |
| 渲染页面 | `FPDF_RenderPageBitmap()` / `FPDF_RenderPageBitmapWithMatrix()` | fpdfview.h | 支持变换矩阵 |
| 获取页面数 | `FPDF_GetPageCount()` | fpdfview.h | |
| 获取页面尺寸 | `FPDF_GetPageSizeByIndex()` / `FPDF_GetPageWidthF()` | fpdfview.h | |
| 渐进式渲染 | `FPDF_RenderPageBitmap_Start()` / `FPDF_RenderPage_Continue()` | fpdfview.h | 大页面分步渲染 |
| **批注** | | | |
| 创建文本注解 | `FPDFPage_CreateAnnot(doc, FPDF_ANNOT_TEXT)` | fpdf_annot.h | |
| 创建高亮注解 | `FPDFPage_CreateAnnot(doc, FPDF_ANNOT_HIGHLIGHT)` | fpdf_annot.h | |
| 创建 Ink 注解 | `FPDFPage_CreateAnnot(doc, FPDF_ANNOT_INK)` | fpdf_annot.h | 手写笔迹 |
| 添加 Ink 笔画 | `FPDFAnnot_AddInkStroke()` | fpdf_annot.h | ISO 32000 InkList |
| 设置注解颜色 | `FPDFAnnot_SetColor()` | fpdf_annot.h | 描边/填充色 |
| 设置注解位置 | `FPDFAnnot_SetRect()` | fpdf_annot.h | |
| 设置注解标志 | `FPDFAnnot_SetFlags()` | fpdf_annot.h | Print/NoZoom 等 |
| 删除注解 | `FPDFPage_RemoveAnnot()` | fpdf_annot.h | |
| 获取注解列表 | `FPDFPage_GetAnnot()` / `FPDFPage_GetAnnotCount()` | fpdf_annot.h | |
| 设置外观流 | `FPDFAnnot_SetAP()` | fpdf_annot.h | 自定义渲染外观 |
| **手写叠加** | | | |
| 创建 Ink 注解 | `FPDFPage_CreateAnnot() + FPDFAnnot_AddInkStroke()` | fpdf_annot.h | 标准方式 |
| 压感数据 | 通过 `FPDFAnnot_SetStringValue()` 存储自定义数据 | fpdf_annot.h | PDF 标准不直接支持压力 |
| **页面管理** | | | |
| 创建新页面 | `FPDFPage_New()` | fpdf_edit.h | 指定宽高 |
| 删除页面 | `FPDF_DeletePage()` | fpdfview.h | |
| 旋转页面 | `FPDFPage_SetRotation()` | fpdf_edit.h | 0/90/180/270 |
| 导入页面 | `FPDF_ImportPages()` | fpdf_ppo.h | 跨文档 |
| 移动页面 | `FPDF_MovePages()` | fpdfview.h | 新版 API |
| 合并文档 | `FPDF_ImportPages()` 循环导入 | fpdf_ppo.h | 逐页导入合并 |
| 设置页面边界 | `FPDFPage_SetMediaBox()` / `FPDFPage_SetCropBox()` | fpdf_edit.h | |
| **字体** | | | |
| 加载 TrueType 字体 | `FPDFText_LoadFont()` | fpdf_edit.h | 嵌入字体到 PDF |
| 加载标准字体 | `FPDFText_LoadStandardFont()` | fpdf_edit.h | Helvetica 等 14 种 |
| 加载 CJK 字体 | `FPDFText_LoadCidType2Font()` | fpdf_edit.h | 中日韩文字支持 |
| 添加文本 | `FPDFPageObj_NewTextObj() + FPDFText_SetText()` | fpdf_edit.h | |
| **内容编辑** | | | |
| 创建文本对象 | `FPDFPageObj_NewTextObj()` | fpdf_edit.h | |
| 创建路径对象 | `FPDFPageObj_CreateNewPath()` | fpdf_edit.h | 矢量图形 |
| 创建图像对象 | `FPDFPageObj_NewImageObj()` | fpdf_edit.h | |
| 插入对象到页面 | `FPDFPage_InsertObject()` | fpdf_edit.h | |
| 移除对象 | `FPDFPage_RemoveObject()` | fpdf_edit.h | |
| 变换对象 | `FPDFPageObj_Transform()` | fpdf_edit.h | 矩阵变换 |
| 重建内容流 | `FPDFPage_GenerateContent()` | fpdf_edit.h | 修改后必须调用 |
| **导出** | | | |
| 扁平化 | `FPDFPage_Flatten()` | fpdf_flatten.h | 注解→页面内容 |
| 保存副本 | `FPDF_SaveAsCopy()` | fpdf_save.h | 全量保存 |
| 增量保存 | `FPDF_SaveAsCopy()` + FPDF_INCREMENTAL | fpdf_save.h | |
| **文本层** | | | |
| 提取文本 | `FPDFText_GetText()` | fpdf_text.h | |
| 字符位置 | `FPDFText_GetCharBox()` | fpdf_text.h | 用于选择/高亮 |
| 文本搜索 | `FPDFText_FindStart/Next/Prev/Close()` | fpdf_text.h | |
| **书签/大纲** | | | |
| 获取书签 | `FPDFBookmark_GetFirstChild/NextSibling/Title/Dest()` | fpdf_doc.h | |

---

## 8. 存储格式建议

### 8.1 推荐格式设计

**格式名称**：`.notebook`（或自定义扩展名）

**容器**：ZIP 压缩包（跨平台通用，各语言均有标准库支持）

```
notebook.zip (.notebook)
├── manifest.json              ← 文档级元数据与索引
├── document.pdf               ← 原始 PDF 文件（可选，空白笔记时无此文件）
├── thumbnails/
│   ├── page_001.png           ← 1x 缩略图
│   ├── page_001@2x.png        ← 2x 缩略图
│   └── ...
├── pages/
│   ├── page_001.json          ← 页面元数据（尺寸/旋转/背景类型/图层列表）
│   ├── page_001.strokes       ← 笔迹数据（二进制格式，含坐标/压力/时间/工具）
│   ├── page_002.json
│   ├── page_002.strokes
│   └── ...
├── assets/
│   ├── image_001.png          ← 嵌入的图片
│   └── ...
└── audio/
    ├── recording_001.m4a      ← 录音（可选）
    └── recording_index.json   ← 录音时间戳索引
```

### 8.2 manifest.json 结构

```json
{
  "version": 1,
  "app_version": "1.0.0",
  "created_at": "2026-10-12T10:00:00Z",
  "modified_at": "2026-10-12T12:00:00Z",
  "title": "我的笔记",
  "tags": ["工作", "会议"],
  "pages": [
    {
      "id": "page_001",
      "width": 595.0,
      "height": 842.0,
      "rotation": 0,
      "background": {
        "type": "pdf",
        "pdf_page": 0
      },
      "layers": [
        {
          "id": "layer_0",
          "name": "笔迹层",
          "visible": true,
          "locked": false,
          "strokes_file": "page_001.strokes"
        }
      ]
    }
  ],
  "pdf_page_count": 10,
  "outline": []
}
```

### 8.3 笔迹数据格式（page_xxx.strokes）

推荐使用紧凑的二进制格式（非 JSON），以优化大文档性能：

```
[Header]
  magic: 4 bytes ("STRK")
  version: uint16
  stroke_count: uint32

[Stroke] × stroke_count
  tool_type: uint8        (0=pen, 1=pencil, 2=highlighter, 3=eraser)
  color: uint32           (RGBA)
  line_width: float32
  point_count: uint32
  timestamp_start: uint64 (毫秒)

  [Point] × point_count
    x: float32
    y: float32
    pressure: float16     (0.0 ~ 1.0)
    timestamp: uint32     (相对起始时间的偏移，毫秒)
```

**估算大小**：每个点 ~14 字节。一笔画 100 个点 ≈ 1.4KB + 笔画头 ≈ 1.5KB。一页 50 笔 ≈ 75KB。100 页笔记 ≈ 7.5MB（纯笔迹数据，压缩后更小）。

### 8.4 导出扁平化 PDF 流程

```
1. 加载原始 PDF → FPDF_LoadDocument()
2. 遍历每页：
   a. 读取该页笔迹数据
   b. 为每条笔画创建 Ink Annotation：
      FPDFPage_CreateAnnot(page, FPDF_ANNOT_INK)
      FPDFAnnot_AddInkStroke(annot, points, count)
      FPDFAnnot_SetColor(annot, ..., R, G, B, A)
      FPDFAnnot_SetFlags(annot, FPDF_ANNOT_FLAG_PRINT)
   c. 或：将笔迹转为 Path Object 写入内容流（更灵活）：
      FPDFPageObj_CreateNewPath()
      逐点 FPDFPath_MoveTo() / FPDFPath_LineTo()
      FPDFPageObj_SetStrokeColor/Width()
      FPDFPage_InsertObject()
      FPDFPage_GenerateContent()
3. 扁平化（可选）：
   FPDFPage_Flatten(page, FPDF_ANNOT)  // 注解→内容
4. 保存：
   FPDF_SaveAsCopy(doc, writer, 0)
```

### 8.5 与业界做法对比

| 维度 | 本方案 | GoodNotes | Notability | Xournal++ |
|------|--------|-----------|------------|-----------|
| 容器 | ZIP | ZIP | ZIP | gzip XML |
| PDF 存储 | 嵌入 | 嵌入 | 嵌入 | **外部引用** |
| 笔迹格式 | 自定义二进制 | 专有矢量 | plist 二进制 | XML 文本 |
| 元数据 | JSON | 专有 | plist | XML 属性 |
| 多图层 | ✅ | ✅ | ❌ | ✅ |
| 增量保存 | ✅（按页文件） | 未知 | 未知 | ❌（全量保存） |
| 音频录制 | ✅（可选） | ✅ | ✅ | ✅ |
| 加密 | 可选（ZIP 密码） | ❌ | ❌ | ❌ |

---

## 附录：关键来源索引

| # | 来源 | URL |
|---|------|-----|
| 1 | PDFium 源码 | https://github.com/chromium/pdfium |
| 2 | pdfium-binaries 预编译 | https://github.com/bblanchon/pdfium-binaries |
| 3 | pdfium-render (Rust) | https://github.com/ajrcarey/pdfium-render |
| 4 | pdfium-lib (多平台编译) | https://github.com/paulocoutinho/pdfium-lib |
| 5 | MuPDF 官网 | https://mupdf.com |
| 6 | Apache PDFBox | https://github.com/apache/pdfbox |
| 7 | pdf-lib (JS) | https://github.com/Hopding/pdf-lib |
| 8 | PSPDFKit vs Apryse 对比 | https://www.nutrient.io/sdk/vs/apryse |
| 9 | Apryse 定价 | https://apryse.com/ |
| 10 | Xournal++ 文件格式 | https://m.toutiao.com/a7649946057387835910/ |
| 11 | Notability 逆向工程 | https://m.freebuf.com/news/167120.html |
| 12 | GoodNotes 格式分析 | https://m.bilibili.com/opus/715403829276835897 |
| 13 | Saber 技术架构 | https://m.blog.csdn.net/gitblog_00199/article/details/157239360 |
| 14 | iOS PDF 渲染优化 | https://blog.csdn.net/weixin_42360846/article/details/151765359 |
| 15 | AndroidPdfViewer 内存优化 | https://m.blog.csdn.net/gitblog_00469/article/details/151667687 |
| 16 | SumatraPDF 性能优化 | https://m.blog.csdn.net/gitblog_07312/article/details/148942779 |
| 17 | PDFium 头文件说明 | https://blog.csdn.net/m0_53768564/article/details/128866008 |
| 18 | PDF 注解 JS 完整概览 | https://www.nutrient.io/blog/pdf-annotations-with-javascript-a-complete-overview/ |
| 19 | PSPDFKit 功能列表 | https://www.nutrient.io/sdk/features-list/ |
| 20 | Rust PDF 库对比 | https://bbs.itying.com/topic/68f71d1f61dd0500413cec53 |
| 21 | PDFium Dart/Flutter 绑定 | https://pub.dev/packages/pdfium_dart |
| 22 | printing (Flutter PDF) | https://pub.dev/packages/printing |
| 23 | @embedpdf/pdfium (WASM) | https://www.npmjs.com/package/@embedpdf/pdfium |
| 24 | PDF Generation API 对比 | https://segmentfault.com/a/1190000047916465 |
| 25 | MuPDF 许可说明 | https://m.weibo.cn/status/OlCh8EaAg |
| 26 | GoodNotes 支持文档 | https://support.goodnotes.com/hc/zh-cn/articles/4599403401359 |
| 27 | Notability 文件格式 | https://wenku.csdn.net/doc/6792u7t75r |
| 28 | Xournal++ .xopp 格式详解 | https://m.blog.csdn.net/gitblog_00965/article/details/151628886 |
| 29 | pdf.js Web 端高效渲染 | https://blog.csdn.net/weixin_42400643/article/details/152602790 |
| 30 | Android PDF 库性能对比 | https://m.blog.csdn.net/gitblog_00590/article/details/154974491 |
