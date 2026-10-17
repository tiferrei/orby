# Orby

Orby configures an [OrbStack](https://orbstack.dev/) NixOS machine as an
`aarch64-linux` remote builder for nix-darwin on Apple Silicon. OrbStack owns
the machine and its disk; Orby creates, provisions, starts, stops, and deletes
the named machine using `orbctl`.

Orby is a fork of [Virby](https://github.com/quinneden/virby-nix-darwin) by
Quinn Edenfield. It retains Virby's goal of providing a Nix-darwin Linux
builder while replacing its vfkit/krunkit VM backend with OrbStack.

> **Destructive disable:** turning off `services.orby.enable` deletes the
> Orby-managed machine, including its Nix store and all files in it, as well
> as Orby's local credentials. Back up anything you need first.

## Setup

Install and start OrbStack under the macOS account that will own the machine.
The account must be able to run `orbctl` without interaction. By default Orby
uses the first `orbctl` found in that account's login `PATH` when activating,
and saves the resolved path for its runner and eventual purge. In a nix-darwin
configuration:

```nix
{
  inputs.darwin.url = "github:nix-darwin/nix-darwin";
  inputs.orby.url = "github:YOUR-ACCOUNT/orby";
  outputs = { darwin, orby, ... }: {
    darwinConfigurations.myMac = darwin.lib.darwinSystem {
      system = "aarch64-darwin";
      modules = [ orby.darwinModules.default ./configuration.nix ];
    };
  };
}
```

```nix
{
  services.orby = {
    enable = true;
    user = "my-macos-username";
    # Optional: override PATH discovery with an absolute executable path.
    # orbctl = "/opt/homebrew/bin/orbctl";
    cores = 8;
    memoryMiB = 6144;
    diskSize = "100G";
  };
}
```

The first activation downloads OrbStack's NixOS distribution and runs
`nixos-rebuild` inside it. It may take time and requires network access from
the guest; unlike Virby, it needs no prebuilt disk-image cache or existing
Linux builder. Orby generates its own SSH key for Nix under `/var/lib/orby`,
uses a loopback-only SSH proxy, and registers a Nix `ssh-ng` build machine.
Try an `aarch64-linux` build to verify that the guest is available.

## Lifecycle

By default, Orby keeps its machine running. For on-demand use:

```nix
services.orby.onDemand = {
  enable = true;
  ttl = 180; # minutes since the last SSH connection closes
};
```

Orby starts the named machine when an SSH build connection arrives and stops
it when idle; it never stops OrbStack itself or any other machine. On service
disable, Orby deletes the machine and its host state. If deletion fails, it
reports an activation error instead of silently dropping the ownership record.
Orby refuses to take over a pre-existing machine named `orby-builder`.
It also checks a machine identity stored on both the host and the guest before
reconfiguring or deleting an existing machine. If first-time provisioning was
interrupted before that identity was installed, Orby reports a failure rather
than deleting an unverified machine. If you have confirmed that Orby created
`orby-builder`, restore the interrupted identity before retrying:

```sh
orbctl run -m orby-builder -u root mkdir -p /etc/nixos/orby
orbctl run -m orby-builder -u root sh -c \
  'printf "%s\n" "$1" > /etc/nixos/orby/identity' sh \
  "$(sudo cat /var/lib/orby/identity)"
```

Do not run these commands on a machine that Orby did not create.

Additional NixOS module declarations can be supplied as trusted Nix source:

```nix
services.orby.guestConfig = ''
  environment.systemPackages = with pkgs; [ git ];
'';
```

These declarations run in the guest and are applied by `nixos-rebuild switch`
at activation. The guest includes its original OrbStack NixOS configuration.

To deliberately expose a host directory to the builder, use
`services.orby.sharedDirectories.project = "/Users/me/project";`. It will be
available in the guest at `/mnt/virtiofs/project`. This opts that directory
out of OrbStack's default host-file isolation.

## Limitations and security

Orby currently exposes only `aarch64-linux` builds. Rosetta and nested
virtualization are not enabled; do not advertise
`x86_64-linux` or `kvm` until verified on the target machine. OrbStack
machines share one Linux kernel: even isolated machines are not equivalent to
Virby's dedicated-kernel VMs as a security boundary. The Orby machine is
created with OrbStack's `--isolated` mode, without host filesystem mounts
unless `sharedDirectories` is configured, and without SSH agent forwarding.

The OrbStack owner can control the machine and its Nix builder; choose a
trusted local user. `services.orby.orbctl` optionally overrides CLI discovery
without requiring the unfree OrbStack package in your Nix store. This fork does
not automatically migrate or delete a Virby installation. To purge Orby,
first switch with `services.orby.enable = false` while keeping the Orby module
imported; remove the import only after that switch succeeds.

## Credits

- [Virby](https://github.com/quinneden/virby-nix-darwin) by Quinn Edenfield is
  the original project. Its MIT copyright notice is preserved in [LICENSE](LICENSE).
- Virby was modeled after [nix-rosetta-builder](https://github.com/cpick/nix-rosetta-builder)
  by cpick, from which it borrowed and adapted parts of its implementation.
- [OrbStack](https://orbstack.dev/) by Orbital Labs provides the machine runtime
  used by Orby.
