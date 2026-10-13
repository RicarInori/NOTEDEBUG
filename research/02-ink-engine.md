# 高性能手写笔迹渲染引擎技术调研报告

> 调研日期：2026-10-12  
> 调研目标：跨平台笔记软件（Android/iOS/桌面/Web）的笔迹渲染引擎技术选型  
> 核心指标：低延迟、压感支持、跨平台一致性

---

## 一、手写输入延迟的来源与业界优化手段

### 1.1 延迟预算表（Touch-to-Pixel Pipeline）

| 环节 | 典型延迟 | 优化手段 | 备注 |
|------|---------|---------|------|
| **触控采样** | 1-10 ms | 高触控采样率（120-1000Hz） | 触摸屏 ADC 转换 |
| **事件批处理** | 0-8 ms | MotionEvent 批处理、历史点提取 | Android 将多点打包 |
| **事件分发** | 1-3 ms | 减少监听器、避免阻塞 | 系统层开销 |
| **VSync 等待** | 0-16.67 ms | 预测算法、前置缓冲 | 60Hz=16.67ms, 120Hz=8.33ms |
| **应用处理** | 1-5 ms | 避免对象创建、优化算法 | 插值/平滑/压感映射 |
| **GPU 渲染** | 2-8 ms | 前置缓冲、Shader 预编译 | 光栅化开销 |
| **显示刷新** | 0-16.67 ms | 高刷新率屏幕 | 像素响应时间 |
| **端到端总计** | **20-70 ms** | - | 业界目标 <40ms |

**关键阈值**：
- <40ms：用户感知为"即时响应"
- 40-80ms：可接受但能感知延迟
- \>80ms：明显卡顿感

> 来源：[触摸延迟系统级优化](https://wenku.csdn.net/column/5dkv9sivw7)

### 1.2 Android 低延迟优化方案

#### 1.2.1 前置缓冲渲染（Front-Buffered Rendering）

**技术原理**：  
传统双缓冲需要等待完整帧提交，前置缓冲允许局部内容提前进入显示链路，缩短笔迹首次可见时间。

**官方 API**（Android 13+）：
- `GLFrontBufferedRenderer`：OpenGL 低延迟渲染
- `CanvasFrontBufferedRenderer`：Canvas 低延迟渲染
- `LowLatencyCanvasView`：Compose 低延迟画布
- `CanvasBufferedRenderer`：通用缓冲渲染器

**使用示例**（Compose）：
```kotlin
@Composable
fun DrawArea(modifier: Modifier = Modifier) {
    Canvas(
        modifier = modifier
            .clipToBounds()
            .pointerInteropFilter { viewModel.processMotionEvent(it) }
    ) {
        // 低延迟绘制代码
    }
}
```

> 来源：[Android 手写渲染技术演进](https://juejin.cn/post/7642626890507796531)

#### 1.2.2 MotionEvent 批处理与历史点

**核心机制**：  
触摸屏采样率（100-1000Hz）高于屏幕刷新率（60-120Hz），Android 将多个触摸点打包到一个 `MotionEvent` 中，通过 `getHistorical*()` API 提取所有采样点。

**代码示例**：
```kotlin
override fun onTouchEvent(event: MotionEvent): Boolean {
    val historySize = event.historySize
    for (i in 0 until historySize) {
        val x = event.getHistoricalX(i)
        val y = event.getHistoricalY(i)
        val pressure = event.getHistoricalPressure(i)
        val time = event.getHistoricalEventTime(i)
        // 处理历史点
        processPoint(x, y, pressure, time)
    }
    // 处理当前点
    processPoint(event.x, event.y, event.pressure, event.eventTime)
    return true
}
```

**优势**：
- 采样率从屏幕刷新率（60-120Hz）提升到触摸屏采样率（100-1000Hz）
- 笔迹更密集、更平滑
- 对压感笔效果提升显著

> 来源：[Android 画图白板 APP 开发](https://blog.csdn.net/indeedes/article/details/150341495)

#### 1.2.3 运动预测库（MotionEventPredictor）

**官方库**：`androidx.input:input-motionprediction:1.0.0-beta01`

**工作原理**：
1. `record(motionEvent)`：记录真实 MotionEvent
2. `predict()`：预测未来 MotionEvent（临时点）
3. 预测点仅用于可视化，真实点到达后替换预测点

**代码示例**：
```kotlin
var motionEventPredictor = MotionEventPredictor.newInstance(view)

// 在 onTouchEvent 中
motionEventPredictor.record(motionEvent)

if (motionEvent.action == MotionEvent.ACTION_MOVE) {
    val predictedEvent = motionEventPredictor.predict()
    if (predictedEvent != null) {
        // 用预测点绘制临时笔迹
        drawTemporaryStroke(predictedEvent.x, predictedEvent.y)
    }
}
```

**注意事项**：
- 预测点不能作为最终持久化数据
- 仅用于改善主观跟手性
- 支持 Android 4.4+（API 19+）

> 来源：[Advanced Stylus Features](https://developer.android.com/develop/ui/compose/touch-input/stylus-input/advanced-stylus-features)

#### 1.2.4 Google Ink API（androidx.ink）

**发布时间**：2025 年 12 月 17 日正式发布 1.0.0 稳定版

**核心特性**：
- **超低延迟**：基于 Jetpack Graphics Library，4ms 级响应
- **模块化设计**：按需引入
- **Compose 优先**：完美适配 Jetpack Compose
- **广泛兼容**：支持 Android 5.0+（API 21+）

**模块架构**：
```
┌─────────────────────────────────────────────────────────┐
│                    Ink API                                │
├──────────────┬──────────────┬──────────────┬────────────┤
│   Strokes    │    Brush     │   Geometry   │  Rendering │
│   笔触模块    │   画笔模块    │   几何模块    │   渲染模块  │
├──────────────┴──────────────┴──────────────┴────────────┤
│              Authoring (实时创作模块)                      │
└─────────────────────────────────────────────────────────┘
```

**核心模块**：
- `ink-strokes`：墨迹输入的数据结构
- `ink-brush`：声明式画笔风格
- `ink-geometry`：几何操作（擦除、选择、碰撞检测）
- `ink-rendering`：高效渲染笔触
- `ink-authoring`：实时输入处理，生成平滑笔触
- `ink-storage`：笔触序列化与持久化

**使用示例**：
```kotlin
dependencies {
    val ink_version = "1.0.0"
    implementation("androidx.ink:ink-nativeloader:$ink_version")
    implementation("androidx.ink:ink-strokes:$ink_version")
    implementation("androidx.ink:ink-rendering:$ink_version")
    implementation("androidx.ink:ink-authoring-compose:$ink_version")
    implementation("androidx.ink:ink-brush-compose:$ink_version")
}

// 创建压感画笔
val blackPen = Brush.createWithComposeColor(
    family = StockBrushes.pressurePen(),
    colorIntArgb = Color.Black.toArgb(),
    size = 5f,
    epsilon = 0.1f
)
```

**实际应用**：
- Google Docs（文档标注）
- Google Photos（照片编辑）
- Chrome PDF（PDF 批注）
- Circle to Search（圈选搜索）
- Pixel Studio（创意绘画）

> 来源：[Android Ink API 1.0 正式版](https://juejin.cn/post/7595478717985062921)

### 1.3 iOS 低延迟优化方案

#### 1.3.1 PencilKit 框架

**核心特性**：
- **低延迟捕获**：专为 Apple Pencil 优化的触摸捕获
- **预测触碰**：iOS 13 大幅改进预测功能
- **Metal 渲染**：必须在 Metal 中渲染以实现最低延迟

**延迟优化要点**（来自 WWDC 2019）：
1. **使用 Metal 渲染**：每帧只有几毫秒，逐帧重复渲染
2. **使用预测触碰**：iOS 13 大幅改进预测功能
3. **避免透明 Metal 图层**：不要在 Metal 图层上叠加模糊效果
4. **避免导航栏**：主界面可视性会为渲染增加额外成本

**代码示例**：
```swift
import PencilKit

class DrawingViewController: UIViewController {
    var canvasView: PKCanvasView!
    
    override func viewDidLoad() {
        super.viewDidLoad()
        
        canvasView = PKCanvasView(frame: view.bounds)
        canvasView.drawingPolicy = .pencilOnly  // 仅 Apple Pencil
        canvasView.tool = PKInkingTool(.pen, color: .black, width: 5)
        view.addSubview(canvasView)
    }
}
```

> 来源：[PencilKit 简介 WWDC 2019](https://developer.apple.com/cn/videos/play/wwdc2019/221/)

#### 1.3.2 120Hz ProMotion 适配

**延迟对比**：
| 设备组合 | 屏幕刷新率 | 实测延迟 | 适用场景 |
|---------|-----------|---------|---------|
| iPad Pro M4 + Apple Pencil Pro | 120Hz ProMotion | 8.7ms | 专业设计 |
| iPad Air M3 + Apple Pencil Pro | 120Hz ProMotion | ~9ms | 进阶插画 |
| iPad 11 + Apple Pencil (USB-C) | 60Hz | 9-12ms | 日常笔记 |

**关键发现**：
- 120Hz 设备平均触控响应时间：**28ms**
- 60Hz 设备平均触控响应时间：**45ms**
- 延迟降低：**38%**
- 高刷屏是消除拖影的关键，而非仅依赖低延迟笔

**注意事项**：
- Apple Pencil Pro 新增侧旋感应、触觉反馈、悬停预览
- 基础延迟方面，Pro 与二代笔无显著差距
- 延迟优化的核心变量是屏幕刷新率与系统协同

> 来源：[120Hz 手写流畅关键](https://post.m.smzdm.com/p/ad7637mx/)

### 1.4 120Hz 刷新率适配策略

**帧预算对比**：
| 刷新率 | 帧预算 | 优化策略 |
|-------|-------|---------|
| 60Hz | 16.67ms | 前置缓冲、预测、批处理 |
| 90Hz | 11.11ms | 前置缓冲、预测 |
| 120Hz | 8.33ms | 前置缓冲、减少 CPU 开销 |

**Flutter 中的问题**：  
Flutter iOS 的 `SmoothPointerDataDispatcher` 会将 99% 的 pointer packet 压到下一个 VSync，导致：
- 120Hz 下增加 8.33ms 延迟
- 60Hz 下增加 16.67ms 延迟

**解决方案**：
- 移除 Smooth 调度器（需解决 VSync waiter 调度缺陷）
- 使用原生平台视图（Platform View）直接渲染

> 来源：[Flutter iOS 深度优化 PR](https://juejin.cn/post/7677435711277629481)

---

## 二、笔迹渲染算法

### 2.1 渲染管线架构

```
┌─────────────────────────────────────────────────────────────┐
│                    笔迹渲染管线                                │
└─────────────────────────────────────────────────────────────┘

输入点采样
    ↓
[1] 点预处理（去噪、重采样）
    ↓
[2] 平滑/插值（贝塞尔/Catmull-Rom/One Euro Filter）
    ↓
[3] 压感宽度映射（pressure/velocity → width）
    ↓
[4] 轮廓生成（stamp/strip 方案）
    ├─ 位图盖章（Stamp）
    └─ 网格生成（Triangle Strip / Tessellation）
    ↓
[5] GPU 渲染
    ├─ 顶点着色器（变换、投影）
    └─ 片元着色器（颜色、纹理、抗锯齿）
    ↓
[6] 显示输出
```

### 2.2 输入点采样与预处理

#### 2.2.1 采样率差异

| 设备类型 | 采样率 | 点间隔 |
|---------|-------|-------|
| 普通触摸屏 | 60-120Hz | 8.3-16.7ms |
| 高触控采样屏 | 240-480Hz | 2.1-4.2ms |
| Apple Pencil | 240Hz | 4.2ms |
| Samsung S Pen | 240Hz | 4.2ms |
| Wacom 数位板 | 133-266Hz | 3.8-7.5ms |

#### 2.2.2 重采样策略

**问题**：采样点分布不均匀（速度快时间隔大，速度慢时间隔小）

**解决方案**：按距离重采样
```python
def resample_points(points, target_distance=2.0):
    """按固定距离重采样点"""
    if len(points) < 2:
        return points
    
    resampled = [points[0]]
    accumulated_distance = 0.0
    
    for i in range(1, len(points)):
        p0 = points[i-1]
        p1 = points[i]
        dx = p1.x - p0.x
        dy = p1.y - p0.y
        segment_length = math.sqrt(dx*dx + dy*dy)
        
        if segment_length == 0:
            continue
        
        # 方向向量
        dir_x = dx / segment_length
        dir_y = dy / segment_length
        
        # 在当前线段上按 target_distance 插值
        while accumulated_distance + segment_length >= target_distance:
            remaining = target_distance - accumulated_distance
            new_x = p0.x + dir_x * remaining
            new_y = p0.y + dir_y * remaining
            resampled.append(Point(new_x, new_y, p1.pressure))
            
            # 更新起点
            p0 = Point(new_x, new_y, p0.pressure)
            segment_length -= remaining
            accumulated_distance = 0.0
        
        accumulated_distance += segment_length
    
    resampled.append(points[-1])
    return resampled
```

### 2.3 平滑与插值算法

#### 2.3.1 二次贝塞尔中点法（Mid-Point Smoothing）

**原理**：用相邻点的中点作为贝塞尔曲线的端点，原始点作为控制点

**伪代码**：
```python
def draw_smooth_curve(points):
    """二次贝塞尔中点平滑"""
    if len(points) < 2:
        return
    
    path = Path()
    path.move_to(points[0].x, points[0].y)
    
    for i in range(1, len(points) - 1):
        # 当前点和下一个点的中点
        mid_x = (points[i].x + points[i+1].x) / 2
        mid_y = (points[i].y + points[i+1].y) / 2
        
        # 用当前点作为控制点，中点作为端点
        path.quad_to(points[i].x, points[i].y, mid_x, mid_y)
    
    # 连接最后一个点
    if len(points) > 1:
        path.line_to(points[-1].x, points[-1].y)
    
    return path
```

**优点**：
- 实现简单，计算量小
- 曲线通过所有中点，视觉平滑
- 适合实时绘制

**缺点**：
- 曲线不通过原始采样点（通过中点）
- 对噪声敏感

> 来源：[Android 画图白板 APP 开发](https://blog.csdn.net/indeedes/article/details/150264843)

#### 2.3.2 Catmull-Rom 样条插值

**原理**：C1 连续的样条曲线，通过所有控制点

**公式**：
```
q(t) = 0.5 * (
    (2 * p1) +
    (-p0 + p2) * t +
    (2*p0 - 5*p1 + 4*p2 - p3) * t^2 +
    (-p0 + 3*p1 - 3*p2 + p3) * t^3
)
```

**伪代码**：
```python
def catmull_rom(p0, p1, p2, p3, t):
    """Catmull-Rom 插值"""
    t2 = t * t
    t3 = t2 * t
    
    x = 0.5 * (
        (2 * p1.x) +
        (-p0.x + p2.x) * t +
        (2*p0.x - 5*p1.x + 4*p2.x - p3.x) * t2 +
        (-p0.x + 3*p1.x - 3*p2.x + p3.x) * t3
    )
    
    y = 0.5 * (
        (2 * p1.y) +
        (-p0.y + p2.y) * t +
        (2*p0.y - 5*p1.y + 4*p2.y - p3.y) * t2 +
        (-p0.y + 3*p1.y - 3*p2.y + p3.y) * t3
    )
    
    return Point(x, y)

def smooth_with_catmull_rom(points, segments=10):
    """用 Catmull-Rom 平滑曲线"""
    if len(points) < 4:
        return points
    
    smoothed = []
    
    for i in range(len(points) - 3):
        p0, p1, p2, p3 = points[i], points[i+1], points[i+2], points[i+3]
        
        # 在 p1 和 p2 之间插值
        for j in range(segments):
            t = j / segments
            smoothed.append(catmull_rom(p0, p1, p2, p3, t))
    
    smoothed.append(points[-2])
    return smoothed
```

**优点**：
- 曲线通过所有控制点
- C1 连续（切线连续）
- 转角处自动圆滑

**缺点**：
- 计算量较大
- 需要 4 个点才能插值

> 来源：[Unity 电子签名系统](https://www.cnblogs.com/DYSY/p/20019145)

#### 2.3.3 One Euro Filter

**原理**：自适应低通滤波器，根据速度动态调整截止频率

**核心思想**：
- 速度慢时：增加滤波强度（降低截止频率），消除抖动
- 速度快时：减少滤波强度（提高截止频率），降低延迟

**伪代码**：
```python
import math
import numpy as np

def smoothing_factor(t_e, cutoff):
    """计算平滑因子"""
    r = 2 * math.pi * cutoff * t_e
    return r / (r + 1)

def exponential_smoothing(a, x, x_prev):
    """指数平滑"""
    return a * x + (1 - a) * x_prev

class OneEuroFilter:
    def __init__(self, t0, x0, min_cutoff=1.0, beta=0.0, d_cutoff=1.0):
        """
        初始化 One Euro Filter
        
        Args:
            t0: 初始时间
            x0: 初始值
            min_cutoff: 最小截止频率 (Hz)，值越小越平滑
            beta: 速度系数，值越大响应越快
            d_cutoff: 导数截止频率 (Hz)
        """
        self.min_cutoff = min_cutoff
        self.beta = beta
        self.d_cutoff = d_cutoff
        
        self.x_prev = np.array(x0, dtype=float)
        self.dx_prev = np.zeros_like(self.x_prev)
        self.t_prev = t0
    
    def __call__(self, t, x):
        """
        滤波
        
        Args:
            t: 当前时间
            x: 当前值 (numpy array)
        """
        t_e = t - self.t_prev
        
        # 滤波导数（速度）
        a_d = smoothing_factor(t_e, self.d_cutoff)
        dx = (x - self.x_prev) / t_e
        dx_hat = exponential_smoothing(a_d, dx, self.dx_prev)
        
        # 动态计算截止频率
        cutoff = self.min_cutoff + self.beta * np.abs(dx_hat)
        
        # 滤波主信号
        a = smoothing_factor(t_e, cutoff)
        x_hat = exponential_smoothing(a, x, self.x_prev)
        
        # 更新状态
        self.x_prev = x_hat
        self.dx_prev = dx_hat
        self.t_prev = t
        
        return x_hat

# 使用示例
filter = OneEuroFilter(t0=0, x0=[0, 0], min_cutoff=1.0, beta=0.01)

for t, x, y in input_points:
    filtered = filter(t, np.array([x, y]))
    # 使用 filtered[0], filtered[1]
```

**参数调优**：
- `min_cutoff=1.0`：静止时平滑
- `beta=0.01`：快速移动时响应
- `d_cutoff=1.0`：速度平滑

**优点**：
- 自适应，无需手动切换模式
- 实现简单，计算量小
- 在抖动和延迟之间取得良好平衡

**缺点**：
- 参数需要调优
- 对突发噪声敏感

> 来源：[One Euro Filter 实现](https://blog.csdn.net/caokaifa/article/details/155966079)

### 2.4 压感宽度映射

#### 2.4.1 压力映射

**线性映射**：
```python
def pressure_to_width(pressure, min_width=1.0, max_width=10.0):
    """压力到宽度的线性映射"""
    return min_width + pressure * (max_width - min_width)
```

**Sigmoid 映射**（更自然）：
```python
def pressure_to_width_sigmoid(pressure, min_width=1.0, max_width=10.0):
    """Sigmoid 映射，中间段更敏感"""
    # 将 [0,1] 映射到 [-3,3]
    x = (pressure - 0.5) * 6
    # Sigmoid
    s = 1 / (1 + math.exp(-x))
    return min_width + s * (max_width - min_width)
```

#### 2.4.2 速度映射（无压感设备）

**原理**：速度快 → 笔画细，速度慢 → 笔画粗

**伪代码**：
```python
def velocity_to_width(v0, v1, t0, t1, base_width=5.0):
    """速度到宽度映射"""
    dx = v1.x - v0.x
    dy = v1.y - v0.y
    distance = math.sqrt(dx*dx + dy*dy)
    dt = t1 - t0
    
    if dt == 0:
        return base_width
    
    speed = distance / dt  # 像素/毫秒
    
    # Sigmoid 映射
    # 中心点设为 0.5 像素/毫秒
    width_factor = 1.0 / (1.0 + math.exp(0.01 * (speed - 0.5)))
    
    # 宽度范围 [0.5*base, 2.0*base]
    return base_width * (0.5 + 1.5 * width_factor)
```

> 来源：[手写模拟器工具](https://blog.csdn.net/weixin_42613018/article/details/153623756)

### 2.5 轮廓生成方案对比

#### 2.5.1 位图盖章（Stamp）方案

**原理**：在每个采样点绘制一个圆形/椭圆形位图，通过重叠形成笔迹

**伪代码**：
```python
def render_with_stamp(points, get_width):
    """位图盖章渲染"""
    for point in points:
        width = get_width(point)
        radius = width / 2
        
        # 在 point 位置绘制圆形
        draw_circle(
            center=(point.x, point.y),
            radius=radius,
            color=point.color,
            alpha=point.alpha
        )
```

**优点**：
- 实现极其简单
- 天然支持变宽
- 无需复杂几何计算

**缺点**：
- 点间距大时出现断裂
- 需要高采样率
- 无法实现复杂笔刷效果
- GPU 开销大（大量圆形绘制）

**适用场景**：
- 简单笔迹
- 低性能设备
- 快速原型

#### 2.5.2 网格生成（Triangle Strip）方案

**原理**：根据笔迹中心线和宽度，生成左右两侧的轮廓点，形成三角带

**伪代码**：
```python
def generate_stroke_mesh(points, get_width):
    """生成笔迹网格"""
    if len(points) < 2:
        return []
    
    vertices = []
    indices = []
    
    for i in range(len(points)):
        p = points[i]
        width = get_width(p)
        
        # 计算切线方向
        if i == 0:
            # 第一个点：用 p0->p1 方向
            tangent = normalize(points[1] - points[0])
        elif i == len(points) - 1:
            # 最后一个点：用 p(n-2)->p(n-1) 方向
            tangent = normalize(points[-1] - points[-2])
        else:
            # 中间点：用前后两段的平均方向
            t1 = normalize(p - points[i-1])
            t2 = normalize(points[i+1] - p)
            tangent = normalize(t1 + t2)
        
        # 法线方向（垂直于切线）
        normal = Point(-tangent.y, tangent.x)
        
        # 左右轮廓点
        half_width = width / 2
        left = Point(p.x - normal.x * half_width, p.y - normal.y * half_width)
        right = Point(p.x + normal.x * half_width, p.y + normal.y * half_width)
        
        # 添加顶点
        vertices.append(Vertex(left.x, left.y, p.u, 0.0))  # u: 纹理坐标
        vertices.append(Vertex(right.x, right.y, p.u, 1.0))
        
        # 生成索引（三角带）
        if i > 0:
            base = i * 2
            indices.extend([base - 2, base - 1, base, base + 1])
    
    return vertices, indices

def normalize(v):
    """向量归一化"""
    length = math.sqrt(v.x*v.x + v.y*v.y)
    if length == 0:
        return Point(0, 0)
    return Point(v.x / length, v.y / length)
```

**优点**：
- 连续平滑的笔迹
- 支持复杂笔刷效果（纹理、渐变）
- GPU 友好（单次绘制调用）
- 可实现高级效果（笔锋、毛边）

**缺点**：
- 实现复杂
- 需要处理拐角、自交
- 计算量较大

**适用场景**：
- 高质量笔迹渲染
- 专业绘图应用
- 需要复杂笔刷效果

> 来源：[手写笔迹还原算法](https://blog.csdn.net/luansxx/article/details/120960073)

#### 2.5.3 方案对比表

| 特性 | 位图盖章 | 网格生成 |
|-----|---------|---------|
| 实现复杂度 | ★☆☆☆☆ | ★★★★☆ |
| 渲染质量 | ★★★☆☆ | ★★★★★ |
| GPU 效率 | ★★☆☆☆ | ★★★★★ |
| 采样率要求 | 高（>100Hz） | 中（>30Hz） |
| 支持复杂笔刷 | 否 | 是 |
| 内存占用 | 中 | 低 |
| 适用场景 | 简单笔迹、快速原型 | 专业应用、高质量渲染 |

### 2.6 GPU 渲染优化

#### 2.6.1 顶点着色器

```glsl
// 顶点着色器
#version 300 es
layout(location = 0) in vec2 a_position;
layout(location = 1) in vec2 a_texCoord;

uniform mat4 u_mvpMatrix;

out vec2 v_texCoord;

void main() {
    gl_Position = u_mvpMatrix * vec4(a_position, 0.0, 1.0);
    v_texCoord = a_texCoord;
}
```

#### 2.6.2 片元着色器（带纹理）

```glsl
// 片元着色器
#version 300 es
precision mediump float;

in vec2 v_texCoord;

uniform sampler2D u_texture;
uniform vec4 u_color;

out vec4 fragColor;

void main() {
    vec4 texColor = texture(u_texture, v_texCoord);
    fragColor = texColor * u_color;
}
```

#### 2.6.3 抗锯齿

**Alpha 混合抗锯齿**：
```glsl
// 片元着色器
void main() {
    float dist = length(v_texCoord - vec2(0.5));
    float alpha = 1.0 - smoothstep(0.45, 0.5, dist);
    fragColor = vec4(u_color.rgb, u_color.a * alpha);
}
```

---

## 三、现有开源/商用方案

### 3.1 Android 官方方案

#### 3.1.1 stylus-samples（Low-Latency Ink）

**仓库**：[android/large-screen-codelabs](https://github.com/android/large-screen-codelabs)

**特点**：
- 展示低延迟手写实现
- 使用 OpenGL + SurfaceView
- 集成运动预测库
- 支持压感、倾斜度

**技术栈**：
- Kotlin
- OpenGL ES
- Jetpack Compose
- androidx.input:input-motionprediction

#### 3.1.2 Google Ink API（androidx.ink）

**发布时间**：2025 年 12 月 1.0.0 稳定版

**核心优势**：
- 官方维护，长期支持
- 4ms 级延迟
- 完整的笔刷系统
- 服务端渲染支持（Linux x86_64）

**模块**：
- `ink-strokes`：数据结构
- `ink-brush`：画笔定义
- `ink-rendering`：渲染
- `ink-authoring`：实时创作
- `ink-storage`：持久化

> 来源：[Android Ink API](https://developer.android.google.cn/jetpack/androidx/releases/ink)

### 3.2 iOS 方案

#### 3.2.1 PencilKit

**特点**：
- Apple 官方框架
- 最低延迟（8-9ms）
- 内置工具调色板
- 支持手写识别

**限制**：
- 仅 iOS/macOS
- 定制性有限
- 无法完全控制渲染管线

> 来源：[PencilKit Documentation](https://developer.apple.com/documentation/pencilkit/)

### 3.3 Web 端方案

#### 3.3.1 perfect-freehand

**仓库**：[steveruizok/perfect-freehand](https://github.com/steveruizok/perfect-freehand)

**特点**：
- 纯 JavaScript/TypeScript
- 压感模拟（基于速度）
- 生成轮廓点（非直接渲染）
- 支持 SVG/Canvas 渲染

**核心 API**：
```typescript
import { getStroke } from 'perfect-freehand'

const points = [[0, 0], [10, 5], [20, 8]]
const stroke = getStroke(points, {
    size: 16,
    thinning: 0.5,
    smoothing: 0.5,
    streamline: 0.5
})

// stroke 是轮廓点数组，可用于渲染 SVG 或 Canvas
```

**优点**：
- 轻量级
- 跨平台（Web）
- 易于集成
- 效果自然

**缺点**：
- 仅 Web
- 无 GPU 加速
- 长笔画可能卡顿

**应用**：
- tldraw
- Excalidraw（部分功能）

> 来源：[perfect-freehand GitHub](https://github.com/steveruizok/perfect-freehand)

#### 3.3.2 Excalidraw

**仓库**：[excalidraw/excalidraw](https://github.com/excalidraw/excalidraw)

**技术栈**：
- React
- Canvas 2D / SVG
- rough.js（手绘风格）
- perfect-freehand（自由绘制）

**渲染特点**：
- 手绘风格（rough.js）
- 随机抖动模拟
- 贝塞尔曲线平滑
- 低饱和度配色

> 来源：[Excalidraw GitHub](https://github.com/excalidraw/excalidraw)

#### 3.3.3 tldraw

**仓库**：[tldraw/tldraw](https://github.com/tldraw/tldraw)

**技术栈**：
- React
- Canvas 2D
- perfect-freehand

**特点**：
- 无限画布
- 实时协作
- 丰富的形状工具
- 自由绘制

> 来源：[tldraw GitHub](https://github.com/tldraw/tldraw)

### 3.4 Rust 生态方案

#### 3.4.1 Rnote

**仓库**：[flxzt/rnote](https://github.com/flxzt/rnote)

**技术栈**：
- Rust
- GTK4
- Piet（2D 图形库）
- 矢量绘图引擎

**特点**：
- 矢量优先架构
- 分块渲染机制
- 增量渲染
- 支持 e-ink 显示

**渲染架构**：
```
crates/rnote-engine/src/engine/rendering.rs
├── 分块渲染（viewport-based）
├── LOD（Level of Detail）
└── 增量更新
```

**性能优化**：
- 仅渲染可见区域
- 根据缩放级别调整精度
- 多线程渲染

> 来源：[Rnote GitHub](https://github.com/flxzt/rnote)

#### 3.4.2 Xournal++

**仓库**：[xournalpp/xournalpp](https://github.com/xournalpp/xournalpp)

**技术栈**：
- C++
- GTK3
- Cairo（2D 图形库）

**特点**：
- 成熟的开源笔记应用
- PDF 批注支持
- 跨平台（Linux/macOS/Windows）
- 支持 Wacom 数位板

**渲染架构**：
```
src/core/gui/
├── StrokeView.cpp（笔迹绘制）
├── DocumentView.cpp（页面渲染）
└── src/util/GtkUtil.cpp（GTK 适配）
```

**渲染流程**：
1. 数据层：`Stroke.cpp` 定义笔迹数据结构
2. 视图层：`StrokeView.cpp` 实现绘制算法
3. 控制层：`Control.cpp` 协调渲染参数

> 来源：[Xournal++ GitHub](https://github.com/xournalpp/xournalpp)

#### 3.4.3 Vello

**仓库**：[linebender/vello](https://github.com/linebender/vello)

**技术栈**：
- Rust
- wgpu（GPU 抽象）
- GPU 计算着色器

**特点**：
- 完全 GPU 驱动的 2D 渲染器
- 基于计算着色器（Compute Shader）
- 前缀和算法并行化
- 分块渲染（Tiling）

**架构**：
```
场景编码层（CPU）
    ↓
GPU 计算管线（13 阶段）
    ├─ 路径解析
    ├─ 图元处理
    ├─ 空间变换
    └─ 光栅化
    ↓
渲染输出
```

**性能**：
- M1 Max 上 paris-30k 场景：177 fps（1600x1600）
- 10 万路径处理：9ms（传统方案 38ms）
- 4K 屏渲染延迟：8ms（传统方案 25ms）

**状态**：Alpha，仍在开发中

**应用**：
- Xilem（Rust GUI 工具包）
- Servo（画布渲染后端）

> 来源：[Vello GitHub](https://github.com/linebender/vello)

#### 3.4.4 wgpu

**仓库**：[gfx-rs/wgpu](https://github.com/gfx-rs/wgpu)

**特点**：
- 纯 Rust 实现的图形 API
- 跨平台（Vulkan/Metal/DX12/WebGPU）
- 内存安全
- WebAssembly 支持

**架构**：
```
Rust 应用
    ↓
wgpu API
    ├─ Vulkan（Android/Linux/Windows）
    ├─ Metal（iOS/macOS）
    ├─ DX12（Windows）
    └─ WebGPU/WebGL2（Web）
    ↓
GPU 硬件
```

**优势**：
- 统一 API
- 跨平台一致性
- 现代图形特性
- 计算着色器支持

> 来源：[wgpu GitHub](https://github.com/gfx-rs/wgpu)

### 3.5 Flutter 生态方案

#### 3.5.1 Saber

**仓库**：[saber-notes/saber](https://github.com/saber-notes/saber)

**技术栈**：
- Flutter
- Skia/Impeller（渲染后端）
- perfect-freehand（笔迹生成）

**特点**：
- 跨平台（Android/iOS/Windows/macOS/Linux）
- 智能荧光笔（不叠加颜色）
- PDF 批注
- 跨设备同步

**渲染架构**：
```
应用层（UI/UX）
├── 画布组件系统
├── 工具管理系统
└── 数据同步层
服务层
├── 文件管理模块
├── 加密安全模块
└── Nextcloud 集成
```

**核心组件**：
- `CustomPainter`：自定义绘制
- 矢量笔迹实时绘制
- 多层画布结构

> 来源：[Saber GitHub](https://github.com/saber-notes/saber)

---

## 四、渲染后端选型对比

### 4.1 方案概览

| 方案 | 语言 | 跨平台 | GPU 后端 | 状态 | 适用场景 |
|-----|------|-------|---------|------|---------|
| **wgpu** | Rust | ★★★★★ | Vulkan/Metal/DX12/WebGPU | 稳定 | 跨平台应用 |
| **Skia (Ganesh)** | C++ | ★★★★★ | OpenGL/Metal/Vulkan | 稳定 | 通用 2D 渲染 |
| **Skia (Graphite)** | C++ | ★★★★☆ | Vulkan/Metal | 实验性 | 现代 GPU |
| **Impeller** | C++ | ★★★☆☆ | Metal/Vulkan | 稳定（Flutter） | Flutter 应用 |
| **Vello** | Rust | ★★★★☆ | wgpu（计算着色器） | Alpha | 高性能 2D |
| **原生 API** | 各平台 | ★★☆☆☆ | 平台原生 | 稳定 | 极致性能 |

### 4.2 详细对比

#### 4.2.1 wgpu

**优势**：
- ✅ 纯 Rust，内存安全
- ✅ 真正的跨平台（Android/iOS/Windows/macOS/Linux/Web）
- ✅ 统一 API，一次编写多处运行
- ✅ 现代图形特性（计算着色器、Bindless）
- ✅ WebAssembly 支持
- ✅ 活跃社区，持续更新

**劣势**：
- ❌ 2D 渲染需要自建管线
- ❌ 文档相对较少
- ❌ 需要图形编程知识

**包体积**：
- 核心库：~2-5 MB
- 着色器编译：运行时编译（可预编译）

**嵌入能力**：
- 可嵌入任何 Rust 应用
- 支持 winit（窗口管理）
- 可集成到现有 UI 框架

**性能**：
- 接近原生 API
- 计算着色器性能优异
- 适合大规模并行任务

> 来源：[wgpu GitHub](https://github.com/gfx-rs/wgpu)

#### 4.2.2 Skia (Ganesh)

**优势**：
- ✅ 成熟的 2D 图形库
- ✅ 完整的 2D 渲染功能
- ✅ 文本渲染优秀
- ✅ 广泛的平台支持
- ✅ 丰富的文档和示例

**劣势**：
- ❌ C++ 实现，内存安全风险
- ❌ 运行时着色器编译（首帧卡顿）
- ❌ 包体积大（~10-20 MB）
- ❌ 性能不如现代方案

**包体积**：
- 核心库：~10-20 MB
- 字体渲染：额外 ~2-5 MB

**嵌入能力**：
- 可嵌入 C++/Rust/Python 等
- Flutter 使用 Skia（已迁移到 Impeller）
- Compose Multiplatform 使用 Skia

**性能**：
- 成熟稳定
- 首帧着色器编译导致卡顿
- P99 帧耗时较高

> 来源：[Skia 官网](https://skia.org/)

#### 4.2.3 Impeller

**优势**：
- ✅ 预编译着色器（零运行时编译）
- ✅ 消除首帧卡顿
- ✅ 针对 Flutter 优化
- ✅ 更好的帧率稳定性

**劣势**：
- ❌ 仅为 Flutter 设计
- ❌ 无法独立使用
- ❌ 平台支持有限（iOS/Android）
- ❌ GPU 内存占用略高

**包体积**：
- 预编译着色器：额外 ~2-5 MB
- 总体与 Skia 相当

**嵌入能力**：
- 仅 Flutter 框架
- 无法嵌入其他应用

**性能**：
- 首帧渲染时间减少 60%+
- 复杂动画 P99 帧耗时降低 50%
- 长列表滚动掉帧率降低 75%

**实测数据**（120Hz）：
| 指标 | Skia | Impeller | 提升 |
|-----|------|----------|------|
| 首帧 Shader 编译 | 40-200ms | 0ms | 消除 |
| P99 帧耗时 | 22ms | 11ms | 50% |
| 掉帧率 | 3.2% | 0.8% | 75% |
| GPU 内存峰值 | 基准 | +8-15% | 略增 |

> 来源：[Flutter Impeller 性能测试](https://juejin.cn/post/7530854307195404340)

#### 4.2.4 Vello

**优势**：
- ✅ 完全 GPU 驱动
- ✅ 计算着色器并行化
- ✅ 性能优异（177 fps @ 4K）
- ✅ Rust 实现，内存安全
- ✅ 与 wgpu 生态集成

**劣势**：
- ❌ Alpha 状态，不稳定
- ❌ 需要 GPU 支持计算着色器
- ❌ 无软件回退
- ❌ 文档不完善

**包体积**：
- 核心库：~3-8 MB
- 着色器：预编译

**嵌入能力**：
- 可嵌入 Rust 应用
- Xilem GUI 框架使用
- Servo 浏览器使用

**性能**：
- 10 万路径处理：9ms（传统 38ms）
- 4K 渲染延迟：8ms（传统 25ms）
- CPU 占用：<5%（传统满载）

**限制**：
- 需要 GPU 支持计算着色器
- 无软件回退
- 模糊、过滤效果未完善

> 来源：[Vello GitHub](https://github.com/linebender/vello)

#### 4.2.5 原生 API（Vulkan/Metal/OpenGL ES）

**优势**：
- ✅ 极致性能
- ✅ 完全控制
- ✅ 无额外依赖
- ✅ 包体积最小

**劣势**：
- ❌ 需要为每个平台编写代码
- ❌ 开发成本高
- ❌ 维护困难
- ❌ 需要图形编程专家

**包体积**：
- 无额外依赖
- 仅应用代码

**嵌入能力**：
- 完全自定义
- 可集成到任何框架

**性能**：
- 理论最优
- 无抽象层开销
- 可针对硬件优化

### 4.3 方案对比表

| 维度 | wgpu | Skia | Impeller | Vello | 原生 API |
|-----|------|------|----------|-------|---------|
| **跨平台一致性** | ★★★★★ | ★★★★☆ | ★★★☆☆ | ★★★★☆ | ★★☆☆☆ |
| **性能** | ★★★★☆ | ★★★☆☆ | ★★★★★ | ★★★★★ | ★★★★★ |
| **包体积** | ★★★★☆ | ★★☆☆☆ | ★★★☆☆ | ★★★★☆ | ★★★★★ |
| **嵌入能力** | ★★★★☆ | ★★★★★ | ★☆☆☆☆ | ★★★☆☆ | ★★★★★ |
| **开发成本** | ★★★☆☆ | ★★★★★ | ★★★★★ | ★★☆☆☆ | ★☆☆☆☆ |
| **社区生态** | ★★★★☆ | ★★★★★ | ★★★☆☆ | ★★★☆☆ | ★★★★★ |
| **稳定性** | ★★★★☆ | ★★★★★ | ★★★★☆ | ★★☆☆☆ | ★★★★★ |

### 4.4 推荐方案

#### 场景 1：Flutter 应用

**推荐**：Impeller（默认）

**理由**：
- Flutter 官方支持
- 性能最优
- 零配置
- 长期维护

**备选**：Skia（兼容旧版本）

#### 场景 2：Rust 应用

**推荐**：wgpu + 自建 2D 管线

**理由**：
- 真正的跨平台
- 性能优异
- 灵活可控
- 与 Vello 生态兼容

**备选**：Vello（如果接受 Alpha 状态）

#### 场景 3：Web 应用

**推荐**：Canvas 2D + perfect-freehand

**理由**：
- 简单易用
- 广泛支持
- 效果自然

**备选**：WebGL + wgpu（需要高性能）

#### 场景 4：跨平台原生应用

**推荐**：wgpu

**理由**：
- 统一 API
- 跨平台一致性
- 性能接近原生
- 可嵌入任何框架

**备选**：Skia（需要完整 2D 功能）

---

## 五、压感/倾斜/速度数据处理标准

### 5.1 W3C Pointer Events Level 3

**规范**：[Pointer Events Level 3](https://htmlspecs.com/pointerevents3/)

**核心属性**：

| 属性 | 类型 | 范围 | 说明 |
|-----|------|------|------|
| `pressure` | float | 0.0-1.0 | 归一化压力值 |
| `tangentialPressure` | float | -1.0-1.0 | 切向压力（桶压） |
| `tiltX` | long | -90°-90° | X 轴倾斜角度 |
| `tiltY` | long | -90°-90° | Y 轴倾斜角度 |
| `twist` | long | 0°-359° | 笔身旋转角度 |
| `altitudeAngle` | double | 0-π/2 | 高度角（与 Z 轴夹角） |
| `azimuthAngle` | double | 0-2π | 方位角（与 X 轴夹角） |
| `width` | double | CSS 像素 | 接触区域宽度 |
| `height` | double | CSS 像素 | 接触区域高度 |

**角度系统**：

```
        Z 轴（垂直于屏幕）
        ↑
        |  altitudeAngle
        | /
        |/
        +--------→ X 轴
       / 
      / azimuthAngle
     /
    Y 轴（屏幕向下）
```

** tiltX/tiltY vs altitude/azimuth **：
- `tiltX/tiltY`：笛卡尔坐标系，直观但不够精确
- `altitude/azimuth`：球坐标系，数学上更优雅

**转换公式**：
```javascript
// tiltX/tiltY → altitude/azimuth
function tiltToSpherical(tiltX, tiltY) {
    const tiltXRad = tiltX * Math.PI / 180;
    const tiltYRad = tiltY * Math.PI / 180;
    
    const altitude = Math.acos(
        Math.cos(tiltXRad) * Math.cos(tiltYRad)
    );
    
    const azimuth = Math.atan2(
        Math.sin(tiltXRad),
        Math.sin(tiltYRad)
    );
    
    return { altitude, azimuth };
}
```

### 5.2 各平台差异

#### 5.2.1 Android

**API**：`MotionEvent`

**可用属性**：
- `getPressure()`：压力值（0.0-1.0）
- `getAxisValue(AXIS_TILT)`：倾斜度
- `getAxisValue(AXIS_ORIENTATION)`：方向角
- `getToolType()`：工具类型（手指/笔/橡皮）

**特点**：
- 使用 `AXIS_TILT` + `AXIS_ORIENTATION`
- 无 `altitudeAngle/azimuthAngle`
- 需要手动转换

**代码示例**：
```kotlin
val pressure = event.pressure
val tilt = event.getAxisValue(MotionEvent.AXIS_TILT)
val orientation = event.getAxisValue(MotionEvent.AXIS_ORIENTATION)
val toolType = event.getToolType(0)
```

> 来源：[Android MotionEvent](https://developer.android.com/reference/android/view/MotionEvent)

#### 5.2.2 iOS

**API**：`UITouch`

**可用属性**：
- `force`：压力值（0.0-maximumPossibleForce）
- `altitudeAngle`：高度角
- `azimuthAngle(in:)`：方位角
- `azimuthUnitVector(in:)`：方位单位向量
- `type`：触摸类型（直接/间接/笔）

**特点**：
- 使用球坐标系（altitude/azimuth）
- Apple Pencil 专属 API
- 精度最高

**代码示例**：
```swift
let pressure = touch.force / touch.maximumPossibleForce
let altitude = touch.altitudeAngle
let azimuth = touch.azimuthAngle(in: view)
```

> 来源：[UITouch Documentation](https://developer.apple.com/documentation/uikit/uitouch)

#### 5.2.3 Web

**API**：`PointerEvent`

**可用属性**：
- `pressure`：压力值（0.0-1.0）
- `tiltX`：X 轴倾斜（-90°-90°）
- `tiltY`：Y 轴倾斜（-90°-90°）
- `twist`：旋转角度（0°-359°）
- `altitudeAngle`：高度角（Pointer Events Level 3）
- `azimuthAngle`：方位角（Pointer Events Level 3）

**浏览器支持**：
| 属性 | Chrome | Firefox | Safari | Edge |
|-----|--------|---------|--------|------|
| `pressure` | ✅ | ✅ | ✅ | ✅ |
| `tiltX/Y` | ✅ | ✅ | ✅ | ✅ |
| `altitudeAngle` | ✅ | ❌ | ❌ | ✅ |
| `azimuthAngle` | ✅ | ❌ | ❌ | ✅ |

**特点**：
- Level 2：仅支持 `tiltX/Y`
- Level 3：支持 `altitude/azimuth`（2026 年推进为推荐标准）
- iOS Safari 不暴露 `pressure`（需用速度模拟）

**代码示例**：
```javascript
canvas.addEventListener('pointermove', (event) => {
    if (event.pointerType === 'pen') {
        const pressure = event.pressure;
        const tiltX = event.tiltX;
        const tiltY = event.tiltY;
        
        // Level 3（如果支持）
        if ('altitudeAngle' in event) {
            const altitude = event.altitudeAngle;
            const azimuth = event.azimuthAngle;
        }
    }
});
```

> 来源：[PointerEvent MDN](https://developer.mozilla.org/ja/docs/Web/API/PointerEvent)

### 5.3 跨平台数据标准化

**建议方案**：统一使用球坐标系（altitude/azimuth）

**标准化结构**：
```rust
#[derive(Debug, Clone, Copy)]
pub struct StylusData {
    /// 归一化压力值 [0.0, 1.0]
    pub pressure: f32,
    /// 高度角（与 Z 轴夹角）[0, π/2]
    pub altitude: f32,
    /// 方位角（与 X 轴夹角）[0, 2π]
    pub azimuth: f32,
    /// 笔身旋转角度 [0, 2π]
    pub twist: f32,
    /// 接触区域宽度（CSS 像素）
    pub width: f32,
    /// 接触区域高度（CSS 像素）
    pub height: f32,
}

impl StylusData {
    /// 从 Android MotionEvent 转换
    pub fn from_android(pressure: f32, tilt: f32, orientation: f32) -> Self {
        // tilt + orientation → altitude + azimuth
        let tilt_rad = tilt;
        let altitude = (std::f32::consts::FRAC_PI_2 - tilt_rad).abs();
        let azimuth = orientation;
        
        Self {
            pressure,
            altitude,
            azimuth,
            twist: 0.0, // Android 不提供
            width: 0.0,
            height: 0.0,
        }
    }
    
    /// 从 iOS UITouch 转换
    pub fn from_ios(pressure: f32, altitude: f32, azimuth: f32) -> Self {
        Self {
            pressure,
            altitude,
            azimuth,
            twist: 0.0, // iOS 不提供
            width: 0.0,
            height: 0.0,
        }
    }
    
    /// 从 Web PointerEvent 转换
    pub fn from_web(pressure: f32, tilt_x: f32, tilt_y: f32) -> Self {
        // tiltX/tiltY → altitude/azimuth
        let tilt_x_rad = tilt_x.to_radians();
        let tilt_y_rad = tilt_y.to_radians();
        
        let altitude = (tilt_x_rad.cos() * tilt_y_rad.cos()).acos();
        let azimuth = tilt_x_rad.sin().atan2(tilt_y_rad.sin());
        
        Self {
            pressure,
            altitude,
            azimuth,
            twist: 0.0,
            width: 0.0,
            height: 0.0,
        }
    }
}
```

### 5.4 压感曲线校准

**问题**：不同设备的压感曲线不同

**解决方案**：用户可配置的压感曲线

**实现**：
```rust
pub struct PressureCurve {
    /// 控制点 [(x, y), ...]
    points: Vec<(f32, f32)>,
}

impl PressureCurve {
    pub fn linear() -> Self {
        Self {
            points: vec![(0.0, 0.0), (1.0, 1.0)],
        }
    }
    
    pub fn ease_in() -> Self {
        Self {
            points: vec![(0.0, 0.0), (0.5, 0.2), (1.0, 1.0)],
        }
    }
    
    pub fn ease_out() -> Self {
        Self {
            points: vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)],
        }
    }
    
    /// 应用曲线
    pub fn apply(&self, pressure: f32) -> f32 {
        // 线性插值
        for i in 0..self.points.len() - 1 {
            let (x0, y0) = self.points[i];
            let (x1, y1) = self.points[i + 1];
            
            if pressure >= x0 && pressure <= x1 {
                let t = (pressure - x0) / (x1 - x0);
                return y0 + t * (y1 - y0);
            }
        }
        
        pressure
    }
}
```

---

## 六、推荐结论

### 6.1 技术选型建议

#### 推荐方案：wgpu + 自建笔迹渲染管线

**理由**：
1. **真正的跨平台**：Android/iOS/Windows/macOS/Linux/Web 统一 API
2. **性能优异**：接近原生，支持计算着色器
3. **灵活可控**：可针对笔迹渲染优化
4. **未来兼容**：与 Vello 生态兼容，可逐步迁移
5. **Rust 生态**：内存安全，现代语言特性

**架构建议**：
```
┌─────────────────────────────────────────────────────┐
│              跨平台笔记应用                            │
└─────────────────────────────────────────────────────┘
                      ↓
┌─────────────────────────────────────────────────────┐
│              笔迹渲染引擎（Rust）                      │
├─────────────────────────────────────────────────────┤
│  输入处理层                                          │
│  ├─ Android: MotionEvent → StylusData               │
│  ├─ iOS: UITouch → StylusData                       │
│  ├─ Web: PointerEvent → StylusData                  │
│  └─ Desktop: 平台原生 → StylusData                  │
├─────────────────────────────────────────────────────┤
│  笔迹处理层                                          │
│  ├─ 点预处理（去噪、重采样）                           │
│  ├─ 平滑插值（One Euro Filter / Catmull-Rom）        │
│  ├─ 压感映射（pressure/velocity → width）            │
│  └─ 轮廓生成（Triangle Strip）                       │
├─────────────────────────────────────────────────────┤
│  渲染层（wgpu）                                      │
│  ├─ 顶点缓冲区（顶点 + 索引）                         │
│  ├─ 顶点着色器（变换、投影）                           │
│  ├─ 片元着色器（颜色、纹理、抗锯齿）                    │
│  └─ 计算着色器（可选，用于大规模并行）                   │
└─────────────────────────────────────────────────────┘
                      ↓
┌─────────────────────────────────────────────────────┐
│              UI 框架集成                               │
├─────────────────────────────────────────────────────┤
│  ├─ Android: Jetpack Compose + Platform View        │
│  ├─ iOS: SwiftUI + UIViewRepresentable              │
│  ├─ Desktop: 原生窗口 + wgpu surface                │
│  └─ Web: Canvas + WebAssembly                       │
└─────────────────────────────────────────────────────┘
```

### 6.2 关键实现要点

#### 6.2.1 低延迟渲染

**Android**：
- 使用 `CanvasFrontBufferedRenderer` 或 `GLFrontBufferedRenderer`
- 集成 `MotionEventPredictor`
- 提取历史点（`getHistorical*()`）

**iOS**：
- 使用 PencilKit 或 Metal 直接渲染
- 使用预测触碰（iOS 13+）
- 避免透明 Metal 图层

**Web**：
- 使用 `requestAnimationFrame` 驱动渲染
- 使用 `getCoalescedEvents()` 获取合并点
- 实现客户端预测

#### 6.2.2 笔迹质量

**平滑算法**：
- 使用 One Euro Filter 去噪
- 使用 Catmull-Rom 或二次贝塞尔平滑
- 按距离重采样（2-4 像素间隔）

**压感映射**：
- 提供用户可配置的压感曲线
- 支持速度映射（无压感设备）
- 支持 Sigmoid 映射（更自然）

**轮廓生成**：
- 使用 Triangle Strip 方案
- 处理拐角（miter/bevel/round）
- 处理自交（可选）

#### 6.2.3 性能优化

**GPU 优化**：
- 批量提交顶点数据
- 使用实例化渲染（多笔迹）
- 预编译着色器

**CPU 优化**：
- 避免对象创建（使用对象池）
- 使用 SIMD 指令（可选）
- 多线程处理（输入/渲染分离）

**内存优化**：
- 仅渲染可见区域
- 使用 LOD（Level of Detail）
- 定期清理旧笔迹

### 6.3 开发路线图

#### Phase 1：基础功能（2-3 个月）
- [ ] wgpu 初始化与跨平台 surface
- [ ] 基础笔迹渲染（Triangle Strip）
- [ ] 压感支持
- [ ] 简单平滑（二次贝塞尔）

#### Phase 2：质量提升（2-3 个月）
- [ ] One Euro Filter
- [ ] Catmull-Rom 平滑
- [ ] 压感曲线校准
- [ ] 抗锯齿

#### Phase 3：性能优化（1-2 个月）
- [ ] 批量渲染
- [ ] 实例化渲染
- [ ] 视口裁剪
- [ ] LOD

#### Phase 4：高级功能（2-3 个月）
- [ ] 倾斜支持
- [ ] 复杂笔刷（纹理、渐变）
- [ ] 橡皮擦（精确擦除）
- [ ] 手写识别集成

### 6.4 风险评估

| 风险 | 概率 | 影响 | 缓解措施 |
|-----|------|------|---------|
| wgpu 在某些设备上不支持 | 中 | 高 | 提供 Skia 回退方案 |
| 笔迹质量不达标 | 低 | 高 | 参考 Google Ink API 实现 |
| 跨平台一致性问题 | 中 | 中 | 统一测试套件 |
| 性能不足 | 低 | 中 | 使用计算着色器优化 |
| Web 端性能差 | 中 | 中 | 降级为 Canvas 2D |

### 6.5 替代方案

如果 wgpu 方案风险过高，可考虑：

**备选 1：Flutter + Impeller**
- 优势：成熟、性能优、官方支持
- 劣势：仅限 Flutter 生态

**备选 2：Skia**
- 优势：成熟、功能完整
- 劣势：包体积大、首帧卡顿

**备选 3：Vello（等待稳定）**
- 优势：性能优异、Rust 生态
- 劣势：Alpha 状态、不稳定

---

## 七、参考资料

### 7.1 官方文档

- [Android Advanced Stylus Features](https://developer.android.com/develop/ui/compose/touch-input/stylus-input/advanced-stylus-features)
- [Android Ink API](https://developer.android.google.cn/jetpack/androidx/releases/ink)
- [iOS PencilKit](https://developer.apple.com/documentation/pencilkit/)
- [W3C Pointer Events Level 3](https://htmlspecs.com/pointerevents3/)
- [wgpu Documentation](https://docs.rs/wgpu/)
- [Vello GitHub](https://github.com/linebender/vello)

### 7.2 开源项目

- [Rnote](https://github.com/flxzt/rnote) - Rust 手写笔记应用
- [Xournal++](https://github.com/xournalpp/xournalpp) - C++ 手写笔记应用
- [Saber](https://github.com/saber-notes/saber) - Flutter 手写笔记应用
- [perfect-freehand](https://github.com/steveruizok/perfect-freehand) - Web 笔迹生成库
- [Excalidraw](https://github.com/excalidraw/excalidraw) - 手绘风格白板
- [tldraw](https://github.com/tldraw/tldraw) - 无限画布白板

### 7.3 技术文章

- [Android 手写渲染技术演进](https://juejin.cn/post/7642626890507796531)
- [Flutter Impeller 性能测试](https://juejin.cn/post/7530854307195404340)
- [One Euro Filter 实现](https://blog.csdn.net/caokaifa/article/details/155966079)
- [手写笔迹还原算法](https://blog.csdn.net/luansxx/article/details/120960073)
- [触摸延迟系统级优化](https://wenku.csdn.net/column/5dkv9sivw7)

---

## 八、附录

### 8.1 术语表

| 术语 | 说明 |
|-----|------|
| **Front-Buffered Rendering** | 前置缓冲渲染，允许局部内容提前进入显示链路 |
| **MotionEvent** | Android 触摸事件对象 |
| **Triangle Strip** | 三角带，一种高效的网格表示方式 |
| **One Euro Filter** | 自适应低通滤波器 |
| **Catmull-Rom** | C1 连续的样条曲线 |
| **altitude/azimuth** | 球坐标系中的高度角和方位角 |
| **LOD** | Level of Detail，根据距离调整细节级别 |
| **Bindless** | 无绑定渲染，现代 GPU 特性 |

### 8.2 性能基准

| 场景 | 目标帧率 | 帧预算 | 关键指标 |
|-----|---------|-------|---------|
| 60Hz 书写 | 60 fps | 16.67ms | 端到端延迟 <40ms |
| 120Hz 书写 | 120 fps | 8.33ms | 端到端延迟 <28ms |
| 1000 点笔迹 | 60 fps | 16.67ms | 渲染时间 <5ms |
| 10000 点笔迹 | 60 fps | 16.67ms | 渲染时间 <10ms |

### 8.3 检查清单

**低延迟**：
- [ ] 使用前置缓冲渲染
- [ ] 集成运动预测
- [ ] 提取历史点
- [ ] 避免 VSync 等待

**笔迹质量**：
- [ ] 使用 One Euro Filter 去噪
- [ ] 使用 Catmull-Rom 平滑
- [ ] 提供压感曲线校准
- [ ] 实现抗锯齿

**跨平台**：
- [ ] 统一 StylusData 结构
- [ ] 处理平台差异
- [ ] 统一测试套件
- [ ] 性能基准测试

**性能**：
- [ ] 批量提交顶点数据
- [ ] 使用实例化渲染
- [ ] 视口裁剪
- [ ] 预编译着色器

---

**文档版本**：v1.0  
**最后更新**：2026-10-12  
**作者**：技术调研团队
