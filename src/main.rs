//! 多任务调度器模拟器：把一批 CPU 与 GPU 任务排进有限的名额里执行。
//!
//! 【知识点：包与 crate】src/main.rs 是这个包的二进制入口，
//! 它所在的模块就是 crate 根，所有模块声明都写在这里。

// 【知识点：模块与包管理】mod 声明本 crate 里有哪些模块，
// 编译器据此去找 src/error.rs、src/task.rs、src/scheduler.rs
mod error;
mod scheduler;
mod task;

use std::rc::Rc; // 【知识点：智能指针】引用计数指针

// 【知识点：模块与包管理】use 把路径引进来；模块树是分层的，路径按层写
use crate::error::SchedulerError;
use crate::scheduler::{Device, PlanItem, Scheduler, save_log};
use crate::task::{Describable, Task, TaskId, TaskKind, TaskState};

/// 一轮执行完之后的产出。
// 【知识点：结构体】把同一轮里相关的几个数放在一起，读起来是一件事
#[derive(Debug)]
struct RoundStat {
    round: usize,
    finished: usize,
    cpu_slices: u32,
    gpu_slices: u32,
}

/// main 的返回类型可以是 Result：错误会打到标准错误并让进程以非零码退出。
// 里面出现的 `?` 会把 SchedulerError 与 std::io::Error 都装进这个盒子里，
// 靠的是标准库为 Box<dyn Error> 实现的 From 转换。
// 【知识点：错误处理】一个函数只该有一种错误类型，装箱让不同的错误能共存
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 【知识点：变量与类型】let 绑定默认不可变；需要改就写 mut。
    // 时间片与两台设备的名额都是这一局的固定参数
    let slice = 5u32;
    let cpu_capacity = 4usize;
    let gpu_capacity = 4usize;

    // 【知识点：集合类型】Vec 是唯一的集合，程序里的任务清单就是它的内容
    let mut scheduler = Scheduler::new(slice);
    let cpu = Device::new("CPU", cpu_capacity);
    let gpu = Device::new("GPU", gpu_capacity);
    // 【知识点：所有权的移动】cpu 与 gpu 的所有权在这一行转给了这个容器
    let mut devices = vec![cpu, gpu];

    // 任务清单：名称、类型、优先级、估计工作量
    // 【知识点：元组字面量】每一项是一个四元组，类型由上面的声明定下来
    let specs: [(&str, TaskKind, u32, u32); 11] = [
        ("vision-detect", TaskKind::Cpu(2), 80, 9),
        ("sensor-fuse", TaskKind::Cpu(1), 60, 6),
        ("path-plan", TaskKind::Cpu(2), 90, 9),
        ("log-rotate", TaskKind::Cpu(1), 20, 3),
        ("cache-warm", TaskKind::Cpu(1), 30, 3),
        ("kernel-matmul", TaskKind::Gpu(3), 95, 9),
        ("kernel-attn", TaskKind::Gpu(2), 70, 6),
        ("kernel-norm", TaskKind::Gpu(2), 50, 6),
        ("kernel-softmax", TaskKind::Gpu(1), 40, 3),
        ("prefetch", TaskKind::Gpu(1), 10, 3),
        ("kv-copy", TaskKind::Gpu(1), 15, 3),
    ];

    // 【知识点：迭代器与闭包】enumerate 交出「第几个 + 元素」，
    // 编号就是遍历顺序；map 把每一项变成任务，collect 再把结果收成 Vec
    let tasks: Vec<Task> = specs
        .iter()
        .enumerate()
        // 【知识点：元组解构】把四元组拆成四个绑定；
        // kind 是 &TaskKind，复制一份出来（TaskKind 实现了 Copy）
        .map(|(index, (name, kind, priority, estimate))| {
            Task::new(index + 1, name, *kind, *priority, *estimate)
        })
        .collect();

    println!("== 调度器起步 ==");
    println!("时间片 = {slice} 单位，CPU 名额 = {cpu_capacity}，GPU 名额 = {gpu_capacity}");
    println!("排队任务 = {} 个", tasks.len());

    // 【知识点：借用与生命周期】借一个任务名出来用，不拥有它；
    // 这份借用在这一行结束后就不再需要
    println!("表头任务 = {}", tasks[0].name);

    // 【知识点：所有权的移动】把建好的任务逐个交给调度器
    for task in tasks {
        // 【知识点：错误处理】add 的返回类型是 Result；这里的 unwrap 表示
        // 「编号由 enumerate 生成，不可能撞号」，真出错时进程会带错误信息结束
        scheduler.add(task).unwrap();
    }

    // 【知识点：Trait 方法】labeled 与 summary 都来自 Describable，
    // Task 与 Device 各自给出自己的那一行
    for task in scheduler.tasks() {
        println!("  {}", task.labeled("排队"));
    }

    // 两台设备各自接纳一个任务
    // 【知识点：索引与可变借用】要改 devices 里的元素，得先可变借用整个 Vec
    // 【知识点：借用与函数的配合】scheduler.tasks() 给出的是只读借用，
    // 传进 admit 时只读借用与 devices 的可变借用互不相干
    println!(
        "{}",
        devices[0]
            .admit(&scheduler.tasks()[0])
            .map(|_| "CPU 接纳成功")
            .unwrap_or("CPU 拒绝")
    );

    // 【知识点：Option 与 if let】find 可能给不出结果，用 if let 只处理有值的情形
    if let Some(task) = scheduler.find(TaskId(6)) {
        // 【知识点：所有权与借用】task 是 &Task，读字段不复制任何东西
        let weight = task.workload();
        let admitted = devices[1].admit(task);
        println!(
            "GPU 接纳 {}（份额 {weight}）→ {:?}",
            task.name,
            admitted.is_ok()
        );
    }

    // 【知识点：函数调用】函数收的是 &Task，借用进去，所有权留在原处
    let backlog = heavier(&scheduler.tasks()[1], &scheduler.tasks()[2]);
    println!("剩余较多的任务 = {}", backlog.name);

    // 【知识点：泛型方法】total_by 的返回类型跟着闭包的返回类型走；
    // 闭包 |t| t.kind 直接当分组键，因此同一台设备上的不同份额会各占一行
    // 此刻还没有任务跑过，所以每组都是 0——这个 0 本身就是「按份额分别统计」的证据
    let totals = scheduler.total_by(|t| t.kind);
    println!("\n== 按任务类型分组（此刻已完成的工作量） ==");
    for (kind, total) in &totals {
        println!("  {kind:?} 合计 {total}");
    }

    // 【知识点：闭包捕获】这次按设备名分组，闭包捕获的是 TaskKind 上的方法
    // device()，返回的键是 &'static str，于是同一台设备的份额被合到一起
    let by_device = scheduler.total_by(|t| t.kind.device());
    println!("== 按设备分组（此刻已完成的工作量） ==");
    for (device_name, total) in &by_device {
        println!("  {device_name} 合计 {total}");
    }

    // 【知识点：迭代器与闭包】分组键也可以是自己算出来的东西：
    // 这里把每个任务的份额加起来，得到整批任务的份额总量
    let total_workload: u32 = specs
        .iter()
        .map(|(_, kind, _, _)| kind.weight() as u32)
        .sum();
    println!("整批任务的份额总量 = {total_workload}");

    println!("\n== 逐轮执行 ==");
    let stats = run_simulation(&mut scheduler, cpu_capacity, gpu_capacity)?;
    report(&stats);

    // 【知识点：智能指针】completions 里每条记录持有一份 Rc<str>；
    // 这里克隆出 Rc 的副本，字符串本身仍然只有一份
    let names: Vec<Rc<str>> = scheduler
        .completions()
        .iter()
        .map(|done| Rc::clone(&done.name))
        .collect();
    println!(
        "\n完成顺序：{}",
        names
            .iter()
            .map(|n| n.as_ref())
            .collect::<Vec<_>>()
            .join(" → ")
    );
    // 【知识点：结构体字段】记录里还有编号与那一段的时间片，
    // 于是「第几个完成、各跑了多久」都能从同一份记录里读出来
    println!("\n== 逐条完成记录 ==");
    for (order, done) in scheduler.completions().iter().enumerate() {
        println!(
            "  第 {} 个完成：{:<14} 编号 {}，本次 {} 单位",
            order + 1,
            done.name,
            done.id,
            done.slice
        );
    }

    // 另一条路：不关心每轮细节时，一次推进到底
    // 【知识点：泛型方法】这里留下的信息写成空元组 ()，
    // 因为只想知道「一共推了多少轮」
    let rounds_all = scheduler.execute_all(cpu_capacity, gpu_capacity)?;
    println!("\n再次检查：execute_all 返回 {rounds_all} 轮（此时已无任务可排）");

    // 再次统计，这次拿到的是实际完成的量
    let done_totals = scheduler.total_by(|t| t.kind.device());
    println!("实际完成量：{done_totals:?}");

    // 【知识点：生命周期】视图借用调度器，所以要在改它之前用完
    let ids = [TaskId(1), TaskId(5), TaskId(99)];
    for view in scheduler.views(&ids) {
        // 【知识点：match 与模式匹配】Option 的两支都要照顾到；
        // 这里只用到视图的三个字段，名字是借来的 &str
        match view {
            Some(v) => println!(
                "  {} {} 在 {}，还剩 {}",
                v.id,
                v.name,
                state_name(v.state),
                v.remaining
            ),
            None => println!("  （该编号不在任务表里）"),
        }
    }

    // 【知识点：错误处理】两类失败被当成正常数据返回，程序继续往下走
    let mut overflow = Device::new("NPU", 3);
    match overflow.admit(&scheduler.tasks()[1]) {
        Ok(()) => println!("NPU 接纳成功"),
        // 【知识点：Trait 与格式化】{e} 用的是 Display 实现，{e:?} 用的是 Debug
        Err(e) => println!("NPU 拒绝：{e}"),
    }
    // 【知识点：错误处理】剩下一个份额放不下份额 3 的任务，同一台设备会拒绝它
    match overflow.admit(&scheduler.tasks()[5]) {
        Ok(()) => println!("NPU 第二次接纳成功"),
        Err(e) => println!("NPU 第二次拒绝：{e}"),
    }
    match scheduler.run_slice(TaskId(999)) {
        Ok(work) => println!("空转 {work} 单位"),
        Err(e) => println!("执行失败：{e}"),
    }

    // 【知识点：错误处理】写文件可能失败；这里用 let _ 明确表示「结果不关心」，
    // 编译器的 unused_must_use 提示也就不会出现
    // 两台设备各写一个文件：fs::write 会截断同名文件，共用一条路径会互相覆盖
    println!("调度器状态 = {}", scheduler_summary(&scheduler));
    for device in &mut devices {
        let moved = device.flush_log();
        println!(
            "{} 搬走 {moved} 条日志，累计 {} 条",
            describe(device),
            device.logged()
        );
        let path = format!("sched-log-{}.txt", device.name());
        let _ = save_log(&path, device);
        println!("  日志写入 {path}");
    }
    Ok(())
}

/// 逐轮排计划、执行、统计，返回每一轮的产出。
fn run_simulation(
    scheduler: &mut Scheduler,
    cpu_capacity: usize,
    gpu_capacity: usize,
) -> Result<Vec<RoundStat>, SchedulerError> {
    let mut stats: Vec<RoundStat> = Vec::new();
    let mut round = 0usize;
    let mut total_slices = 0u32;
    loop {
        round += 1;
        let before = scheduler.completions().len();
        // 【知识点：泛型方法调用】map 决定「每条计划要留下什么」，
        // 这里留下的是给它准备的一行文字，顺手把两台设备的时间片加起来
        let mut cpu_slices = 0u32;
        let mut gpu_slices = 0u32;
        // 【知识点：闭包捕获】闭包捕获了外层的两个计数变量，
        // 因此它必须能改它们；这类闭包实现的是 FnMut
        // 外面这层花括号把闭包的寿命限制在块内，块一结束计数变量就能读了
        let outcome = {
            let note = |item: &PlanItem<'_>| {
                if item.device == "CPU" {
                    cpu_slices += item.slice;
                } else {
                    gpu_slices += item.slice;
                }
                // 【知识点：借用】item.name 是借来的 &str，格式化时只读它
                format!("{}({},{})", item.id, item.name, item.slice)
            };
            scheduler.advance_round(cpu_capacity, gpu_capacity, note)?
        };
        if outcome.is_empty() {
            break;
        }
        total_slices += cpu_slices + gpu_slices;
        let mut planned = Vec::new();
        for (note_text, work) in &outcome {
            planned.push(format!("{note_text}={work}"));
        }
        println!(
            "轮 {round:>2} 推进 {} 条：{}",
            outcome.len(),
            planned.join(" ")
        );
        stats.push(RoundStat {
            round,
            finished: scheduler.completions().len() - before,
            cpu_slices,
            gpu_slices,
        });
    }
    println!("累计推进 {total_slices} 个时间片单位");
    Ok(stats)
}

/// 打印每轮统计，并指出吞吐最高的那一轮。
fn report(stats: &[RoundStat]) {
    println!("\n== 轮次统计 ==");
    println!(
        "{:<6}{:<10}{:<12}{:<12}",
        "轮", "本轮完成", "CPU 时间片", "GPU 时间片"
    );
    for stat in stats {
        println!(
            "{:<6}{:<10}{:<12}{:<12}",
            stat.round, stat.finished, stat.cpu_slices, stat.gpu_slices
        );
    }
    // 【知识点：迭代器与闭包】max_by_key 用闭包挑出「完成数最大」的那一轮；
    // 闭包捕获了 stat 本身，返回值是 Option<&RoundStat>
    let busiest = stats.iter().max_by_key(|stat| stat.finished);
    if let Some(stat) = busiest {
        println!("本轮完成最多：第 {} 轮（{} 个）", stat.round, stat.finished);
    }
}

/// 比较两个任务，返回剩余工作量较大的那一个。
// 【知识点：生命周期】返回的引用来自 a 或 b 之一，编译器无法自己判断它该跟谁同生，
// 所以要显式写出 <'a> 并标注两个入参与返回值用的是同一个 'a；
// 少了这个标注，编译直接报「缺少生命周期说明符」
fn heavier<'a>(a: &'a Task, b: &'a Task) -> &'a Task {
    if a.remaining() >= b.remaining() { a } else { b }
}

/// 用一行字描述一台设备或一个任务。
// 【知识点：Trait object】参数写 &dyn Describable，表示「任何实现了 Describable 的值」；
// 具体是哪个类型到运行期才知道，所以这里是运行期分派，而不是对每种类型各生成一份代码
fn describe(thing: &dyn Describable) -> String {
    thing.summary()
}

/// 把任务状态翻成中文。
// 【知识点：枚举与 match】枚举的每个变体都要照顾到，漏一个编译期就会报出来
fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Ready => "排队",
        TaskState::Running => "运行",
        TaskState::Finished => "完成",
    }
}

/// 汇总调度器当前状态。
fn scheduler_summary(scheduler: &Scheduler) -> String {
    // 【知识点：迭代器与闭包】filter 只留完成的任务，count 数个数
    let finished = scheduler
        .tasks()
        .iter()
        .filter(|t| t.state == TaskState::Finished)
        .count();
    format!(
        "任务 {} 个，完成 {finished} 个，记录 {} 条",
        scheduler.tasks().len(),
        scheduler.completions().len()
    )
}
