//! 本模块是调度器本体：一张任务表、一台设备的占用名额、一份执行记录。

// 【知识点：集合类型】标准库的集合都在 std::collections 下，按需要单个引进
use std::cell::RefCell; // 内部可变性：只有共享借用也能改里面的内容
use std::collections::HashMap; // 哈希表：按键查值
use std::fs; // 一次性写整个文件用
use std::rc::Rc; // 引用计数指针：多处共享同一份只读数据

use crate::error::SchedulerError;
use crate::task::{Describable, Task, TaskId, TaskKind, TaskState};

/// 一条调度计划：某个任务在某台设备上跑多久。
// 【知识点：生命周期】类型名后面的 <'a> 声明一个生命周期参数，
// 意思是「这个结构体借用了别处的数据，借多久由 'a 说」；
// name 是借来的字符串切片，结构体自己不管它的存储
#[derive(Debug, Clone)]
pub struct PlanItem<'a> {
    /// 任务编号。
    pub id: TaskId,
    /// 任务名，借自任务表里那个 Rc<str>。
    // 【知识点：借用】&'a str 是共享借用：只读，可以同时存在多份
    pub name: &'a str,
    /// 跑在哪台设备上。
    pub device: &'static str,
    /// 任务的优先级分数。
    pub priority: u32,
    /// 这一段分到的时间片。
    pub slice: u32,
}

/// 一个任务的只读视图，字段全是借来的。
// 【知识点：生命周期】视图本身不拥有数据，它的寿命不可能超过被看的那张表
#[derive(Debug, Clone, Copy)]
pub struct TaskView<'a> {
    pub id: TaskId,
    pub name: &'a str,
    pub state: TaskState,
    pub remaining: u32,
}

/// 一次时间片执行完留下的记录。
// 【知识点：智能指针】name 用 Rc<str> 而不是 String：
// 任务是有限个，名字被任务与记录两处共享，克隆 Rc 只加计数、不复制字节
#[derive(Debug, Clone)]
pub struct Completion {
    pub id: TaskId,
    pub name: Rc<str>,
    pub slice: u32,
}

/// 调度器：任务的唯一持有者。
#[derive(Debug)]
pub struct Scheduler {
    /// 任务表，顺序即加入顺序。
    // 【知识点：集合类型】Vec<T> 是可增长数组，按位置访问与顺序遍历都很便宜
    tasks: Vec<Task>,
    /// 时间片长度。
    // 【知识点：变量与类型】u32 表示单位数，不会为负
    slice: u32,
    /// 已经完成的记录，按完成先后排列。
    finished: Vec<Completion>,
}

impl Scheduler {
    /// 建一个空调度器，指定每段执行的时间片长度。
    pub fn new(slice: u32) -> Self {
        Self {
            // 【知识点：集合类型】Vec::new 造一个空表，元素类型由字段声明定下来
            tasks: Vec::new(),
            slice,
            finished: Vec::new(),
        }
    }

    /// 把任务表交给调用方只读查看。
    // 【知识点：借用】返回 &[Task] 而不是 &Vec<Task>：
    // 切片更通用，只承诺「能挨个读到 Task」，不承诺底层是哪种容器
    pub fn tasks(&self) -> &[Task] {
        // 【知识点：借用与自动解引用】&self.tasks 是 &Vec<Task>，
        // 返回类型要 &[Task]，这里发生一次把 Vec 当数组看的转换
        &self.tasks
    }

    /// 按编号找任务，返回一个只读可选借用。
    // 【知识点：Option】「可能找不到」用 Option<&Task> 表达，
    // 调用方必须处理「没有」这一支，不能忘记
    pub fn find(&self, id: TaskId) -> Option<&Task> {
        // 【知识点：迭代器与闭包】iter() 逐个交出 &Task，
        // find 用闭包决定哪一个算命中
        // 【知识点：闭包】|t| 是闭包的参数表，它捕获了外层的 id
        self.tasks.iter().find(|t| t.id == id)
    }

    /// 加入一个任务；编号重复时返回错误。
    // 【知识点：错误处理】可能失败的操作用 Result 当返回类型：
    // 成功是 Ok(值)，失败是 Err(错误值)
    pub fn add(&mut self, task: Task) -> Result<(), SchedulerError> {
        // 【知识点：借用】这里先用共享借用查重，借用在这一行结束时已经还回去
        if self.find(task.id).is_some() {
            // 【知识点：所有权的移动】把 task.id 复制进错误值；
            // TaskId 实现了 Copy，所以 task 之后还能用
            return Err(SchedulerError::DuplicateId(task.id));
        }
        // 【知识点：所有权的移动】push 把 task 的所有权收进 Vec，调用方不再拥有它
        self.tasks.push(task);
        // 【知识点：错误处理】成功却没有值要交，写成 Ok(())，() 是「空元组」
        Ok(())
    }

    /// 按优先级排出一段执行计划：CPU 与 GPU 各取容量允许的若干条。
    // 【知识点：生命周期】返回的 PlanItem 借用了 self，签名里的 '_ 表示
    // 「借多久跟 self 一样长」；调用方拿着计划期间，调度器不能被改
    pub fn plan(
        &self,
        cpu_capacity: usize,
        gpu_capacity: usize,
    ) -> Result<Vec<PlanItem<'_>>, SchedulerError> {
        // 【知识点：错误处理】把「参数不合法」这一类失败挡在入口，
        // 后面就不必反复检查
        if self.slice == 0 {
            return Err(SchedulerError::ZeroSlice);
        }
        let mut cpu_used = 0usize;
        let mut gpu_used = 0usize;
        // 【知识点：集合类型】计划本身是个 Vec，长度随入选条数增长
        let mut plan: Vec<PlanItem<'_>> = Vec::new();

        // 【知识点：所有权与借用】tasks() 交出的切片是共享借用；
        // 遍历它只读，因此期间不许有人改任务表
        for task in self.tasks() {
            // 【知识点：match 与模式匹配】已经做完的任务不再参与调度
            if task.is_finished() {
                continue;
            }
            // 【知识点：变量与类型】u8 转 usize，索引与计数都用 usize
            let weight = task.kind.weight() as usize;
            // 【知识点：match 与模式匹配】按设备类型决定动哪个容量计数
            let (used, capacity) = match task.kind {
                TaskKind::Cpu(_) => (&mut cpu_used, cpu_capacity),
                TaskKind::Gpu(_) => (&mut gpu_used, gpu_capacity),
            };
            if *used + weight > capacity {
                continue;
            }
            *used += weight;
            plan.push(PlanItem {
                id: task.id,
                // 【知识点：借用】把 Rc<str> 解引用成 &str 借出来，不复制字符串
                name: &task.name,
                device: task.kind.device(),
                priority: task.priority,
                // 【知识点：方法调用】min 取「时间片」与「剩余量」里的小者
                slice: self.slice.min(task.remaining()),
            });
        }

        // 【知识点：迭代器与闭包】sort_by 用闭包给出两个元素的先后：
        // 先比优先级，再比编号，所以结果稳定可复现；
        // b 与 a 的位置对调，就得到从大到小
        plan.sort_by(|a, b| (b.priority, b.id.0).cmp(&(a.priority, a.id.0)));
        Ok(plan)
    }

    /// 取出若干任务的只读视图，供外部检查状态。
    pub fn views(&self, ids: &[TaskId]) -> Vec<Option<TaskView<'_>>> {
        // 【知识点：迭代器与闭包】map 把每个编号变成「查到的视图」，
        // collect 再把结果收成一个 Vec
        ids.iter()
            .map(|id| {
                self.find(*id).map(|t| TaskView {
                    id: t.id,
                    name: &t.name,
                    state: t.state,
                    remaining: t.remaining(),
                })
            })
            .collect()
    }

    /// 让某个任务跑一个时间片；任务做完就记账。
    // 【知识点：可变借用】&mut self 表示这次调用要改调度器，
    // 调用期间独占它，别的借用都不许同时存在
    pub fn run_slice(&mut self, id: TaskId) -> Result<u32, SchedulerError> {
        // 【知识点：可变借用】iter_mut 交出 &mut Task，同一时刻只能有这一份
        // 【知识点：? 运算符】找不到就返回 Err，找到就拿到那个可变借用
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(SchedulerError::UnknownTask(id))?;

        let work = task.remaining().min(self.slice);
        task.done += work;
        // 【知识点：枚举】状态随进度变化，每次执行后重算一次
        task.state = if task.is_finished() {
            TaskState::Finished
        } else {
            TaskState::Running
        };
        // 【知识点：智能指针】Rc::clone 只让引用计数加一，
        // task.name 仍然有效，不必移交所有权
        let name = Rc::clone(&task.name);
        let done_all = task.is_finished();
        // 【知识点：借用结束】到这里对 task 的可变借用已经用完了；
        // 后面改 finished 不会与它冲突
        if done_all {
            // 【知识点：所有权的移动】把克隆出来的那份 Rc 移进记录表
            self.finished.push(Completion {
                id,
                name,
                slice: work,
            });
        }
        Ok(work)
    }

    /// 推进一轮：排一次计划并执行计划里的每一条，返回这一轮跑过的任务带来的一串信息。
    // 【知识点：泛型】Summary 是类型参数，map 决定「每条计划要留下什么」；
    // 返回的 Vec<(Summary, u32)> 是「留下的信息 + 这一条跑掉的时间片」
    // 【知识点：闭包三类】约束写 FnMut，因为调用方常常用一个会改自己捕获变量的
    // 闭包来顺手记账；只要不需要改自己，Fn 的闭包也满足 FnMut
    // 【知识点：生命周期】计划借用 self，所以这一整轮里调度器不能被别人改
    pub fn advance_round<Summary>(
        &mut self,
        cpu_capacity: usize,
        gpu_capacity: usize,
        mut map: impl FnMut(&PlanItem<'_>) -> Summary,
    ) -> Result<Vec<(Summary, u32)>, SchedulerError> {
        // 【知识点：借用边界】plan 借用了 self，它必须在 run_slice 之前结束：
        // 先把本轮要执行的编号与要留下的信息抄出来，再把计划丢掉
        let plan = self.plan(cpu_capacity, gpu_capacity)?;
        let units: Vec<(TaskId, Summary)> = plan.iter().map(|item| (item.id, map(item))).collect();
        // 【知识点：所有权的回收】drop 在这里显式结束 plan 的借用
        drop(plan);

        let mut outcome = Vec::new();
        for (id, summary) in units {
            // 【知识点：? 运算符】单个时间片失败就整体失败，错误向上传
            let work = self.run_slice(id)?;
            outcome.push((summary, work));
        }
        Ok(outcome)
    }

    /// 反复推进，直到所有任务都完成，返回推进的轮数。
    pub fn execute_all(
        &mut self,
        cpu_capacity: usize,
        gpu_capacity: usize,
    ) -> Result<usize, SchedulerError> {
        // 【知识点：变量与类型】usize 记录轮数
        let mut rounds = 0usize;
        loop {
            // 【知识点：迭代器与闭包】all 要求每个任务都满足条件才为真；
            // 闭包 |t| t.is_finished() 只读地借用每个任务
            if self.tasks.iter().all(|t| t.is_finished()) {
                // 【知识点：流程控制】break 跳出循环
                break;
            }
            // 【知识点：泛型方法调用】这次只关心「这一轮排出了几条」，
            // 所以留下的信息是一个空元组
            let outcome = self.advance_round(cpu_capacity, gpu_capacity, |_| ())?;
            if outcome.is_empty() {
                // 名额排不出任何任务，再转下去也是空转
                break;
            }
            rounds += 1;
        }
        Ok(rounds)
    }

    /// 按任意标准统计已完成的工作量。
    // 【知识点：泛型】Key 是类型参数，by 决定怎么算分组键；
    // `impl FnMut(&Task) -> Key` 是泛型约束，意思是「任何能这样调用的闭包都行」
    // 【知识点：Trait】FnMut 是标准库给闭包定的三个 trait 之一，说明它可以被
    // 反复调用，并且允许改自己捕获到的变量
    pub fn total_by<Key>(&self, mut by: impl FnMut(&Task) -> Key) -> Vec<(Key, u32)>
    where
        Key: PartialEq,
    {
        let mut groups: Vec<(Key, u32)> = Vec::new();
        for task in &self.tasks {
            let key = by(task);
            // 【知识点：迭代器与闭包】在已经出现的分组里找同名的那一桶
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, total)) => *total += task.done,
                None => groups.push((key, task.done)),
            }
        }
        groups
    }

    /// 执行记录表的只读切片。
    pub fn completions(&self) -> &[Completion] {
        &self.finished
    }
}

/// 一台设备：容量固定，占用份额记在一张哈希表里。
// 【知识点：结构体与所有权】结构体拥有自己的字段，这里 running 拥有一张表
#[derive(Debug)]
pub struct Device {
    name: String,
    /// 能同时容纳的份额总数。
    capacity: usize,
    /// 正在运行的任务编号与各自占用的份额。
    // 【知识点：集合类型】HashMap 适合「按编号查份额」；
    // HashSet 只知道在不在，需要顺带记住一个值时就用 map
    running: HashMap<TaskId, usize>,
    /// 设备自己的日志。
    // 【知识点：智能指针与内部可变性】RefCell<Vec<String>> 包着日志：
    // 只拿到共享借用的方法也能往里追加，借用规则由 RefCell 在运行期检查
    log: RefCell<Vec<String>>,
    /// 已经搬出来的日志。
    // 【知识点：智能指针】Box<Vec<String>> 把整个 Vec 放到堆上，
    // Device 自身的大小与日志条数无关；Box 是唯一所有者的堆分配
    sink: Box<Vec<String>>,
}

impl Device {
    /// 建一台设备。
    // 【知识点：泛型与转换】impl Into<String> 让 &str 与 String 都能传进来，
    // 进了函数体再转成 String 存下
    pub fn new(name: impl Into<String>, capacity: usize) -> Self {
        Self {
            name: name.into(),
            capacity,
            // 【知识点：集合类型】HashMap::new 造一张空表，元素类型由字段声明定下来
            running: HashMap::new(),
            log: RefCell::new(Vec::new()),
            sink: Box::new(Vec::new()),
        }
    }

    /// 在当前名额下接纳一个任务。
    // 【知识点：借用】只需要读任务，所以收 &Task 而不是 Task：
    // 设备不拥有任务，任务表仍然归调度器
    pub fn admit(&mut self, task: &Task) -> Result<(), SchedulerError> {
        let id = task.id;
        // 【知识点：集合类型】contains_key 查这个编号在不在
        if self.running.contains_key(&id) {
            return Ok(());
        }
        // 【知识点：迭代器与闭包】把已经在跑的任务份额加起来；
        // values() 只交出值，sum 把一串数加成总数
        let used: usize = self.running.values().sum();
        let weight = task.workload() as usize;
        if used + weight > self.capacity {
            // 【知识点：枚举变体带字段】错误里带上设备名与两个数字，
            // 调用方不必再去别处打听发生了什么
            return Err(SchedulerError::CapacityFull {
                // 【知识点：借用】clone 出一份独立的设备名：错误值要自己拥有它
                device: self.name.clone(),
                occupied: used,
                capacity: self.capacity,
            });
        }
        // 【知识点：集合类型】insert 同时收下键与值；它返回的旧值这里用不到
        self.running.insert(id, weight);
        // 【知识点：内部可变性】log 字段本身是共享借用得到的东西，
        // borrow_mut 在运行期取到可变借用，因此这里不必声明 &mut self
        self.log
            .borrow_mut()
            .push(format!("{} 接纳任务 {}", self.name, id));
        Ok(())
    }

    /// 把日志搬进缓冲，返回搬走的条数。
    // 【知识点：内部可变性】把 borrow_mut 的结果先存进一个变量，
    // 用完即放；同一个 RefCell 在同一个时刻只能借出一次
    pub fn flush_log(&mut self) -> usize {
        let mut current = self.log.borrow_mut();
        let moved = current.len();
        self.sink.append(&mut current);
        moved
    }

    /// 缓冲里的日志条数。
    pub fn logged(&self) -> usize {
        // 【知识点：借用与自动解引用】self.sink 是 Box<Vec<String>>，
        // 方法调用会沿着 Box 自动走到 Vec 上，不必手写 (*self.sink).len()
        self.sink.len()
    }

    /// 设备名。
    // 【知识点：借用】返回借来的 &str，调用方要用更久就自己 clone 一份
    pub fn name(&self) -> &str {
        &self.name
    }
}

// 【知识点：Trait 实现】设备也能自我描述；实现 trait 的类型不必与 trait 同模块
impl Describable for Device {
    fn summary(&self) -> String {
        // 【知识点：借用】只读地借用 log 里的内容，这里只读条数
        format!(
            "{:<4} 名额 {}/{} 日志 {} 条",
            self.name,
            self.running.len(),
            self.capacity,
            self.log.borrow().len()
        )
    }
}

/// 把一台设备已经搬出来的日志写成文件。
// 【知识点：错误处理】返回 Result，把 IO 失败交回调用方去决定怎么处理
pub fn save_log(path: &str, device: &Device) -> std::io::Result<()> {
    let mut text = String::new();
    // 【知识点：迭代器】对已有日志逐行追加，顺序不变
    for line in device.sink.iter() {
        text.push_str(line);
        text.push('\n');
    }
    // 【知识点：泛型接口】fs::write 的路径参数接受任何实现了 AsRef<Path> 的值，
    // 因此 &str 与 String 都能直接传
    fs::write(path, text)
}
