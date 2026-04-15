use anyhow::{Context, Result, anyhow, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::tracker::update::Update;

/// Minimal client for the `rotctld` TCP protocol.
///
/// See https://manpages.ubuntu.com/manpages/xenial/man8/rotctld.8.html
pub struct RotctlClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl RotctlClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .with_context(|| format!("connecting to rotctld at {addr}"))?;
        let (r, w) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(r),
            writer: w,
        })
    }

    /// `P <az> <el>` — point the rotator at the given azimuth/elevation (degrees).
    pub async fn set_position(&mut self, azimuth_deg: f64, elevation_deg: f64) -> Result<()> {
        let cmd = format!("P {azimuth_deg:.2} {elevation_deg:.2}\n");
        self.writer.write_all(cmd.as_bytes()).await?;
        self.expect_rprt_ok().await
    }

    /// `p` — request the current azimuth and elevation (degrees).
    pub async fn get_position(&mut self) -> Result<(f64, f64)> {
        self.writer.write_all(b"p\n").await?;
        let az = self.read_line().await?;

        // If the rig reports an error, it replies with a single `RPRT -n` line
        // instead of the two position lines.
        if let Some(rest) = az.strip_prefix("RPRT ") {
            bail!("rotctld error on `p`: {}", rest);
        }

        let el = self.read_line().await?;
        let az: f64 = az
            .parse()
            .with_context(|| format!("parsing azimuth '{az}'"))?;
        let el: f64 = el
            .parse()
            .with_context(|| format!("parsing elevation '{el}'"))?;
        Ok((az, el))
    }

    /// `K` — park the rotator.
    pub async fn park(&mut self) -> Result<()> {
        self.writer.write_all(b"K\n").await?;
        self.expect_rprt_ok().await
    }

    /// Read a single `RPRT <code>` reply and fail on non-zero codes.
    async fn expect_rprt_ok(&mut self) -> Result<()> {
        let line = self.read_line().await?;
        let code = line
            .strip_prefix("RPRT ")
            .ok_or_else(|| anyhow!("expected 'RPRT <code>', got '{line}'"))?
            .parse::<i32>()
            .with_context(|| format!("parsing RPRT code '{line}'"))?;
        if code == 0 {
            Ok(())
        } else {
            Err(anyhow!("rotctld returned error code {code}"))
        }
    }

    async fn read_line(&mut self) -> Result<String> {
        let mut buf = String::new();
        let n = self.reader.read_line(&mut buf).await?;
        if n == 0 {
            bail!("rotctld connection closed");
        }
        Ok(buf.trim_end_matches(['\r', '\n']).to_string())
    }
}

/// Task that forwards tracker updates to a `rotctld` server.
pub async fn run(addr: String, mut updates: broadcast::Receiver<Update>) {
    let mut client = match RotctlClient::connect(&addr).await {
        Ok(c) => c,
        Err(e) => {
            error!(%addr, ?e, "failed to connect to rotctld");
            return;
        }
    };
    info!(%addr, "connected to rotctld");

    loop {
        match updates.recv().await {
            Ok(update) => {
                if let Err(e) = client
                    .set_position(update.azimuth_degrees, update.elevation_degrees)
                    .await
                {
                    error!(?e, "rotctld set_position failed");
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                warn!("rotctld task lagging, skipped {n} updates");
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    if let Err(e) = client.park().await {
        warn!(?e, "rotctld park failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// Spawn a fake rotctld that runs a single scripted exchange and returns
    /// the bytes the client sent.
    async fn fake_rotctld(
        script: Vec<&'static [u8]>,
    ) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut chunk = [0u8; 128];
            for reply in script {
                // Read one command (a line ending in '\n') from the client.
                loop {
                    let n = sock.read(&mut chunk).await.unwrap();
                    received.extend_from_slice(&chunk[..n]);
                    if received.contains(&b'\n') || n == 0 {
                        break;
                    }
                }
                sock.write_all(reply).await.unwrap();
            }
            received
        });

        (addr, handle)
    }

    #[tokio::test]
    async fn set_position_ok() {
        let (addr, handle) = fake_rotctld(vec![b"RPRT 0\n"]).await;
        let mut c = RotctlClient::connect(&addr).await.unwrap();
        c.set_position(123.45, 67.89).await.unwrap();
        drop(c);

        let sent = handle.await.unwrap();
        assert_eq!(sent, b"P 123.45 67.89\n");
    }

    #[tokio::test]
    async fn set_position_error() {
        let (addr, _handle) = fake_rotctld(vec![b"RPRT -1\n"]).await;
        let mut c = RotctlClient::connect(&addr).await.unwrap();
        assert!(c.set_position(0.0, 0.0).await.is_err());
    }

    #[tokio::test]
    async fn get_position_ok() {
        let (addr, _handle) = fake_rotctld(vec![b"180.00\n45.50\n"]).await;
        let mut c = RotctlClient::connect(&addr).await.unwrap();
        let (az, el) = c.get_position().await.unwrap();
        assert_eq!(az, 180.0);
        assert_eq!(el, 45.5);
    }

    #[tokio::test]
    async fn get_position_error() {
        let (addr, _handle) = fake_rotctld(vec![b"RPRT -6\n"]).await;
        let mut c = RotctlClient::connect(&addr).await.unwrap();
        assert!(c.get_position().await.is_err());
    }

    #[tokio::test]
    async fn park_ok() {
        let (addr, handle) = fake_rotctld(vec![b"RPRT 0\n"]).await;
        let mut c = RotctlClient::connect(&addr).await.unwrap();
        c.park().await.unwrap();
        drop(c);

        let sent = handle.await.unwrap();
        assert_eq!(sent, b"K\n");
    }
}
