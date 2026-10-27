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
  onDemand = evaluate {
    enable = true;
    user = "alice";
    onDemand.enable = true;
    sharedDirectories.project = "/Users/alice/project";
  };
  rosetta = evaluate {
    enable = true;
    user = "alice";
    rosetta = true;
  };
  disabled = evaluate { enable = false; };
in
assert builtins.all (x: x.assertion) enabled.assertions;
assert enabled.launchd.daemons ? orbyd;
assert !(enabled.launchd.daemons.orbyd.serviceConfig ? EnvironmentVariables);
assert !(enabled.services.orby ? speedFactor);
assert !(enabled.services.orby ? supportDeterminateNix);
assert lib.hasInfix " activate /nix/store/" enabled.system.activationScripts.extraActivation.text;
assert lib.hasInfix " purge" disabled.system.activationScripts.postActivation.text;
assert !(lib.hasInfix "activate" disabled.system.activationScripts.postActivation.text);
assert !(lib.hasInfix "orb delete" disabled.system.activationScripts.postActivation.text);
assert lib.any (x: !x.assertion && lib.hasInfix "orbctl" x.message) invalidCLI.assertions;
assert lib.any (x: !x.assertion && lib.hasInfix "diskSize" x.message) invalidDisk.assertions;
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
assert
  rosetta.nix.buildMachines == [
    (
      (builtins.head enabled.nix.buildMachines)
      // {
        systems = [
          "aarch64-linux"
          "x86_64-linux"
        ];
      }
    )
  ];
assert !(disabled.nix ? buildMachines);
assert !onDemand.launchd.daemons.orbyd.serviceConfig.KeepAlive;
assert lib.hasInfix "HostName localhost"
  enabled.environment.etc."ssh/ssh_config.d/100-orby-vm.conf".text;
true
