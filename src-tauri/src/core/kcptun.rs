use crate::{
    config::{Config, IKcpProxy, kcp_listen_host},
    core::handle::Handle,
    utils::dirs,
};
use anyhow::{Context as _, Result, anyhow, bail};
use clash_verge_logging::{Type, logging};
use once_cell::sync::Lazy;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tauri::Manager as _;
use tauri::utils::platform::current_exe;
use tokio::sync::Mutex;

static KCPTUN_MANAGER: Lazy<KcptunManager> = Lazy::new(KcptunManager::default);

#[derive(Default)]
pub struct KcptunManager {
    child: Mutex<Option<Child>>,
}

impl KcptunManager {
    pub fn global() -> &'static Self {
        &KCPTUN_MANAGER
    }

    pub async fn sync_with_config(&self) -> Result<()> {
        let runtime = load_kcp_runtime_config().await;

        if runtime.config.enabled.unwrap_or(false) {
            self.restart_with_config(runtime).await
        } else {
            self.stop().await;
            Ok(())
        }
    }

    pub async fn start(&self) -> Result<()> {
        let runtime = load_kcp_runtime_config().await;
        self.start_with_config(runtime).await
    }

    pub async fn restart(&self) -> Result<()> {
        let runtime = load_kcp_runtime_config().await;
        self.restart_with_config(runtime).await
    }

    pub async fn stop(&self) {
        let mut child = self.child.lock().await;
        if let Some(mut child) = child.take() {
            let pid = child.id();
            if let Err(err) = child.kill() {
                logging!(warn, Type::Core, "Failed to stop kcptun client {pid}: {err}");
            }
            let _ = child.wait();
            logging!(info, Type::Core, "kcptun client stopped: {pid}");
        }
    }

    pub async fn is_running(&self) -> bool {
        let mut child = self.child.lock().await;
        child_is_running(&mut child)
    }

    async fn restart_with_config(&self, runtime: KcptunRuntimeConfig) -> Result<()> {
        self.stop().await;
        self.start_with_config(runtime).await
    }

    async fn start_with_config(&self, runtime: KcptunRuntimeConfig) -> Result<()> {
        let config = &runtime.config;
        if !config.enabled.unwrap_or(false) {
            bail!("kcptun is disabled");
        }

        let bin_path = self.binary_path(config)?;
        let launch = self.build_launch(&runtime)?;
        let pid = {
            let mut child_slot = self.child.lock().await;
            if child_is_running(&mut child_slot) {
                return Ok(());
            }

            logging!(info, Type::Core, "Starting kcptun client: {}", bin_path.display());

            let child = Command::new(&bin_path)
                .args(launch.args)
                .env("KCPTUN_KEY", launch.key)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| format!("failed to start kcptun client: {}", bin_path.display()))?;

            let pid = child.id();
            *child_slot = Some(child);
            pid
        };

        tokio::time::sleep(Duration::from_millis(150)).await;
        let mut child_slot = self.child.lock().await;
        if !child_is_running(&mut child_slot) {
            bail!("kcptun client exited immediately after start; check kcptun.log");
        }
        logging!(info, Type::Core, "kcptun client started: {pid}");
        Ok(())
    }

    fn binary_path(&self, config: &IKcpProxy) -> Result<PathBuf> {
        if let Some(path) = config
            .client_path
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Ok(PathBuf::from(path));
        }

        let exe = current_exe()?;
        let Some(app_dir) = exe.parent() else {
            return Err(anyhow!("failed to resolve app directory"));
        };

        let bin_ext = if cfg!(windows) { ".exe" } else { "" };
        let path = app_dir.join(format!("kcptun-client{bin_ext}"));
        if path.exists() {
            return Ok(path);
        }

        if let Some(path) = find_kcptun_binary(app_dir) {
            return Ok(path);
        }

        if let Ok(resource_dir) = Handle::app_handle().path().resource_dir()
            && let Some(path) = find_kcptun_binary(&resource_dir)
        {
            return Ok(path);
        }

        Err(anyhow!(
            "kcptun client binary not found; set a custom path or place kcptun-client next to the app"
        ))
    }

    fn build_launch(&self, runtime: &KcptunRuntimeConfig) -> Result<KcptunLaunch> {
        let config = &runtime.config;
        let server = config
            .server
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("kcptun server is required"))?;
        let remote_port = nonzero_port(config.remote_port, 29900, "kcptun remote port")?;
        let local_port = nonzero_port(config.local_port, 1087, "kcptun local port")?;
        let key = config
            .key
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("kcptun key is required"))?;

        let log_dir = dirs::app_logs_dir()?;
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("failed to create log directory: {}", log_dir.display()))?;

        let mut args = vec![
            "-r".into(),
            format_endpoint(server, remote_port),
            "-l".into(),
            format_endpoint(&runtime.listen_host, local_port),
            "--mode".into(),
            string_or_default(config.mode.as_deref(), "fast"),
            "--crypt".into(),
            string_or_default(config.crypt.as_deref(), "aes"),
            "--mtu".into(),
            config.mtu.unwrap_or(1350).to_string(),
            "--sndwnd".into(),
            config.sndwnd.unwrap_or(512).to_string(),
            "--rcvwnd".into(),
            config.rcvwnd.unwrap_or(512).to_string(),
            "--datashard".into(),
            config.datashard.unwrap_or(10).to_string(),
            "--parityshard".into(),
            config.parityshard.unwrap_or(3).to_string(),
            "--dscp".into(),
            config.dscp.unwrap_or(0).to_string(),
            "--log".into(),
            dirs::path_to_str(&log_dir.join("kcptun.log"))?.into(),
        ];

        if config.nocomp.unwrap_or(true) {
            args.push("--nocomp".into());
        }

        if config.tcp.unwrap_or(false) {
            if !cfg!(target_os = "linux") {
                bail!(
                    "kcptun --tcp uses tcpraw and is only supported by Linux clients; disable tcp on this device. Non-Linux clients cannot communicate with a Linux kcptun server that requires --tcp/tcpraw."
                );
            }

            args.push("--tcp".into());
        }

        Ok(KcptunLaunch {
            args,
            key: key.to_string(),
        })
    }
}

struct KcptunRuntimeConfig {
    config: IKcpProxy,
    listen_host: String,
}

struct KcptunLaunch {
    args: Vec<String>,
    key: String,
}

async fn load_kcp_runtime_config() -> KcptunRuntimeConfig {
    let verge = Config::verge().await;
    let verge_arc = verge.latest_arc();
    let config = verge_arc.kcp_proxy.clone().unwrap_or_else(IKcpProxy::template);
    let proxy_host = verge_arc.proxy_host.clone();
    drop(verge_arc);
    drop(verge);

    let allow_lan = Config::clash()
        .await
        .latest_arc()
        .0
        .get("allow-lan")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    KcptunRuntimeConfig {
        config,
        listen_host: kcp_listen_host(allow_lan, proxy_host.as_deref()),
    }
}

fn child_is_running(child: &mut Option<Child>) -> bool {
    let Some(child_ref) = child.as_mut() else {
        return false;
    };

    match child_ref.try_wait() {
        Ok(None) => true,
        Ok(Some(status)) => {
            logging!(info, Type::Core, "kcptun client exited: {status}");
            *child = None;
            false
        }
        Err(err) => {
            logging!(warn, Type::Core, "Failed to query kcptun client: {err}");
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            false
        }
    }
}

fn nonzero_port(value: Option<u16>, fallback: u16, label: &str) -> Result<u16> {
    match value.unwrap_or(fallback) {
        0 => bail!("{label} must not be 0"),
        port => Ok(port),
    }
}

fn string_or_default(value: Option<&str>, fallback: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn format_endpoint(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn find_kcptun_binary(dir: &std::path::Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect::<Vec<_>>();

    for name in preferred_kcptun_binary_names() {
        if let Some(path) = entries
            .iter()
            .find(|path| path.file_name().and_then(|value| value.to_str()) == Some(name))
        {
            return Some(path.clone());
        }
    }

    entries.into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("kcptun-client") && name.contains(kcptun_platform_marker()))
    })
}

const fn preferred_kcptun_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &["kcptun-client-x86_64-apple-darwin", "kcptun-client"]
    }
    #[cfg(target_os = "windows")]
    {
        &["kcptun-client-x86_64-pc-windows-msvc.exe", "kcptun-client.exe"]
    }
    #[cfg(target_os = "linux")]
    {
        &["kcptun-client-x86_64-unknown-linux-gnu", "kcptun-client"]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        &["kcptun-client"]
    }
}

const fn kcptun_platform_marker() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "apple-darwin"
    }
    #[cfg(target_os = "windows")]
    {
        "pc-windows-msvc"
    }
    #[cfg(target_os = "linux")]
    {
        "unknown-linux-gnu"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "kcptun-client"
    }
}
