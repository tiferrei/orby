{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.orby;
  name = "orby-builder";
  state = "/var/lib/orby";
  runnerConfigFile = "${state}/runner.json";
  key = "${state}/builder_ed25519";
  port = toString cfg.port;
  diskMatch = builtins.match "([1-9][0-9]*)G" cfg.diskSize;
  diskGiB = if diskMatch == null then 0 else lib.toInt (builtins.head diskMatch);
  mounts = lib.mapAttrsToList (tag: path: "${path}:/mnt/virtiofs/${tag}") cfg.sharedDirectories;
  mountFlags = lib.concatMapStringsSep " " (mount: "--mount ${lib.escapeShellArg mount}") mounts;
  mountSetting = lib.concatStringsSep "," mounts;
  runner = self.packages.${pkgs.stdenv.hostPlatform.system}.orby-runner;
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
  runnerConfigTemplate = pkgs.writeText "orby-runner-template.json" (
    builtins.toJSON {
      machine = name;
      port = cfg.port;
      on-demand = cfg.onDemand.enable;
      ttl = cfg.onDemand.ttl * 60;
    }
  );
  # Activation scripts run as root; OrbStack's state belongs to the real macOS
  # user. Always run orbctl with that user's identity and home directory.
  orb = ''
    orb() {
      /usr/bin/sudo -H -u "$owner" /usr/bin/env HOME="$home" "$orbctl_bin" "$@"
    }
    get_home() {
      /usr/bin/dscl . -read "/Users/$1" NFSHomeDirectory | /usr/bin/awk '{print $2}'
    }
    # OrbStack's push command can succeed without placing files in isolated
    # machines. Transfer bounded base64 chunks via orbctl run instead. Chunks
    # are multiples of four so each can be decoded independently.
    install_guest_file() {
      local source="$1" destination="$2" encoded chunk temporary
      encoded=$(${pkgs.coreutils}/bin/base64 --wrap=0 "$source")
      temporary="$destination.tmp.$(/usr/bin/uuidgen)"
      # $1 and $2 are expanded by the shell inside the guest.
      # shellcheck disable=SC2016
      orb run -m ${name} -u root sh -c 'umask 077; : > "$1"' sh "$temporary"
      while [[ -n "$encoded" ]]; do
        chunk="''${encoded:0:4096}"
        encoded="''${encoded:4096}"
        # shellcheck disable=SC2016
        orb run -m ${name} -u root sh -c 'printf "%s" "$1" | base64 -d >> "$2"' sh "$chunk" "$temporary"
      done
      orb run -m ${name} -u root mv "$temporary" "$destination"
      orb run -m ${name} -u root chmod 644 "$destination"
    }
  '';
  resolveOrbctl =
    if cfg.orbctl == null then
      ''
        if ! orbctl_bin=$(/usr/bin/sudo -i -u "$owner" /bin/sh -c 'command -v orbctl'); then
          echo "Orby: orbctl not found in $owner's login PATH" >&2
          exit 1
        fi
      ''
    else
      "orbctl_bin=${lib.escapeShellArg cfg.orbctl}";
  buildMachines = [
    {
      hostName = "orby-vm";
      maxJobs = cfg.cores;
      protocol = "ssh-ng";
      systems = [ "aarch64-linux" ];
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
          assertion = diskMatch != null;
          message = "services.orby.diskSize must be a positive number of GiB, e.g. 100G.";
        }
        {
          assertion = cfg.memoryMiB >= 1024;
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
        set -euo pipefail
        owner=${lib.escapeShellArg cfg.user}
        ${orb}
        home=$(get_home "$owner")
        if [[ -z "$home" ]]; then
          echo "Orby: no home directory for $owner" >&2
          exit 1
        fi
        ${resolveOrbctl}
        if [[ "$orbctl_bin" != /* ]] || ! /usr/bin/sudo -H -u "$owner" /bin/sh -c 'test -x "$1"' sh "$orbctl_bin"; then
          echo "Orby: orbctl must resolve to an executable absolute path: $orbctl_bin" >&2
          exit 1
        fi
        /bin/mkdir -p ${state}
        # The unprivileged runner reads its configuration in this directory.
        # Private keys and the ownership marker themselves stay root-only.
        /bin/chmod 755 ${state}
        if [[ -f ${state}/owner ]]; then
          saved_owner=$(/bin/cat ${state}/owner)
          if [[ "$saved_owner" != "$owner" ]]; then
            echo "Orby: machine belongs to $saved_owner, not $owner" >&2
            exit 1
          fi
          if ! orb info ${name} >/dev/null; then
            echo "Orby: owned machine is missing; refusing automatic replacement" >&2
            exit 1
          fi
          orb start ${name}
          guest_identity=$(orb run -m ${name} -u root cat /etc/nixos/orby/identity 2>/dev/null || true)
          if [[ -z "$guest_identity" || "$guest_identity" != "$(/bin/cat ${state}/identity)" ]]; then
            echo "Orby: machine identity mismatch; refusing to manage ${name}" >&2
            exit 1
          fi
        else
          # Refuse to adopt a machine based on its name alone.
          if orb info ${name} >/dev/null 2>&1; then
            echo "Orby: ${name} already exists but is not owned by Orby" >&2
            exit 1
          fi
          /bin/echo "$owner" > ${state}/owner
          /bin/chmod 600 ${state}/owner
          orb create --isolated ${mountFlags} --cpus ${toString cfg.cores} \
            --memory ${toString (builtins.ceil (cfg.memoryMiB / 1024))}G \
            --disk ${lib.escapeShellArg cfg.diskSize} nixos ${name}
          /usr/bin/uuidgen > ${state}/identity
          /bin/chmod 600 ${state}/identity
          orb start ${name}
          orb run -m ${name} -u root mkdir -p /etc/nixos/orby
          # $1 expands in the guest shell, not in this activation script.
          # shellcheck disable=SC2016
          orb run -m ${name} -u root sh -c \
            'printf "%s\n" "$1" > /etc/nixos/orby/identity' sh "$(/bin/cat ${state}/identity)"
        fi
        # Resolve orbctl at activation, then publish the complete configuration
        # atomically so launchd never reads a partially written JSON file.
        config_tmp=$(/usr/bin/mktemp ${runnerConfigFile}.XXXXXX)
        if ! ${pkgs.jq}/bin/jq --arg orbctl "$orbctl_bin" \
          '.orbctl = $orbctl' ${runnerConfigTemplate} > "$config_tmp"; then
          /bin/rm -f "$config_tmp"
          exit 1
        fi
        /bin/chmod 644 "$config_tmp"
        /bin/mv -f "$config_tmp" ${runnerConfigFile}
        /bin/rm -f ${state}/orbctl-bin

        desired_resources="${toString cfg.cores}:${toString cfg.memoryMiB}:${toString diskGiB}"
        previous_resources=$(/bin/cat ${state}/resources 2>/dev/null || true)
        restart_needed=0
        if [[ "$previous_resources" != "$desired_resources" ]]; then
          if [[ -n "$previous_resources" ]]; then
            previous_disk=''${previous_resources##*:}
            if (( ${toString diskGiB} < previous_disk )); then
              echo "Orby: shrinking the managed disk is not supported" >&2
              exit 1
            fi
          fi
          orb config set machine.${name}.cpu ${toString cfg.cores}
          orb config set machine.${name}.memory_mib ${toString cfg.memoryMiB}
          orb config set machine.${name}.disk_bytes ${toString (diskGiB * 1024 * 1024 * 1024)}
          if [[ -n "$previous_resources" ]]; then
            restart_needed=1
          fi
        fi

        desired_mounts=${lib.escapeShellArg mountSetting}
        previous_mounts=$(/bin/cat ${state}/mounts 2>/dev/null || true)
        if [[ "$previous_mounts" != "$desired_mounts" ]]; then
          orb config set machine.${name}.mounts "$desired_mounts"
          if [[ -f ${state}/mounts ]]; then
            restart_needed=1
          fi
        fi
        if [[ "$restart_needed" == 1 ]]; then
          orb restart ${name}
        fi
        /bin/echo "$desired_resources" > ${state}/resources
        /bin/chmod 600 ${state}/resources
        /bin/echo "$desired_mounts" > ${state}/mounts
        /bin/chmod 600 ${state}/mounts

        if [[ ! -f ${key} ]]; then
          ${pkgs.openssh}/bin/ssh-keygen -q -t ed25519 -N "" -f ${key}
        fi
        /bin/chmod 600 ${key}

        # The OrbStack NixOS image is the base. Apply a small NixOS module in
        # the guest; never mutate its disk from the macOS host.
        orb start ${name}
        orb run -m ${name} -u root mkdir -p /etc/nixos/orby
        install_guest_file ${guestModule} /etc/nixos/orby/module.nix
        install_guest_file ${guestWrapper} /etc/nixos/orby/configuration.nix
        install_guest_file ${key}.pub /etc/nixos/orby/builder.pub
        # A previous switch may have left the unsupported debugfs mount failed.
        orb run -m ${name} -u root systemctl reset-failed sys-kernel-debug.mount 2>/dev/null || true
        orb run -m ${name} -u root /run/current-system/sw/bin/nixos-rebuild \
          switch -I nixos-config=/etc/nixos/orby/configuration.nix

        orb run -m ${name} -u root systemctl start sshd-keygen.service
        host_key=$(orb run -m ${name} -u root cat /etc/ssh/ssh_host_ed25519_key.pub)
        if [[ ! "$host_key" =~ ^ssh-ed25519[[:space:]] ]]; then
          echo "Orby: missing guest SSH host key" >&2
          exit 1
        fi
        /bin/echo "orby-vm-key $host_key" > ${state}/known_hosts
        /bin/chmod 644 ${state}/known_hosts

        guest_ip=$(orb run -m ${name} -u root hostname -I)
        for addr in $guest_ip; do
          if [[ "$addr" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            guest_ip="$addr"
            break
          fi
        done
        if [[ ! "$guest_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
          echo "Orby: no guest IPv4 address" >&2
          exit 1
        fi
        ${pkgs.openssh}/bin/ssh -i ${key} -o BatchMode=yes \
          -o ConnectTimeout=10 -o HostKeyAlias=orby-vm-key \
          -o UserKnownHostsFile=/dev/null -o GlobalKnownHostsFile=${state}/known_hosts \
          -o StrictHostKeyChecking=yes "builder@$guest_ip" nix-store --version
        ${lib.optionalString cfg.onDemand.enable "orb stop ${name}"}
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
        set -euo pipefail
        if [[ -f ${state}/owner ]]; then
          owner=$(/bin/cat ${state}/owner)
          if [[ ! "$owner" =~ ^[a-z_][a-z0-9_-]*$ ]]; then
            echo "Orby: invalid saved owner; refusing purge" >&2
            exit 1
          fi
          ${orb}
          if [[ ! -f ${runnerConfigFile} ]]; then
            echo "Orby: missing runner configuration; refusing purge" >&2
            exit 1
          fi
          if ! orbctl_bin=$(${pkgs.jq}/bin/jq -er \
            '.orbctl | select(type == "string" and startswith("/") and length > 1)' ${runnerConfigFile}); then
            echo "Orby: invalid saved orbctl path; refusing purge" >&2
            exit 1
          fi
          if [[ ! -x "$orbctl_bin" ]]; then
            echo "Orby: saved orbctl binary is missing: $orbctl_bin" >&2
            exit 1
          fi
          home=$(get_home "$owner")
          if [[ -z "$home" ]]; then
            echo "Orby: owner is missing; refusing purge" >&2
            exit 1
          fi
          # A failed deletion must not erase the only record of ownership.
          if [[ ! -f ${state}/identity ]] || ! orb info ${name} >/dev/null; then
            echo "Orby: missing machine identity or machine; refusing unverified purge" >&2
            exit 1
          fi
          orb start ${name}
          guest_identity=$(orb run -m ${name} -u root cat /etc/nixos/orby/identity 2>/dev/null || true)
          if [[ -z "$guest_identity" || "$guest_identity" != "$(/bin/cat ${state}/identity)" ]]; then
            echo "Orby: machine identity mismatch; refusing to delete ${name}" >&2
            exit 1
          fi
          orb delete -f ${name}
          /bin/rm -rf ${state}
        fi
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
