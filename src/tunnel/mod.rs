pub mod ssh;
pub mod ssm;

use std::future::Future;

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

/// 旧トンネルの終了を待ってから、新しいトンネルを起動する。
///
/// 終了を待たずに起動すると、同じ local_port でまだ待ち受けている旧トンネルに
/// `wait_for_port` が繋がり、新しいトンネルの確立と取り違える。
pub async fn replace<F>(old: Option<Tunnel>, start: F) -> Result<Tunnel>
where
    F: Future<Output = Result<Tunnel>>,
{
    if let Some(mut old) = old {
        old.kill().await;
    }
    start.await
}

/// トンネルを起動する前に、ローカルポートで何も待ち受けていないことを確かめる。
///
/// `wait_for_port` は接続できたことだけで確立とみなすので、旧トンネルや無関係なサービスが
/// 先に待ち受けていると、そこへ繋いだまま成功扱いになる。判定も `wait_for_port` と同じ
/// 127.0.0.1 への connect で行う（bind での確認は、macOS では 0.0.0.0 や IPv6 で
/// 待ち受けている相手を見逃す）。
pub async fn ensure_port_free(port: u16) -> Result<()> {
    if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
        bail!(
            "ローカルポート {} はすでに使われています。別のトンネルやサービスを止めるか、local_port を変えてください",
            port
        );
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn ensure_port_free_rejects_port_with_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(ensure_port_free(port).await.is_err());

        drop(listener);
        assert!(ensure_port_free(port).await.is_ok());
    }

    #[tokio::test]
    async fn replace_waits_for_old_tunnel_to_exit_before_starting() {
        let child = tokio::process::Command::new("sleep")
            .arg("30")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap().to_string();
        let old = Tunnel::Ssh(SshTunnel::from_child(child));

        let old_alive_at_start = AtomicBool::new(true);
        let start = async {
            // kill -0 は対象プロセスが残っていれば成功する
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            old_alive_at_start.store(alive, Ordering::SeqCst);
            bail!("起動処理の代わり")
        };
        let result = tokio::time::timeout(Duration::from_secs(5), replace(Some(old), start)).await;

        assert!(result.expect("replace がタイムアウトした").is_err());
        assert!(!old_alive_at_start.load(Ordering::SeqCst));
    }
}
