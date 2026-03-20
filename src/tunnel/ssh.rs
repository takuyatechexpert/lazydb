use anyhow::{bail, Context, Result};
use tokio::process::{Child, Command};

use super::wait_for_port;

pub struct SshTunnel {
    child: Child,
}

impl SshTunnel {
    /// SSH トンネルを起動する
    /// `ssh_user` が None の場合は ~/.ssh/config の User 設定に委譲する
    pub async fn start(
        ssh_host: &str,
        ssh_user: Option<&str>,
        remote_db_host: &str,
        remote_db_port: u16,
        local_port: u16,
    ) -> Result<Self> {
        let forward = format!(
            "{}:{}:{}",
            local_port, remote_db_host, remote_db_port
        );

        let destination = match ssh_user {
            Some(user) => format!("{}@{}", user, ssh_host),
            None => ssh_host.to_string(),
        };

        let child = Command::new("ssh")
            .arg("-L")
            .arg(&forward)
            .arg("-N")
            .arg("-o").arg("StrictHostKeyChecking=no")
            .arg("-o").arg("ExitOnForwardFailure=yes")
            .arg("-o").arg("ServerAliveInterval=15")
            .arg(&destination)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("SSH プロセスの起動に失敗しました")?;

        let mut tunnel = Self { child };

        if let Err(e) = wait_for_port(local_port).await {
            tunnel.kill().await;
            bail!("SSH トンネルのポート待機に失敗しました: {}", e);
        }

        Ok(tunnel)
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}
