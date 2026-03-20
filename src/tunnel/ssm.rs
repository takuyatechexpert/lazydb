use anyhow::{bail, Context, Result};
use tokio::process::{Child, Command};

use super::wait_for_port;

pub struct SsmTunnel {
    child: Child,
}

impl SsmTunnel {
    pub async fn start(
        instance_id: &str,
        ssh_user: &str,
        ssh_key: Option<&str>,
        aws_profile: Option<&str>,
        remote_db_host: &str,
        remote_db_port: u16,
        local_port: u16,
    ) -> Result<Self> {
        let forward = format!(
            "{}:{}:{}",
            local_port, remote_db_host, remote_db_port
        );

        // ProxyCommand 構築
        let mut proxy_cmd =
            "aws ssm start-session --target %h --document-name AWS-StartSSHSession --parameters portNumber=%p"
                .to_string();
        if let Some(profile) = aws_profile {
            proxy_cmd = format!("{} --profile {}", proxy_cmd, profile);
        }

        let mut cmd = Command::new("ssh");
        cmd.arg("-L").arg(&forward)
            .arg("-N")
            .arg("-o").arg("StrictHostKeyChecking=no")
            .arg("-o").arg("ExitOnForwardFailure=yes")
            .arg("-o").arg("ServerAliveInterval=15")
            .arg("-o").arg(format!("ProxyCommand={}", proxy_cmd));

        if let Some(key) = ssh_key {
            cmd.arg("-i").arg(key);
        }

        cmd.arg(format!("{}@{}", ssh_user, instance_id));

        let child = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("SSM トンネルプロセスの起動に失敗しました")?;

        let mut tunnel = Self { child };

        if let Err(e) = wait_for_port(local_port).await {
            tunnel.kill().await;
            bail!("SSM トンネルのポート待機に失敗しました: {}", e);
        }

        Ok(tunnel)
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}
