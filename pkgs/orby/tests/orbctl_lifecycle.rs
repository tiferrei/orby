#![cfg(target_os = "macos")]

use std::{
    io::{BufRead, BufReader},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use orby::{
    config::{first_ipv4, RunnerConfig, STATE},
    machine::Machine,
};

fn orb(binary: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "orbctl {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

struct Fixture {
    binary: String,
    name: String,
    identity: String,
    marked: bool,
    cleaned: bool,
}

impl Fixture {
    fn cleanup(&self) -> Result<(), String> {
        orb(&self.binary, &["start", &self.name])?;
        let identity = orb(
            &self.binary,
            &[
                "run",
                "-m",
                &self.name,
                "-u",
                "root",
                "cat",
                "/etc/orby-test-identity",
            ],
        )?;
        if identity.trim() != self.identity {
            return Err(format!(
                "identity mismatch: refusing to delete {}",
                self.name
            ));
        }
        orb(&self.binary, &["delete", "-f", &self.name])?;
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if !self.marked {
            eprintln!("unverified test machine {}; inspect it manually", self.name);
        } else if let Err(err) = self.cleanup() {
            eprintln!("test machine {} was not deleted: {err}", self.name);
        }
    }
}

// Runs on macOS with OrbStack; never touches the production 'orby' machine.
// Keep an unverifiable fixture for manual inspection rather than deleting by name.
#[test]
fn isolated_machine_lifecycle() {
    let binary = String::from_utf8_lossy(
        &Command::new("/bin/sh")
            .args(["-c", "command -v orbctl"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_owned();
    assert!(!binary.is_empty(), "orbctl must be in PATH");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let name = format!("orby-test-{nonce}");
    assert!(
        orb(&binary, &["info", &name]).is_err(),
        "refusing to adopt {name}"
    );
    eprintln!("creating disposable machine {name}; first run may download NixOS");
    orb(
        &binary,
        &[
            "create",
            "--isolated",
            "--cpus",
            "1",
            "--memory",
            "2G",
            "--disk",
            "20G",
            "nixos",
            &name,
        ],
    )
    .unwrap();
    let mut fixture = Fixture {
        binary: binary.clone(),
        name: name.clone(),
        identity: nonce.clone(),
        marked: false,
        cleaned: false,
    };
    orb(&binary, &["start", &name]).unwrap();
    orb(
        &binary,
        &[
            "run",
            "-m",
            &name,
            "-u",
            "root",
            "sh",
            "-c",
            "printf '%s\\n' \"$1\" > /etc/orby-test-identity",
            "sh",
            &nonce,
        ],
    )
    .unwrap();
    fixture.marked = true;
    let mut machine = Machine::new(RunnerConfig {
        orbctl: binary.clone(),
        machine: name.clone(),
        port: 31222,
        on_demand: false,
        ttl: 900,
    });
    machine.stop().unwrap();
    orb(&binary, &["start", &name]).unwrap();
    let identity = orb(
        &binary,
        &[
            "run",
            "-m",
            &name,
            "-u",
            "root",
            "cat",
            "/etc/orby-test-identity",
        ],
    )
    .unwrap();
    assert_eq!(
        identity.trim(),
        nonce,
        "refusing to delete unverified {name}"
    );
    fixture.cleanup().unwrap();
    fixture.cleaned = true;
}

// Exercise the installed launchd socket and controller with real SSH traffic.
// This may wake the production machine; it never stops or deletes it.
#[test]
fn installed_builder_and_proxy() {
    let config = RunnerConfig::read(&Path::new(STATE).join("runner.json")).unwrap();
    let listener = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let client = TcpStream::connect_timeout(&listener, Duration::from_secs(5)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(120)))
        .unwrap();
    let mut banner = String::new();
    BufReader::new(client).read_line(&mut banner).unwrap();
    assert!(
        banner.starts_with("SSH-2.0-"),
        "unexpected SSH banner: {banner:?}"
    );

    let mut machine = Machine::new(config.clone());
    let address = machine.start().unwrap();
    let expected = first_ipv4(
        &orb(
            &config.orbctl,
            &["run", "-m", &config.machine, "-u", "root", "hostname", "-I"],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(address.ip(), expected);
    assert_eq!(address.port(), 22);
    assert_eq!(machine.start().unwrap(), address);
}
