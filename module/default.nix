{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.orby;
  state = "/var/lib/orby";
  key = "${state}/builder_ed25519";
  port = toString cfg.port;
  diskMatch = if cfg.diskSize == null then null else builtins.match "([1-9][0-9]*)G" cfg.diskSize;
  diskGiB = if diskMatch == null then null else lib.toInt (builtins.head diskMatch);
  runner = self.packages.${pkgs.stdenv.hostPlatform.system}.orby;
  runDaemon = pkgs.writeShellScript "orbyd-start" ''
    export HOME=$(/usr/bin/dscl . -read /Users/${cfg.user} NFSHomeDirectory | /usr/bin/awk '{print $2}')
    if [[ -z "$HOME" ]]; then
      echo "Orby: machine owner's home directory is missing" >&2
      exit 1
    fi
    exec ${lib.getExe runner}
  '';
  guestModule = pkgs.writeText "orby-guest-module.nix" ''
    { lib, pkgs, ... }: {
      services.openssh = {
        # OrbStack's stock NixOS module disables guest sshd in favor of its
        # built-in SSH proxy. Orby needs a separate sshd for Nix's ssh-ng
        # builder connection to the guest's own IP address.
        enable = lib.mkForce true;
        # NixOS 25.11 generates keys when sshd starts. OrbStack uses socket
        # activation, so activation must start sshd-keygen before reading one.
        hostKeys = lib.mkForce [ { type = "ed25519"; path = "/etc/ssh/ssh_host_ed25519_key"; } ];
        settings = {
          PasswordAuthentication = false;
          KbdInteractiveAuthentication = false;
          PermitRootLogin = "no";
        };
      };
      users.users.builder = {
        isNormalUser = true;
        openssh.authorizedKeys.keyFiles = [ /etc/nixos/orby/builder.pub ];
      };
      nix.settings.trusted-users = [ "builder" ];
      nix.settings.experimental-features = [ "nix-command" "flakes" ];
      nix.settings.auto-optimise-store = true;
      ${lib.optionalString cfg.rosetta ''nix.settings.extra-platforms = lib.mkAfter [ "x86_64-linux" ];''}
      # OrbStack runs its NixOS distribution as an LXC machine. Mounting
      # debugfs there requires privileges the machine does not have and makes
      # nixos-rebuild switch fail even though the builder itself is healthy.
      systemd.mounts = [ { where = "/sys/kernel/debug"; enable = false; } ];
      ${cfg.guestConfig}
    }
  '';
  guestWrapper = pkgs.writeText "orby-guest-configuration.nix" ''
    { ... }: {
      imports = [ /etc/nixos/configuration.nix /etc/nixos/orby/module.nix ];
    }
  '';
  activationConfig = pkgs.writeText "orby-activation.json" (
    builtins.toJSON {
      owner = cfg.user;
      orbctl = cfg.orbctl;
      cores = cfg.cores;
      memory_mib = cfg.memoryMiB;
      disk_gib = diskGiB;
      disk_size = cfg.diskSize;
      shared_directories = cfg.sharedDirectories;
      port = cfg.port;
      on_demand = cfg.onDemand.enable;
      ttl = cfg.onDemand.ttl * 60;
      rosetta = cfg.rosetta;
      guest_module = toString guestModule;
      guest_wrapper = toString guestWrapper;
      ssh = "${pkgs.openssh}/bin/ssh";
      ssh_keygen = "${pkgs.openssh}/bin/ssh-keygen";
    }
  );
  buildMachines = [
    {
      hostName = "orby-vm";
      maxJobs = if cfg.cores == null then 8 else cfg.cores;
      protocol = "ssh-ng";
      systems = [ "aarch64-linux" ] ++ lib.optional cfg.rosetta "x86_64-linux";
      supportedFeatures = [
        "benchmark"
        "big-parallel"
      ];
      speedFactor = 1;
    }
  ];
in
{
  imports = [ ./options.nix ];

  config = lib.mkMerge [
    (lib.mkIf cfg.enable {
      assertions = [
        {
          assertion = pkgs.stdenv.hostPlatform.system == "aarch64-darwin";
          message = "Orby requires Apple Silicon macOS.";
        }
        {
          assertion = builtins.match "[a-z_][a-z0-9_-]*" cfg.user != null;
          message = "services.orby.user must name the macOS user who owns OrbStack.";
        }
        {
          assertion = cfg.diskSize == null || diskMatch != null;
          message = "services.orby.diskSize must be a positive number of GiB, e.g. 100G.";
        }
        {
          assertion = cfg.memoryMiB == null || cfg.memoryMiB >= 1024;
          message = "services.orby.memoryMiB must be at least 1024.";
        }
        {
          assertion = cfg.orbctl == null || builtins.match "/.+" cfg.orbctl != null;
          message = "services.orby.orbctl must be an absolute path when overridden.";
        }
        {
          assertion =
            lib.all (tag: builtins.match "[a-zA-Z0-9_-]+" tag != null) (
              builtins.attrNames cfg.sharedDirectories
            )
            && lib.all (path: builtins.match "/[^,:]*" path != null) (
              builtins.attrValues cfg.sharedDirectories
            );
          message = "Orby shared directory tags must be simple names and host paths must be absolute without commas or colons.";
        }
      ];

      system.activationScripts.extraActivation.text = lib.mkAfter ''
        ${lib.getExe runner} activate ${activationConfig} || exit 1
      '';

      environment.etc."ssh/ssh_config.d/100-orby-vm.conf".text = ''
        Host orby-vm
          HostName localhost
          Port ${port}
          User builder
          IdentityFile ${key}
          IdentitiesOnly yes
          HostKeyAlias orby-vm-key
          GlobalKnownHostsFile ${state}/known_hosts
          UserKnownHostsFile /dev/null
          StrictHostKeyChecking yes
      '';

      launchd.daemons.orbyd = {
        command = toString runDaemon;
        serviceConfig = {
          UserName = cfg.user;
          KeepAlive = !cfg.onDemand.enable;
          StandardOutPath = "/tmp/orbyd.log";
          StandardErrorPath = "/tmp/orbyd.log";
          Sockets.Listener = {
            SockFamily = "IPv4";
            SockNodeName = "localhost";
            SockServiceName = port;
          };
        };
      };
    })

    (lib.mkIf (!cfg.enable) {
      system.activationScripts.postActivation.text = lib.mkBefore ''
        ${lib.getExe runner} purge || exit 1
      '';
    })

    (lib.mkIf cfg.enable {
      nix = {
        distributedBuilds = true;
        inherit buildMachines;
      };
    })
  ];
}
