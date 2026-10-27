{ lib, ... }:
{
  options.services.orby = {
    enable = lib.mkEnableOption "Orby, an OrbStack NixOS builder (disabling purges its machine)";
    user = lib.mkOption {
      type = lib.types.str;
      default = "";
      example = "alice";
      description = "macOS account that owns the OrbStack machine and runs Orby's launchd service.";
    };
    orbctl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/opt/homebrew/bin/orbctl";
      description = "Override the OrbStack CLI with an absolute path. By default Orby uses the first orbctl in the machine owner's login PATH.";
    };
    cores = lib.mkOption {
      type = lib.types.nullOr lib.types.ints.positive;
      default = null;
      example = 8;
      description = "OrbStack machine CPU limit in cores; null leaves it unlimited. Nix still schedules at most 8 concurrent builds by default.";
    };
    memoryMiB = lib.mkOption {
      type = lib.types.nullOr lib.types.ints.positive;
      default = null;
      example = 6144;
      description = "OrbStack machine memory limit in MiB; null leaves it unlimited.";
    };
    diskSize = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "100G";
      description = "OrbStack machine disk limit in GiB, e.g. 100G; null leaves it unlimited.";
    };
    sharedDirectories = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        project = "/Users/alice/work";
      };
      description = "Explicit host mounts in the isolated machine, under /mnt/virtiofs/<name>. These expose host files to the guest.";
    };
    port = lib.mkOption {
      type = lib.types.port;
      default = 31222;
    };
    guestConfig = lib.mkOption {
      type = lib.types.lines;
      default = "";
      example = "environment.systemPackages = with pkgs; [ git ];";
      description = "Additional NixOS module declarations evaluated in the OrbStack guest. Changes are applied with nixos-rebuild; use only trusted input.";
    };
    rosetta = lib.mkEnableOption "x86_64-linux builds using OrbStack's Rosetta support";
    onDemand = {
      enable = lib.mkEnableOption "start the Orby machine when a build connects";
      ttl = lib.mkOption {
        type = lib.types.ints.positive;
        default = 180;
        description = "Idle time in minutes before the named machine is stopped.";
      };
    };
  };
}
