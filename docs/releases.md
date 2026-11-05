# Releases

Releases are Linux x86_64 tarballs. Windows support is planned separately.
The binary uses the existing release profile: full LTO, one codegen unit,
and stripped symbols. Packaging adds no startup work.

## Install

Download the tarball from the repository's GitHub Releases page, extract it,
and install the binary:

```sh
tar -xzf mdview-v0.1.0-linux-x86_64.tar.gz
install -Dm755 mdview-v0.1.0-linux-x86_64/mdview ~/.local/bin/mdview
mdview README.md
```

Make sure `~/.local/bin` is on your `PATH`. The archive also contains the
README, this guide and the application icon. The SHA-256 checksum is in the
release notes; compare it with `sha256sum` before installing.

The binary is built on Ubuntu 22.04 with glibc 2.35. Use a compatible Linux
x86_64 distribution with glibc 2.35 or newer. It requires X11 or Wayland,
libxkbcommon (including its X11 library), libxcb, fontconfig, freetype,
a Vulkan loader and a working GPU driver. For the file picker, install
xdg-desktop-portal and a suitable backend. On Ubuntu:

```sh
sudo apt-get install libxkbcommon0 libxkbcommon-x11-0 libxcb1 libwayland-client0 libwayland-cursor0 libwayland-egl1 libfontconfig1 libfreetype6 libvulkan1 xdg-desktop-portal xdg-desktop-portal-gtk
```

Install the Vulkan driver appropriate for your GPU as described in the
contributing guide. The tarball does not bundle system libraries or drivers.

## Publish

Before tagging, test the intended commit, update the package version in
Cargo.toml and Cargo.lock, and merge the changes to main. A release is an
explicit maintainer action; ordinary pushes do not publish anything.

```sh
git switch main
git pull --ff-only
git tag -a v0.1.0 -m 'mdview v0.1.0'
git push origin v0.1.0
```

Use the next version for subsequent releases. Never move a published tag.
The release workflow rejects tags that do not match the app package version.
It runs formatting, guards, clippy and the full headless workspace test suite
using the CI profile, then builds the Linux release binary, packages it,
checks the extracted binary's help output and shared libraries, and publishes
a GitHub Release directly with one tarball and generated notes. Real window
tests remain local only. There is no draft stage. A failed test or build does
not publish a release; fix the cause before rerunning the workflow.

```mermaid
flowchart LR
    tested["Test and merge commit"] --> tag["Push version tag"]
    tag --> version["Verify package version"]
    version --> tests["Full headless tests and checks"]
    tests --> build["Build Linux release binary"]
    build --> package["Package and smoke test tarball"]
    package --> publish["Publish GitHub Release"]
```
