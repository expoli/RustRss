//! 运行期日志行为：级别门控 + panic hook。
//!
//! 为什么放在集成测试（独占一个测试进程）而不是 `logging.rs` 的 `#[cfg(test)]` 里：
//! 这两条都动**全局状态**（全局 logger / `log::max_level` / panic hook），与同进程内
//! 并行跑的其它单测互相干扰会变成偶发红；本文件独占进程，断言因此是确定性的。
//!
//! 覆盖 T2 验收 3（PRD 验收 2）：debug 关闭时不出现、打开时出现；panic 后日志文件里
//! 有 payload + 位置，且原 hook 仍被调用（既有 stderr 行为不丢）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use log::LevelFilter;

/// 原 hook 是否被调用过（新 hook 必须链式调用它，否则 stderr 那份现场就丢了）。
static PREVIOUS_HOOK_CALLED: AtomicBool = AtomicBool::new(false);

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustrss-logging-runtime-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("测试目录应能创建");
    dir
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("应能读回日志文件")
}

#[test]
fn level_gate_and_panic_hook_recording() {
    let dir = tmp_dir("runtime");
    // ① 启动口径：默认 info 初始化（init 装全局 logger + set_max_level）+ 装 panic hook
    let log_file = rustrss_core::logging::init(&dir, LevelFilter::Info).expect("init 应成功");
    rustrss_core::logging::install_panic_hook();
    assert_eq!(log::max_level(), LevelFilter::Info, "init 要按传入级别设门");

    // ② info 级别（默认）：debug 行不落盘，info 行照落
    log::debug!("DEBUG-不应出现-9f3c1a7b");
    log::info!("INFO-应出现-9f3c1a7b");
    let text = read(&log_file);
    assert!(
        !text.contains("DEBUG-不应出现"),
        "info 级别下 debug 不该落盘：{text}"
    );
    assert!(text.contains("INFO-应出现"), "info 行应落盘：{text}");

    // ③ 切到 debug（T3 的 `log.level=debug` 走同一条 set_max_level 路径）：debug 行出现
    log::set_max_level(LevelFilter::Debug);
    log::debug!("DEBUG-应出现-9f3c1a7b");
    let text = read(&log_file);
    let debug_line = text
        .lines()
        .find(|l| l.contains("DEBUG-应出现"))
        .unwrap_or_else(|| panic!("debug 打开后应有 debug 行：{text}"));
    assert!(debug_line.contains(" DEBUG "), "级别字段应补齐：{debug_line}");
    assert!(
        debug_line.contains("logging_runtime: DEBUG-应出现"),
        "行格式应是 `{{target}}: {{message}}`：{debug_line}"
    );
    assert!(
        debug_line.contains('T') && debug_line.contains('+'),
        "应带本地 RFC3339 时间戳：{debug_line}"
    );

    // ④ panic：日志文件里有 payload + 位置，且原 hook 仍被调用
    std::panic::set_hook(Box::new(|_| {
        PREVIOUS_HOOK_CALLED.store(true, Ordering::SeqCst);
    }));
    rustrss_core::logging::install_panic_hook();
    let caught = std::panic::catch_unwind(|| panic!("panic-载荷-9f3c1a7b"));
    assert!(caught.is_err(), "catch_unwind 应捕获到 panic");
    assert!(
        PREVIOUS_HOOK_CALLED.load(Ordering::SeqCst),
        "新 hook 必须调用原 hook（保留既有 stderr 行为）"
    );

    let text = read(&log_file);
    let panic_line = text
        .lines()
        .find(|l| l.contains("panic: "))
        .unwrap_or_else(|| panic!("日志里应有 panic 行：{text}"));
    assert!(
        panic_line.contains("panic-载荷-9f3c1a7b"),
        "payload 要进日志：{panic_line}"
    );
    assert!(panic_line.contains("location="), "位置要进日志：{panic_line}");
    assert!(
        panic_line.contains("logging_runtime.rs"),
        "位置应指向 panic 现场文件：{panic_line}"
    );
    assert!(
        panic_line.contains(" ERROR "),
        "panic 用 error 级别：{panic_line}"
    );
    assert!(
        panic_line.contains("rustrss_core::logging"),
        "target 应是 hook 所在模块：{panic_line}"
    );
}
