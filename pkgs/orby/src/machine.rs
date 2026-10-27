use std::{
    io::Read,
    net::{SocketAddr, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    config::{first_ipv4, RunnerConfig},
    error, Result,
};

// Run orbctl directly; there is only one production implementation. Drain both
// pipes concurrently so a verbose command cannot block waiting for us to read.
fn call(binary: &str, arguments: &[&str]) -> Result<String> {
    let mut child = Command::new(binary)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| error("missing orbctl stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| error("missing orbctl stderr"))?;
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(error(format!("orbctl {arguments:?} timed out")));
        }
        thread::sleep(Duration::from_millis(50));
    };
    let output = out
        .join()
        .map_err(|_| error("orbctl stdout reader panicked"))??;
    let errors = err
        .join()
        .map_err(|_| error("orbctl stderr reader panicked"))??;
    if !status.success() {
        return Err(error(format!(
            "orbctl {:?}: {}: {}",
            arguments,
            status,
            String::from_utf8_lossy(&errors).trim()
        )));
    }
    Ok(String::from_utf8(output)?)
}

fn ssh_ready(address: SocketAddr) -> bool {
    TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok()
}

// The proxy alone owns this controller. Start and stop are serialized, so an
// idle stop cannot race a connection that has already been accepted.
pub struct Machine {
    pub config: RunnerConfig,
    address: Option<SocketAddr>,
}

impl Machine {
    pub fn new(config: RunnerConfig) -> Self {
        Self {
            config,
            address: None,
        }
    }

    pub fn start(&mut self) -> Result<SocketAddr> {
        if let Some(address) = self.address {
            if ssh_ready(address) {
                return Ok(address);
            }
            self.address = None;
        }
        call(&self.config.orbctl, &["start", &self.config.machine])?;
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let last_error = match call(
                &self.config.orbctl,
                &[
                    "run",
                    "-m",
                    &self.config.machine,
                    "-u",
                    "root",
                    "hostname",
                    "-I",
                ],
            )
            .and_then(|output| first_ipv4(&output))
            {
                Ok(ip) => {
                    let address = SocketAddr::from((ip, 22));
                    if ssh_ready(address) {
                        self.address = Some(address);
                        return Ok(address);
                    }
                    format!("SSH not ready at {address}")
                }
                Err(err) => err.to_string(),
            };
            if Instant::now() >= deadline {
                return Err(error(format!(
                    "SSH not ready after 90 seconds: {last_error}"
                )));
            }
            thread::sleep(Duration::from_secs(1));
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        // Discard the address even if stop fails: the guest may have changed IP.
        self.address = None;
        call(&self.config.orbctl, &["stop", &self.config.machine])?;
        Ok(())
    }
}
