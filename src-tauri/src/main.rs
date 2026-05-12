#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_os = "macos")]
mod safari_frameworks {
    use std::{env, ffi::OsString, os::unix::process::CommandExt as _, path::Path, process::Command};

    const SAFARI_FRAMEWORKS_DIR: &str = "/Library/Apple/System/Library/StagedFrameworks/Safari";
    const REEXEC_ENV: &str = "CLASH_VERGE_SAFARI_FRAMEWORKS_REEXEC";

    pub fn reexec_with_safari_frameworks() {
        if env::var_os(REEXEC_ENV).is_some() || !safari_frameworks_available() {
            return;
        }

        let current_framework_path = env::var_os("DYLD_FRAMEWORK_PATH");
        if framework_path_has_safari(&current_framework_path) {
            return;
        }

        let Ok(current_exe) = env::current_exe() else {
            return;
        };

        let mut framework_path = OsString::from(SAFARI_FRAMEWORKS_DIR);
        if let Some(existing) = current_framework_path
            && !existing.is_empty()
        {
            framework_path.push(":");
            framework_path.push(existing);
        }

        let error = Command::new(current_exe)
            .args(env::args_os().skip(1))
            .env(REEXEC_ENV, "1")
            .env("DYLD_FRAMEWORK_PATH", framework_path)
            .exec();

        eprintln!("failed to re-exec with Safari frameworks: {error}");
    }

    fn safari_frameworks_available() -> bool {
        let base = Path::new(SAFARI_FRAMEWORKS_DIR);
        base.join("WebKit.framework").is_dir()
            && base.join("JavaScriptCore.framework").is_dir()
            && base.join("WebCore.framework").is_dir()
    }

    fn framework_path_has_safari(value: &Option<OsString>) -> bool {
        value
            .as_ref()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.split(':').any(|path| path == SAFARI_FRAMEWORKS_DIR))
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    safari_frameworks::reexec_with_safari_frameworks();

    let default_parallelism = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let worker_limit = std::cmp::min(default_parallelism, 16);
    let blocking_limit = 4 * worker_limit;

    #[allow(clippy::unwrap_used)]
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_limit)
        .max_blocking_threads(blocking_limit)
        .enable_all()
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("clash-verge-runtime-{id}")
        })
        .build()
        .unwrap();
    let tokio_handle = tokio_runtime.handle();
    tauri::async_runtime::set(tokio_handle.clone());

    #[cfg(feature = "tokio-trace")]
    console_subscriber::init();

    app_lib::run();
}
