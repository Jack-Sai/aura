//! 执行期间阻止系统休眠（v0.3.2）
//!
//! 窗口最小化本身不会影响 Rust 侧的 tokio 任务，但**系统休眠**会让正在
//! 运行的子进程被挂起、流式请求中断。跑 `cargo build` 这类耗时命令时，
//! 笔记本一旦进入睡眠，任务就卡死到唤醒为止，表现为「模型突然没反应了」。
//!
//! 实现要点：`SetThreadExecutionState` 是**线程局部**的，而 tokio 的
//! work-stealing 会在 await 点迁移任务。因此不能直接在 `run_turn` 的线程上
//! 设置——必须由一个专用线程持有该状态，guard 负责通知它退出。

#[cfg(windows)]
mod platform {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    /// 持续生效直到被清除或状态被替换
    pub const ES_CONTINUOUS: u32 = 0x8000_0000;
    /// 阻止系统进入睡眠（不阻止显示器关闭，用户屏幕仍可正常熄灭）
    pub const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    pub fn request() {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
        }
    }

    pub fn release() {
        unsafe {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

/// 非 Windows 平台为无操作占位：当前仅 Windows 发行版受支持。
#[cfg(not(windows))]
mod platform {
    pub fn request() {}
    pub fn release() {}
}

/// 防休眠守卫。创建时阻止系统休眠，销毁时恢复。
pub struct KeepAwake {
    #[cfg(windows)]
    stop: Option<std::sync::mpsc::Sender<()>>,
    #[cfg(windows)]
    handle: Option<std::thread::JoinHandle<()>>,
}

impl KeepAwake {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let handle = std::thread::spawn(move || {
                platform::request();
                // 阻塞直到守卫释放。线程专属保证状态绑定在这一线程上。
                let _ = rx.recv();
                platform::release();
            });
            Self {
                stop: Some(tx),
                handle: Some(handle),
            }
        }
        #[cfg(not(windows))]
        {
            platform::request();
            Self {}
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            // 先发停止信号，再 join，确保 release() 在线程内完成
            if let Some(tx) = self.stop.take() {
                let _ = tx.send(());
            }
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
        #[cfg(not(windows))]
        platform::release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_is_safe_to_create_and_drop() {
        // 创建后立即释放，重复多次不应崩溃或残留线程
        for _ in 0..5 {
            let g = KeepAwake::new();
            assert!(std::ptr::addr_of!(g) as usize != 0);
            drop(g);
        }
    }

    #[test]
    fn multiple_guards_can_coexist() {
        let a = KeepAwake::new();
        let b = KeepAwake::new();
        drop(a);
        drop(b);
    }
}