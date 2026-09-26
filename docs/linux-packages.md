# Linux Packages

Rism ships native packages for the three major Linux families. A package —
not a setup wizard — is the idiomatic Linux installer: the distro's package
manager owns PATH placement, upgrades, dependency checking, and clean
removal. Each release publishes all three formats from the **same** binary.

## Install

=== Debian / Ubuntu

```bash
# download rism_<version>_amd64.deb from the releases page
sudo apt install ./rism_<version>_amd64.deb
# or on a clean dpkg-only system:
sudo dpkg -i rism_<version>_amd64.deb
```

Upgrade: `sudo apt install ./rism_<new>.deb` (or your apt source once
configured). Remove: `sudo apt remove rism`.

=== Fedora / RHEL

```bash
sudo dnf install ./rism-<version>-1.x86_64.rpm
# or: sudo rpm -i rism-<version>-1.x86_64.rpm
```

Upgrade: `sudo dnf upgrade ./rism-<new>.rpm` (or `rpm -Uvh`).
Remove: `sudo dnf remove rism`.

=== Arch Linux

Preferred: install from the **AUR** (`rism`) once published — `yay -S rism`.

Direct file install (before/without AUR):

```bash
sudo pacman -U rism-<version>-1-x86_64.pkg.tar.zst
```

Upgrade: same command with the new file, or `yay -Syu`.
Remove: `sudo pacman -R rism`.

## What gets installed

| Path | Content |
|---|---|
| `/usr/bin/rism` | the binary (statically linked — no runtime dependencies) |
| `/usr/share/doc/rism/README.md`, `LICENSE` | docs + AGPL-3.0 text |
| `~/.config/rism/config.toml` | **never touched by the package** — created by Rism at first use; survives uninstall (same policy as the Windows installer) |

`postinst` checks for a running `rism` process and prints a restart
reminder if the package was upgraded under a live MCP session — it never
kills or fails the transaction.

## Verification (CI-proven)

`.github/workflows/linux-packages.yml` runs the full
install → upgrade → uninstall contract inside real `debian:12`,
`ubuntu:24.04`, `fedora:42`, and `archlinux:latest` containers, plus a
config-preservation probe, on every change to `packaging/**` or the
pipelines — the same gate philosophy as
[Windows installer contract](windows-store.md). Package *contents* are
asserted against the package listing (`dpkg-deb -c` / `rpm -qlp` /
`bsdtar -tf`), not the container filesystem: the official Debian/Ubuntu
images ship a `path-exclude=/usr/share/doc/*` dpkg filter and the Arch image
a pacman `NoExtract` for the same path — a doc-slimming choice of the image
maintainers, so files the package provably owns legitimately never appear on
disk there.

## Packaging internals

- `packaging/nfpm.yaml` — single [nfpm](https://nfpm.goreleaser.com)
  definition producing all three formats; CI substitutes
  `RISM_VERSION` / `RISM_ARCH` / `RISM_RELEASE` from the tag.
- Version edge cases are handled by nfpm's `semver` schema automatically:
  tag `v0.2.0-rc1` becomes `0.2.0~rc1` (deb/rpm) and `0.2.0rc1` (pacman),
  which all order correctly below `0.2.0`.
- The Linux release binary is built for `x86_64-unknown-linux-musl` (fully
  static, no glibc floor — see [Getting Started](getting-started.md)).
- AUR note: the `-bin` PKGBUILD points at the GitHub release archive;
  publishing to AUR is a separate manual step (aur.archlinux.org account +
  one `git push` per release).
