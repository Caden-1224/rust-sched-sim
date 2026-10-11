//! 本模块定义「任务」这个概念：一台机器上排队等执行的一个工作单元。

// 【知识点：模块与包管理】std 里按功能分模块，用到的名字要显式引进来
use std::rc::Rc; // 引用计数指针，用在任务名的共享上

// 【知识点：结构与枚举】TaskId 是一个「新类型」：给 usize 起一个带含义的名字
// 【知识点：derive 与 trait 派生】Copy 表示按位复制即安全；Hash、Eq 是放进
// HashSet / HashMap 当下标所需的两个能力
// 【知识点：变量与类型】结构体变体在内存里就是这一个 usize，没有额外开销
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub usize);

// 【知识点：Trait】为一个类型实现 Display 之后，它就能用 `{}` 印出来
impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 【知识点：所有权与借用】self 是共享借用，读它的字段不复制任何东西
        write!(f, "T{}", self.0)
    }
}

// 【知识点：枚举】任务的两种类型：CPU 侧的计算任务与 GPU 侧的内核任务，
// 每个变体各带自己的数据——这是「同一概念的不同形态」的标准建模方式
// 【知识点：derive 与 trait 派生】PartialOrd 与 Ord 让枚举值可以比大小，
// 比较顺序就是变体的书写顺序：Cpu 小于 Gpu
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TaskKind {
    /// CPU 计算任务，附带它在该设备上占用的资源份额。
    Cpu(u8),
    /// GPU 内核任务，附带它在该设备上占用的资源份额。
    Gpu(u8),
}

impl TaskKind {
    /// 资源份额（1..=4）：占用越大，同一台设备上能同时跑的任务越少。
    // 【知识点：方法】&self 借用自身，返回一个 Copy 的 u8，不涉及所有权转移
    pub fn weight(self) -> u8 {
        // 【知识点：match 与模式匹配】两个变体都用同一个名字绑定内部的值
        match self {
            TaskKind::Cpu(w) | TaskKind::Gpu(w) => w,
        }
    }

    /// 设备名，用于日志与错误消息。
    pub fn device(self) -> &'static str {
        match self {
            TaskKind::Cpu(_) => "CPU",
            TaskKind::Gpu(_) => "GPU",
        }
    }
}

// 【知识点：枚举】任务在生命周期里的几种状态；这里的变体不带数据
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// 还在排队。
    Ready,
    /// 被分配了时间片、正在推进。
    Running,
    /// 工作量已经做完。
    Finished,
}

/// 调度器管理的一个任务：编号、名字、类型、状态、优先级、工作量。
// 【知识点：结构体】结构体把一组相关字段打包成一个类型，字段各自有类型
#[derive(Debug, Clone)]
pub struct Task {
    /// 唯一编号。
    pub id: TaskId,
    /// 任务名。
    // 【知识点：智能指针】Rc<str> 是「共享的、不可变的字符串」，
    // 克隆一次只让引用计数加一，字符串本身不复制
    pub name: Rc<str>,
    /// 任务类型。
    // 【知识点：枚举】字段的类型可以是一个枚举，取值只能是列出的形态之一
    pub kind: TaskKind,
    /// 当前状态。
    pub state: TaskState,
    /// 优先级分数，数值大的先被调度。
    pub priority: u32,
    /// 估计要花的设备时间单位总数。
    pub estimate: u32,
    /// 已经花掉的时间单位数。
    pub done: u32,
}

// 【知识点：实现块】impl 块把方法与类型绑在一起；一个类型可以有多个 impl 块
impl Task {
    /// 构造一个刚进入队列的任务：编号、名字、类型、优先级、估计工作量。
    // 【知识点：方法与关联函数】没有 self 参数的关联函数当构造函数用，
    // 调用写法是 `Task::new(...)`，路径里走的是类型名
    // 【知识点：泛型】name 处接受任何能转成 Rc<str> 的输入，
    // 所以 &str 与 String 都能直接传进来
    pub fn new(id: usize, name: &str, kind: TaskKind, priority: u32, estimate: u32) -> Self {
        Self {
            // 【知识点：变量与类型】字面量 0 与 usize 字段，靠字段类型推断
            id: TaskId(id),
            name: Rc::from(name),
            kind,
            // 【知识点：枚举】新任务只可能是排队状态
            state: TaskState::Ready,
            priority,
            estimate,
            done: 0,
        }
    }

    /// 还剩多少工作量；已做满时返回 0。
    // 【知识点：变量与类型】u32 是无符号数，相减前必须先确认不会下溢
    pub fn remaining(&self) -> u32 {
        // 【知识点：match 与模式匹配】用饱和减法把「已超额」压回 0
        self.estimate.saturating_sub(self.done)
    }

    /// 任务是否已经做完。
    pub fn is_finished(&self) -> bool {
        // 【知识点：表达式】函数的最后一个表达式就是返回值，不写 return
        self.done >= self.estimate
    }

    /// 这个任务在设备上占用的资源份额。
    // 【知识点：方法转发】把请求转给字段自己的方法，外面用起来少写一层
    pub fn workload(&self) -> u32 {
        u32::from(self.kind.weight())
    }
}

/// 「能把自己说清楚」的能力：任何实现它的类型都能给出一行摘要。
// 【知识点：Trait】trait 里只写签名，具体内容由实现者补；
// 这里给了一个默认实现，实现者可以沿用也可以覆盖
pub trait Describable {
    /// 返回一行摘要。
    fn summary(&self) -> String;

    /// 带前缀的摘要，默认实现调用上面的方法。
    // 【知识点：Trait 默认方法】默认方法可以调用同一 trait 里别的方法
    fn labeled(&self, tag: &str) -> String {
        // 【知识点：格式化】format! 按模板拼一个新 String 出来
        format!("[{tag}] {}", self.summary())
    }
}

// 【知识点：Trait 实现】为 Task 实现 Describable，写法是 `impl Trait for 类型`
impl Describable for Task {
    fn summary(&self) -> String {
        // 【知识点：格式化与 Debug】{state:?} 用 Debug 实现打印枚举变体名
        format!(
            "{:<12} {:<4} prio={:<3} {}/{} state={:?}",
            self.name,
            self.kind.device(),
            self.priority,
            self.done,
            self.estimate,
            self.state
        )
    }
}
