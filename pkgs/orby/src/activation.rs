//! Privileged, activation-time reconciliation. The daemon never gets the
//! permissions needed to modify host credentials or delete a machine.
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;

use crate::{
    config::{first_ipv4, RunnerConfig, MACHINE, STATE},
    error, Result,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationConfig {
    owner: String,
    orbctl: Option<String>,
    cores: Option<u32>,
    memory_mib: Option<u64>,
    disk_gib: Option<u64>,
    disk_size: Option<String>,
    shared_directories: BTreeMap<String, String>,
    port: u16,
    on_demand: bool,
    ttl: u64,
    rosetta: bool,
    guest_module: String,
    guest_wrapper: String,
    ssh: String,
    ssh_keygen: String,
}

impl ActivationConfig {
    fn validate(&self) -> Result<()> {
        validate_owner(&self.owner)?;
        let disk_valid = match (&self.disk_gib, &self.disk_size) {
            (None, None) => true,
            (Some(disk), Some(size)) => *disk > 0 && *size == format!("{disk}G"),
            _ => false,
        };
        if self.cores == Some(0)
            || self.memory_mib.is_some_and(|memory| memory < 1024)
            || !disk_valid
            || self.port < 1024
            || self.ttl == 0
            || self
                .orbctl
                .as_ref()
                .is_some_and(|path| !Path::new(path).is_absolute())
            || !Path::new(&self.guest_module).is_absolute()
            || !Path::new(&self.guest_wrapper).is_absolute()
            || !Path::new(&self.ssh).is_absolute()
            || !Path::new(&self.ssh_keygen).is_absolute()
        {
            return Err(error("invalid Orby activation configuration"));
        }
        for (tag, path) in &self.shared_directories {
            if tag.is_empty()
                || !tag
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !Path::new(path).is_absolute()
                || path.contains([',', ':'])
            {
                return Err(error("invalid shared directory"));
            }
        }
        Ok(())
    }
}

// OrbStack uses zero for an unset per-machine resource limit. Fresh machines
// omit unset limits from `create`; existing machines clear them with `set`.
fn resource_settings(config: &ActivationConfig) -> Result<[(&'static str, String); 3]> {
    let disk_bytes = config
        .disk_gib
        .unwrap_or_default()
        .checked_mul(1024 * 1024 * 1024)
        .ok_or_else(|| error("disk size overflow"))?;
    Ok([
        (
            "machine.orby.cpu",
            config.cores.unwrap_or_default().to_string(),
        ),
        (
            "machine.orby.memory_mib",
            config.memory_mib.unwrap_or_default().to_string(),
        ),
        ("machine.orby.disk_bytes", disk_bytes.to_string()),
    ])
}

fn creation_resource_args(config: &ActivationConfig) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(cores) = config.cores {
        args.extend(["--cpus".into(), cores.to_string()]);
    }
    if let Some(memory) = config.memory_mib {
        args.extend(["--memory".into(), format!("{}G", memory.div_ceil(1024))]);
    }
    if let Some(disk) = &config.disk_size {
        args.extend(["--disk".into(), disk.clone()]);
    }
    args
}

fn validate_disk_change(previous: &str, requested: Option<u64>) -> Result<()> {
    if previous.is_empty() {
        return Ok(());
    }
    let disk: u64 = previous
        .trim()
        .rsplit(':')
        .next()
        .ok_or_else(|| error("invalid saved resources"))?
        .parse()?;
    if requested.is_some_and(|size| disk != 0 && size < disk) {
        return Err(error("shrinking the managed disk is not supported"));
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<()> {
    let mut bytes = owner.bytes();
    if !matches!(bytes.next(), Some(b'a'..=b'z' | b'_'))
        || !bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    {
        return Err(error("invalid saved OrbStack owner"));
    }
    Ok(())
}

fn root_only() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(error("Orby activation and purge must run as root"));
    }
    Ok(())
}

fn command(binary: &str, arguments: &[&str]) -> Result<String> {
    let output = Command::new(binary).args(arguments).output()?;
    if !output.status.success() {
        return Err(error(format!(
            "{binary} {:?}: {}: {}",
            arguments,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn optional_file(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn uuid() -> Result<String> {
    Ok(command("/usr/bin/uuidgen", &[])?.trim().to_owned())
}

fn home(owner: &str) -> Result<String> {
    let output = command(
        "/usr/bin/dscl",
        &[".", "-read", &format!("/Users/{owner}"), "NFSHomeDirectory"],
    )?;
    let home = output
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| error("machine owner has no home directory"))?;
    if !Path::new(home).is_absolute() {
        return Err(error("machine owner's home must be absolute"));
    }
    Ok(home.into())
}

fn discover_orbctl(owner: &str) -> Result<String> {
    let path = command(
        "/usr/bin/sudo",
        &["-i", "-u", owner, "/bin/sh", "-c", "command -v orbctl"],
    )?;
    let path = path.trim().to_owned();
    if !Path::new(&path).is_absolute() {
        return Err(error(
            "orbctl not found as an absolute path in owner's login PATH",
        ));
    }
    Ok(path)
}

struct Orb {
    owner: String,
    home: String,
    binary: String,
}

impl Orb {
    fn new(owner: String, binary: String) -> Result<Self> {
        validate_owner(&owner)?;
        if !Path::new(&binary).is_absolute() || !Path::new(&binary).is_file() {
            return Err(error("orbctl must be an executable absolute file"));
        }
        let home = home(&owner)?;
        let orb = Self {
            owner,
            home,
            binary,
        };
        orb.as_owner("/bin/sh", &["-c", "test -x \"$1\"", "sh", &orb.binary])?;
        Ok(orb)
    }

    fn owner_command(&self, binary: &str) -> Command {
        let mut cmd = Command::new("/usr/bin/sudo");
        cmd.args(["-H", "-u", &self.owner, "/usr/bin/env"])
            .arg(format!("HOME={}", self.home))
            .arg(binary);
        cmd
    }

    fn as_owner(&self, binary: &str, args: &[&str]) -> Result<String> {
        let output = self.owner_command(binary).args(args).output()?;
        if !output.status.success() {
            return Err(error(format!(
                "{binary} {:?}: {}: {}",
                args,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    fn call(&self, args: &[&str]) -> Result<String> {
        self.as_owner(&self.binary, args)
    }

    fn call_verbose(&self, args: &[&str]) -> Result<()> {
        let status = self
            .owner_command(&self.binary)
            .args(args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()?;
        if !status.success() {
            return Err(error(format!("orbctl {:?} failed: {status}", args)));
        }
        Ok(())
    }

    fn verify_owned(&self, action: &str) -> Result<()> {
        let identity = fs::read_to_string(format!("{STATE}/identity"))
            .map_err(|_| error(format!("missing identity; refusing to {action} {MACHINE}")))?;
        self.call(&["info", MACHINE])?;
        self.call(&["start", MACHINE])?;
        let guest = self.call(&[
            "run",
            "-m",
            MACHINE,
            "-u",
            "root",
            "cat",
            "/etc/nixos/orby/identity",
        ])?;
        if identity.trim().is_empty() || guest.trim() != identity.trim() {
            return Err(error(format!(
                "machine identity mismatch; refusing to {action} {MACHINE}"
            )));
        }
        Ok(())
    }

    fn guest(&self, args: &[&str]) -> Result<String> {
        let mut command = vec!["run", "-m", MACHINE, "-u", "root"];
        command.extend_from_slice(args);
        self.call(&command)
    }

    fn install_file(&self, source: &str, destination: &str) -> Result<()> {
        let encoded = STANDARD.encode(fs::read(source)?);
        let temporary = format!("{destination}.tmp.{}", uuid()?);
        self.guest(&["sh", "-c", "umask 077; : > \"$1\"", "sh", &temporary])?;
        for chunk in encoded.as_bytes().chunks(4096) {
            let chunk = std::str::from_utf8(chunk)?;
            self.guest(&[
                "sh",
                "-c",
                "printf '%s' \"$1\" | base64 -d >> \"$2\"",
                "sh",
                chunk,
                &temporary,
            ])?;
        }
        self.guest(&["mv", &temporary, destination])?;
        self.guest(&["chmod", "644", destination])?;
        Ok(())
    }
}

fn private_write(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn atomic_write(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("tmp.{}", uuid()?));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn activate(path: &Path) -> Result<()> {
    root_only()?;
    let config: ActivationConfig = serde_json::from_slice(&fs::read(path)?)?;
    config.validate()?;
    let binary = match &config.orbctl {
        Some(path) => path.clone(),
        None => discover_orbctl(&config.owner)?,
    };
    let orb = Orb::new(config.owner.clone(), binary.clone())?;
    if config.rosetta && orb.call(&["config", "get", "rosetta"])?.trim() != "true" {
        return Err(error(
            "enable Rosetta in OrbStack before enabling services.orby.rosetta",
        ));
    }

    fs::create_dir_all(STATE)?;
    fs::set_permissions(STATE, fs::Permissions::from_mode(0o755))?;
    let owner_path = Path::new(STATE).join("owner");
    let mut created = false;
    if owner_path.exists() {
        if fs::read_to_string(&owner_path)?.trim() != config.owner {
            return Err(error("machine belongs to a different owner"));
        }
        orb.verify_owned("manage")?;
    } else {
        if orb.call(&["info", MACHINE]).is_ok() {
            return Err(error("orby already exists but is not owned by Orby"));
        }
        private_write(&owner_path, &format!("{}\n", config.owner))?;
        let mut create = vec!["create", "--isolated"];
        let mounts = config
            .shared_directories
            .iter()
            .map(|(tag, path)| format!("{path}:/mnt/virtiofs/{tag}"))
            .collect::<Vec<_>>();
        for mount in &mounts {
            create.extend(["--mount", mount]);
        }
        let limits = creation_resource_args(&config);
        create.extend(limits.iter().map(String::as_str));
        create.extend(["nixos", MACHINE]);
        orb.call_verbose(&create)?;
        created = true;
        let identity = uuid()?;
        private_write(&Path::new(STATE).join("identity"), &format!("{identity}\n"))?;
        orb.call(&["start", MACHINE])?;
        orb.guest(&["mkdir", "-p", "/etc/nixos/orby"])?;
        orb.guest(&[
            "sh",
            "-c",
            "printf '%s\\n' \"$1\" > /etc/nixos/orby/identity",
            "sh",
            &identity,
        ])?;
    }

    let runner = RunnerConfig {
        orbctl: binary,
        machine: MACHINE.into(),
        port: config.port,
        on_demand: config.on_demand,
        ttl: config.ttl,
    };
    runner.validate()?;
    atomic_write(
        &Path::new(STATE).join("runner.json"),
        &serde_json::to_vec(&runner)?,
        0o644,
    )?;
    if Path::new(STATE).join("orbctl-bin").exists() {
        fs::remove_file(Path::new(STATE).join("orbctl-bin"))?;
    }

    // Keep the old colon-separated state format; zero represents no limit.
    let resources = format!(
        "{}:{}:{}",
        config.cores.unwrap_or_default(),
        config.memory_mib.unwrap_or_default(),
        config.disk_gib.unwrap_or_default()
    );
    let previous = optional_file(&Path::new(STATE).join("resources"))?.unwrap_or_default();
    let mut restart = false;
    if previous.trim() != resources {
        validate_disk_change(&previous, config.disk_gib)?;
        // On creation, don't set zero for limits omitted from `create`, but
        // apply explicit settings (including exact MiB after the rounded
        // --memory flag). Existing machines must clear omitted limits too.
        for (key, value) in resource_settings(&config)? {
            if !created || value != "0" {
                orb.call(&["config", "set", key, &value])?;
            }
        }
        if !created {
            restart = true;
        }
    }
    let mounts = config
        .shared_directories
        .iter()
        .map(|(tag, path)| format!("{path}:/mnt/virtiofs/{tag}"))
        .collect::<Vec<_>>()
        .join(",");
    let previous_mounts = optional_file(&Path::new(STATE).join("mounts"))?;
    if previous_mounts.as_deref().map(str::trim_end) != Some(mounts.as_str()) {
        orb.call(&["config", "set", "machine.orby.mounts", &mounts])?;
        restart |= previous_mounts.is_some();
    }
    if restart {
        orb.call(&["restart", MACHINE])?;
    }
    private_write(
        &Path::new(STATE).join("resources"),
        &format!("{resources}\n"),
    )?;
    private_write(&Path::new(STATE).join("mounts"), &format!("{mounts}\n"))?;

    let key = Path::new(STATE).join("builder_ed25519");
    if !key.exists() {
        command(
            &config.ssh_keygen,
            &[
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-f",
                key.to_str().ok_or_else(|| error("invalid SSH key path"))?,
            ],
        )?;
    }
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600))?;
    orb.call(&["start", MACHINE])?;
    orb.guest(&["mkdir", "-p", "/etc/nixos/orby"])?;
    orb.install_file(&config.guest_module, "/etc/nixos/orby/module.nix")?;
    orb.install_file(&config.guest_wrapper, "/etc/nixos/orby/configuration.nix")?;
    orb.install_file(
        &format!("{}.pub", key.display()),
        "/etc/nixos/orby/builder.pub",
    )?;
    let _ = orb.guest(&["systemctl", "reset-failed", "sys-kernel-debug.mount"]);
    orb.call_verbose(&[
        "run",
        "-m",
        MACHINE,
        "-u",
        "root",
        "/run/current-system/sw/bin/nixos-rebuild",
        "switch",
        "-I",
        "nixos-config=/etc/nixos/orby/configuration.nix",
    ])?;
    orb.guest(&["systemctl", "start", "sshd-keygen.service"])?;
    let host_key = orb.guest(&["cat", "/etc/ssh/ssh_host_ed25519_key.pub"])?;
    if !host_key.starts_with("ssh-ed25519 ") {
        return Err(error("missing guest SSH host key"));
    }
    let known_hosts = Path::new(STATE).join("known_hosts");
    atomic_write(
        &known_hosts,
        format!("orby-vm-key {}\n", host_key.trim()).as_bytes(),
        0o644,
    )?;
    let ip = first_ipv4(&orb.guest(&["hostname", "-I"])?)?;
    let key = key.to_str().ok_or_else(|| error("invalid SSH key path"))?;
    command(
        &config.ssh,
        &[
            "-i",
            key,
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "HostKeyAlias=orby-vm-key",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            &format!("GlobalKnownHostsFile={}", known_hosts.display()),
            "-o",
            "StrictHostKeyChecking=yes",
            &format!("builder@{ip}"),
            "nix-store",
            "--version",
        ],
    )?;
    if config.on_demand {
        orb.call(&["stop", MACHINE])?;
    }
    Ok(())
}

pub fn purge() -> Result<()> {
    root_only()?;
    let owner_path = Path::new(STATE).join("owner");
    if !owner_path.exists() {
        return Ok(());
    }
    let owner = fs::read_to_string(owner_path)?.trim().to_owned();
    validate_owner(&owner)?;
    // Read the exact format deployed by the old Go runner. Never infer the
    // OrbStack owner or binary from the current nix-darwin configuration.
    let runner = RunnerConfig::read(&PathBuf::from(STATE).join("runner.json"))?;
    let orb = Orb::new(owner, runner.orbctl)?;
    orb.verify_owned("delete")?;
    orb.call(&["delete", "-f", MACHINE])?;
    fs::remove_dir_all(STATE)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_owner_never_reaches_sudo() {
        for invalid in ["", "Root", "a;rm", "a/b", "a\nb"] {
            assert!(validate_owner(invalid).is_err());
        }
        validate_owner("a_builder-1").unwrap();
    }

    #[test]
    fn activation_config_rejects_invalid_mounts_and_cli() {
        let mut config: ActivationConfig = serde_json::from_str(
            r#"{
            "owner":"alice", "orbctl":null, "cores":8, "memory_mib":6144,
            "disk_gib":100, "disk_size":"100G", "shared_directories":{},
            "port":31222, "on_demand":true, "ttl":900, "rosetta":false,
            "guest_module":"/nix/store/guest/module.nix",
            "guest_wrapper":"/nix/store/guest/configuration.nix",
            "ssh":"/nix/store/ssh", "ssh_keygen":"/nix/store/ssh-keygen"
        }"#,
        )
        .unwrap();
        config.validate().unwrap();
        config
            .shared_directories
            .insert("project".into(), "/Users/alice/work".into());
        config.validate().unwrap();
        config
            .shared_directories
            .insert("bad:tag".into(), "/Users/alice/work".into());
        assert!(config.validate().is_err());
        config.shared_directories.remove("bad:tag");
        config.orbctl = Some("orbctl".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn resource_limits_can_be_independently_unset() {
        let mut config: ActivationConfig = serde_json::from_str(
            r#"{
            "owner":"alice", "orbctl":null, "cores":null, "memory_mib":null,
            "disk_gib":null, "disk_size":null, "shared_directories":{},
            "port":31222, "on_demand":true, "ttl":900, "rosetta":false,
            "guest_module":"/nix/store/guest/module.nix",
            "guest_wrapper":"/nix/store/guest/configuration.nix",
            "ssh":"/nix/store/ssh", "ssh_keygen":"/nix/store/ssh-keygen"
        }"#,
        )
        .unwrap();
        config.validate().unwrap();
        assert!(creation_resource_args(&config).is_empty());
        assert_eq!(
            resource_settings(&config).unwrap(),
            [
                ("machine.orby.cpu", "0".into()),
                ("machine.orby.memory_mib", "0".into()),
                ("machine.orby.disk_bytes", "0".into()),
            ]
        );

        config.cores = Some(4);
        config.disk_gib = Some(100);
        config.disk_size = Some("100G".into());
        config.validate().unwrap();
        assert_eq!(
            creation_resource_args(&config),
            ["--cpus", "4", "--disk", "100G"]
        );
        config.memory_mib = Some(1536);
        config.validate().unwrap();
        assert_eq!(
            creation_resource_args(&config),
            ["--cpus", "4", "--memory", "2G", "--disk", "100G"]
        );
        config.memory_mib = None;
        assert_eq!(
            resource_settings(&config).unwrap(),
            [
                ("machine.orby.cpu", "4".into()),
                ("machine.orby.memory_mib", "0".into()),
                ("machine.orby.disk_bytes", "107374182400".into()),
            ]
        );
        config.disk_size = None;
        assert!(config.validate().is_err());
        config.disk_gib = None;
        config.memory_mib = Some(512);
        assert!(config.validate().is_err());
        config.memory_mib = None;
        config.cores = Some(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn old_disk_limits_can_be_cleared_without_shrinking() {
        validate_disk_change("8:6144:100\n", None).unwrap();
        validate_disk_change("8:6144:100\n", Some(100)).unwrap();
        assert!(validate_disk_change("8:6144:100\n", Some(99)).is_err());
        validate_disk_change("0:0:0\n", Some(100)).unwrap();
    }
}
