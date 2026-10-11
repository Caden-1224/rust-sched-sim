//! 本模块只做一件事：把「调度器可能失败的方式」写成一个类型。
//!
//! 【知识点：模块与包管理】`mod error;` 在 crate 根里声明本文件是一个模块；
//! 文件名叫什么由声明决定，`mod error;` 去找 `error.rs` 或 `error/mod.rs`。

// 【知识点：模块与包管理】`use` 把别处的名字引进本模块的作用域，省掉写全路径
use std::error::Error; // 标准库的错误 trait，实现它才算「一个正经的错误类型」
use std::fmt; // 标准库的格式化模块，Display 的实现要用它的类型

use crate::task::TaskId; // crate:: 表示「从本 crate 的根开始」找路径

// 【知识点：枚举】一个枚举类型把一个值可能处于的若干种形态一次列全，
// 每种形态还能各自带数据；这里的每个变体就是调度器的一种失败方式
// 【知识点：derive 与 trait 派生】derive 让编译器替我们生成 trait 实现：
// Debug 给出可打印的调试格式，Clone 让错误值可以被复制，PartialEq 让错误值可以用 == 比较

#[derive(Debug, Clone, PartialEq)]

pub enum SchedulerError {
    /// 同一个任务编号被加入两次。
    // 【知识点：枚举】变体带数据时，写在变体名后面的括号里
    DuplicateId(TaskId),
    /// 调度计划里出现了一个表里没有的任务。
    UnknownTask(TaskId),
    /// 设备上的名额用完了。
    // 【知识点：结构体与枚举】带字段的变体写成结构体的形状，使用处按字段名读
    CapacityFull {
        device: String,
        occupied: usize,
        capacity: usize,
    },
    /// 要求排出的时间片数量是零。
    ZeroSlice,
}

// 【知识点：Trait 与 derive】traits 描述「一个类型能做什么」；Display 的含义是
// 「这个值可以按人看的格式印出来」，`{}` 就是用这个实现
impl fmt::Display for SchedulerError {
    // 【知识点：方法签名】`&self` 是对「调用它的那个错误值」的共享借用
    // 【知识点：生命周期省略】返回的 Formatter 带生命周期参数，这里省略不写；
    // 单个输入借用且方法带 &self 时，编译器按规则补全
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 【知识点：match 与模式匹配】match 把值拆开，每条分支负责一种形态
        match self {
            // 分支里绑定到的 id 是 `&TaskId`，直接打印即可
            SchedulerError::DuplicateId(id) => write!(f, "任务编号 {id} 已经存在"),
            SchedulerError::UnknownTask(id) => write!(f, "计划里出现了未知任务 {id}"),
            SchedulerError::CapacityFull {
                device,
                occupied,
                capacity,
            } => write!(f, "{device} 名额已满（{occupied}/{capacity}）"),
            SchedulerError::ZeroSlice => write!(f, "时间片数量为零，排不出任何计划"),
        }
    }
}

// 【知识点：Trait】Error 是标准库给「错误类型」定的契约，它默认要求同时实现
// Display 与 Debug；实现它之后，这个错误就能被 `?` 传播、被别的错误包装
impl Error for SchedulerError {}
