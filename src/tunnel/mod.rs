pub mod ssh;
pub mod ssm;

use anyhow::{bail, Result};
use ssh::SshTunnel;
use ssm::SsmTunnel;
use tokio::net::TcpStream;

pub enum Tunnel {
    Ssh(SshTunnel),
    Ssm(SsmTunnel),
}

impl Tunnel {
    pub async fn kill(&mut self) {
        match self {
            Tunnel::Ssh(t) => t.kill().await,
            Tunnel::Ssm(t) => t.kill().await,
        }
    }
}

/// ローカルポートが開くまで待機（500ms 間隔、最大 5 秒）
pub async fn wait_for_port(port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    for _ in 0..10 {
        if TcpStream::connect(&addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    bail!("ポート {} への接続がタイムアウトしました（5秒）", port);
}
