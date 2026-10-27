use std::{
    io,
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{machine::Machine, Result};

// A single event loop owns machine state and the idle deadline. Workers only
// forward bytes and report completion; they never stop a machine themselves.
pub struct Proxy {
    pub machine: Machine,
    active: usize,
    idle_since: Option<Instant>,
}

impl Proxy {
    pub fn new(machine: Machine) -> Self {
        Self {
            machine,
            active: 0,
            idle_since: None,
        }
    }

    fn accepted(&mut self) {
        self.active += 1;
        self.idle_since = None;
    }

    fn released(&mut self) {
        self.active -= 1;
        if self.active == 0 && self.machine.config.on_demand {
            self.idle_since = Some(Instant::now());
        }
    }

    fn stop_if_idle(&mut self) {
        if self.active == 0 && self.machine.config.on_demand {
            if let Some(since) = self.idle_since {
                if since.elapsed() >= Duration::from_secs(self.machine.config.ttl) {
                    self.idle_since = None;
                    if let Err(err) = self.machine.stop() {
                        eprintln!("Orby: idle stop failed: {err}");
                    }
                }
            }
        }
    }

    pub fn serve(&mut self, listener: TcpListener, shutdown: &AtomicBool) -> Result<()> {
        if !self.machine.config.on_demand {
            self.machine.start()?;
        }
        listener.set_nonblocking(true)?;
        let (done, completed) = mpsc::channel();
        while !shutdown.load(Ordering::Relaxed) {
            while completed.try_recv().is_ok() {
                self.released();
            }
            // Accept before checking the idle deadline, so queued connections
            // cannot be overtaken by a stop on the same event-loop iteration.
            match listener.accept() {
                Ok((client, _)) => {
                    self.accepted();
                    match self.machine.start() {
                        Ok(address) => {
                            let done = done.clone();
                            thread::spawn(move || {
                                if let Err(err) = forward(client, address) {
                                    eprintln!("Orby: SSH forwarding failed: {err}");
                                }
                                let _ = done.send(());
                            });
                        }
                        Err(err) => {
                            eprintln!("Orby: machine start failed: {err}");
                            self.released();
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    self.stop_if_idle();
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(err.into()),
            }
        }
        // Give in-flight connections a bounded opportunity to finish after
        // launchd requests shutdown. If they remain, never stop the guest.
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.active > 0 && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if completed.recv_timeout(remaining).is_ok() {
                self.released();
            } else {
                break;
            }
        }
        if self.active == 0 && self.machine.config.on_demand {
            if let Err(err) = self.machine.stop() {
                eprintln!("Orby: shutdown stop failed: {err}");
            }
        }
        Ok(())
    }
}

fn forward(mut client: TcpStream, address: SocketAddr) -> io::Result<()> {
    // The launchd listener is nonblocking so the event loop can poll it.
    // Darwin may propagate that flag to accepted streams; io::copy expects
    // blocking sockets and otherwise fails with EAGAIN (os error 35).
    client.set_nonblocking(false)?;
    let mut upstream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    upstream.set_nonblocking(false)?;
    let mut client_reader = client.try_clone()?;
    let mut upstream_writer = upstream.try_clone()?;
    let outgoing = thread::spawn(move || {
        let result = io::copy(&mut client_reader, &mut upstream_writer);
        let _ = upstream_writer.shutdown(Shutdown::Write);
        result
    });
    let incoming = io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let outgoing = outgoing
        .join()
        .map_err(|_| io::Error::other("SSH forwarding thread panicked"))?;
    incoming?;
    outgoing?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunnerConfig;
    use std::io::{Read, Write};

    #[test]
    fn idle_stop_is_cancelled_by_new_connection() {
        let config = RunnerConfig {
            orbctl: "/bin/orbctl".into(),
            machine: "orby".into(),
            port: 31222,
            on_demand: true,
            ttl: 1,
        };
        let mut proxy = Proxy::new(Machine::new(config));
        proxy.accepted();
        proxy.released();
        assert!(proxy.idle_since.is_some());
        proxy.accepted();
        assert!(proxy.idle_since.is_none());
        proxy.stop_if_idle();
        assert_eq!(proxy.active, 1);
    }

    #[test]
    fn forwards_both_halves_until_eof() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = upstream.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = upstream.accept().unwrap();
            let mut input = String::new();
            stream.read_to_string(&mut input).unwrap();
            assert_eq!(input, "hello");
            stream.write_all(b"world").unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
        });
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (connection, _) = listener.accept().unwrap();
        connection.set_nonblocking(true).unwrap();
        let proxy = thread::spawn(move || forward(connection, address).unwrap());
        client.write_all(b"hello").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let mut output = String::new();
        client.read_to_string(&mut output).unwrap();
        assert_eq!(output, "world");
        proxy.join().unwrap();
        server.join().unwrap();
    }
}
