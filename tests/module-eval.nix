{ self, nixpkgs }:
let
  lib = nixpkgs.lib;
  pkgs = nixpkgs.legacyPackages.aarch64-darwin;
  stub = {
    options = {
      assertions = lib.mkOption {
        type = lib.types.listOf lib.types.anything;
        default = [ ];
      };
      system.activationScripts.extraActivation.text = lib.mkOption {
        type = lib.types.lines;
        default = "";
      };
      system.activationScripts.postActivation.text = lib.mkOption {
        type = lib.types.lines;
        default = "";
      };
      environment.etc = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };
      launchd.daemons = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };
      nix = lib.mkOption {
        type = lib.types.attrsOf lib.types.anything;
        default = { };
      };
    };
  };
  evaluate =
    cfg:
    (lib.evalModules {
      specialArgs = { inherit pkgs; };
      modules = [
        stub
        self.darwinModules.default
        { services.orby = cfg; }
      ];
    }).config;
  enabled = evaluate {
    enable = true;
    user = "alice";
  };
  overriddenCLI = evaluate {
    enable = true;
    user = "alice";
    orbctl = "/opt/homebrew/bin/orbctl";
  };
  invalidCLI = evaluate {
    enable = true;
    user = "alice";
    orbctl = "orbctl";
  };
  invalidDisk = evaluate {
    enable = true;
    user = "alice";
    diskSize = "nope";
  };
  disabled = evaluate { enable = false; };
in
assert builtins.all (x: x.assertion) enabled.assertions;
assert enabled.launchd.daemons ? orbyd;
assert lib.hasInfix "command -v orbctl" enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix "orbctl_bin=/opt/homebrew/bin/orbctl"
  overriddenCLI.system.activationScripts.extraActivation.text;
assert
  !(lib.hasInfix "command -v orbctl" overriddenCLI.system.activationScripts.extraActivation.text);
assert lib.any (x: !x.assertion && lib.hasInfix "orbctl" x.message) invalidCLI.assertions;
assert lib.hasInfix "orb create --isolated" enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix "orb run -m orby-builder -u root sh -c"
  enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix "install_guest_file" enabled.system.activationScripts.extraActivation.text;
assert !(lib.hasInfix "orb push -m" enabled.system.activationScripts.extraActivation.text);
assert lib.hasInfix "systemctl reset-failed sys-kernel-debug.mount"
  enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix "systemctl start sshd-keygen.service"
  enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix "orb config set machine.orby-builder.cpu"
  enabled.system.activationScripts.extraActivation.text;
assert
  enabled.nix.buildMachines == [
    {
      hostName = "orby-vm";
      maxJobs = 8;
      protocol = "ssh-ng";
      systems = [ "aarch64-linux" ];
      supportedFeatures = [
        "benchmark"
        "big-parallel"
      ];
      speedFactor = 1;
    }
  ];
assert !(disabled.nix ? buildMachines);
assert lib.any (x: !x.assertion && lib.hasInfix "diskSize" x.message) invalidDisk.assertions;
assert lib.hasInfix "orb delete -f orby-builder"
  disabled.system.activationScripts.postActivation.text;
true
