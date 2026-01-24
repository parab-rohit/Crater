# Crater

A tiny, educational container runtime written in Rust. Crater demonstrates how to:
- Unshare UTS, PID and mount namespaces
- Create and mount a writable ext4 root filesystem from a disk image via a loop device
- Pivot into the new root (pivot_root) and mount /proc
- Launch a shell inside the isolated environment

The repository includes a Dockerfile and a deploy script that prepare an Alpine-based root filesystem inside an ext4 disk image, then run the Rust runtime to enter it.

Note: Crater is for learning and experimentation only. It is not a secure container runtime. Use in trusted environments with appropriate isolation.

## How it works (high level)
1) The Docker image downloads an Alpine minirootfs and places it under /app/crater_rootfs.
2) deploy.sh builds the Rust binary and creates a 500 MB ext4 disk image at /app/container_disk.img, then copies the Alpine files into it.
3) The Rust program:
   - Unshares UTS, PID, and mount namespaces
   - Forks: the parent waits; the child sets the hostname, makes mounts private
   - Finds a free /dev/loopN via ioctl(LOOP_CTL_GET_FREE), and attaches the disk image with ioctl(LOOP_SET_FD)
   - Mounts the loop device to /app/crater_rootfs as ext4
   - Bind-mounts rootfs, performs pivot_root into it, and cleans up the old root
   - Mounts a safe /proc and execs /bin/sh inside the new root

Key files:
- src/main.rs: namespace + loop device + mount + pivot_root + /proc + shell
- Dockerfile: prepares toolchain and Alpine minirootfs in the image layers
- deploy.sh: builds, creates the ext4 image, copies the Alpine rootfs into it, then runs the binary
- Cargo.toml: uses nix and libc crates (Rust 2024 edition)

## Requirements

Host/kernel capabilities (whether inside Docker or on a bare-metal host):
- Linux kernel with:
  - CONFIG_USER_NS, CONFIG_PID_NS, CONFIG_UTS_NS, CONFIG_NAMESPACES
  - CONFIG_BLK_DEV_LOOP (loop device support); module "loop" must be available/loaded
  - ext4 filesystem support
  - procfs support
- Root privileges (or sufficient capabilities: SYS_ADMIN, SYS_RESOURCE, SYS_CHROOT, MKNOD, etc.). In practice: run as root or in a fully privileged container.
- Tools used by the helper script: dd, mkfs.ext4, mount, cp, umount, curl, tar.
- Rust toolchain if building outside Docker.

Security notes:
- This project requires powerful privileges and manipulates mounts/loop devices. Run only in disposable VMs or test hosts you control.
- SELinux/AppArmor can block operations like mount or pivot_root. You may need to switch to permissive mode temporarily for testing.

## Quick start using Docker (recommended)

1) Build the image:
   docker build -t crater:latest .

2) Run with sufficient privileges. The simplest way for experimentation is --privileged:
   docker run --rm -it --privileged crater:latest

   What happens:
   - The entrypoint deploy.sh builds the Rust binary
   - Creates /app/container_disk.img (500 MB ext4)
   - Mounts it temporarily to copy the Alpine rootfs into it, then unmounts
   - Starts the Crater runtime which attaches the disk to a /dev/loopN and pivots into it

3) You should see logs similar to:
   Crater Runtime Starting...
   Successfully isolated namespaces!
   Parent: Waiting for container process ... to finish...
   Child: Setting up isolated environment...
   Child: Finding free loop device...
   Child: Attaching /app/container_disk.img to /dev/loopX
   Child: Environment isolated. Launching shell...

   Then you will drop into an Alpine shell inside the isolated root. Try:
   uname -n
   ps -ef
   mount

   Exit the shell with Ctrl-D or the exit command to terminate the containerized process.

Alternative to --privileged (advanced):
- You may experiment with granting only the needed capabilities and devices, e.g.:
  docker run --rm -it \
    --cap-add SYS_ADMIN --cap-add SYS_CHROOT --cap-add SYS_RESOURCE \
    --device /dev/loop-control --device /dev/loop0 --device /dev/loop1 \
    --security-opt apparmor:unconfined --security-opt seccomp=unconfined \
    crater:latest
  Note: the program selects a free /dev/loopN dynamically; providing multiple /dev/loop* devices or using --privileged is simpler.

## Running on a Linux host (without Docker)

Prerequisites:
- Root shell
- loop module loaded and devices present:
  modprobe loop
  ls -l /dev/loop-control /dev/loop0
- Tools: dd, mkfs.ext4, mount, cp, umount, curl, tar, losetup (optional for inspection)
- Rust toolchain (rustup with Rust 1.84+ compatible with edition 2024)

Steps:
1) Build the binary:
   cargo build

2) Prepare an Alpine rootfs (same as the Dockerfile does):
   mkdir -p ./crater_rootfs
   curl -o /tmp/alpine.tar.gz https://dl-cdn.alpinelinux.org/alpine/v3.18/releases/x86_64/alpine-minirootfs-3.18.4-x86_64.tar.gz
   sudo tar -xzvf /tmp/alpine.tar.gz -C ./crater_rootfs

3) Create a 500 MB ext4 disk image and populate it:
   dd if=/dev/zero of=./container_disk.img bs=1M count=500
   mkfs.ext4 ./container_disk.img
   sudo mkdir -p /mnt/tmp_disk
   sudo mount -o loop ./container_disk.img /mnt/tmp_disk
   sudo cp -a ./crater_rootfs/. /mnt/tmp_disk/
   sudo umount /mnt/tmp_disk

4) Run the runtime (as root):
   sudo ./target/debug/Crater

Expected outcome: logs similar to the Docker quick start, followed by an interactive shell. The runtime will dynamically attach ./container_disk.img to a /dev/loopN and pivot into it.

Cleanup (optional):
- The loop device attached by the runtime should be released when the process exits and the mount is gone. If you need to inspect or clean manually:
  sudo losetup -a           # list loop mappings
  sudo losetup -D           # detach all loop devices (careful)

## Troubleshooting
- Permission denied on mount/pivot_root/ioctl:
  - Ensure you are root or running with --privileged inside Docker
  - Check SELinux/AppArmor: try setenforce 0 (SELinux) or apparmor=unconfined for the container
- /dev/loop-control not found:
  - Load the loop module: modprobe loop
  - Check your environment (e.g., WSL2 may not provide loop devices)
- mount: wrong fs type, bad option, bad superblock:
  - Ensure mkfs.ext4 succeeded and you copied the Alpine rootfs correctly
  - Verify the image is not still mounted elsewhere
- pivot_root failed:
  - The new root must be a mount point; code bind-mounts it before pivot_root, but ensure the mount succeeded
  - Check dmesg for LSM denials
- Shell (/bin/sh) not found:
  - Ensure the Alpine rootfs is fully copied; cp -a preserves permissions and symlinks

## Limitations and notes
- This runtime is intentionally minimal: no cgroups, no networking setup, no user namespace mapping, no seccomp.
- The child process currently execs /bin/sh; change src/main.rs if you want to run a specific command.
- The disk image size is fixed at 500 MB in deploy.sh; adjust as needed.
- Loop device cleanup is basic; for production you’d use a more robust loop-control strategy.

## Development
- Code location: src/main.rs (about 90 lines)
- Build: cargo build
- Run (Docker): docker build -t crater . && docker run --rm -it --privileged crater
- Run (host): see steps above

## License
This project is provided as-is for educational purposes. See LICENSE if present or treat as all-rights-reserved if absent.
