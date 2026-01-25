# Crater

A tiny, educational container runtime written in Rust. Crater demonstrates how to:
- Unshare UTS, PID, mount, and network namespaces
- Create and mount a writable ext4 root filesystem from a disk image via a loop device
- Pivot into the new root (pivot_root), mount /proc and /sys
- Apply basic cgroup v2 limits (pids, memory, cpu) to the containerized process
- Bring up the loopback interface inside the network namespace
- Launch a specified command inside the isolated environment (defaults to /bin/sh)

The repository includes a Dockerfile and a deploy script that prepare an Alpine-based root filesystem inside an ext4 disk image, then run the Rust runtime to enter it.

Note: Crater is for learning and experimentation only. It is not a secure container runtime. Use in trusted environments with appropriate isolation.

## How it works (high level)
1) The Docker image downloads an Alpine minirootfs and places it under /app/crater_rootfs.
2) deploy.sh builds the Rust binary and creates a 500 MB ext4 disk image at /app/container_disk.img, then copies the Alpine files into it.
3) The Rust program:
   - Unshares UTS, PID, mount, and network namespaces
   - Forks: the parent configures cgroup v2 limits for the child; the child sets the hostname and makes mounts private
   - Finds a free /dev/loopN via ioctl(LOOP_CTL_GET_FREE), and attaches the disk image with ioctl(LOOP_SET_FD)
   - Mounts the loop device to /app/crater_rootfs as ext4
   - Bind-mounts rootfs, performs pivot_root into it, and cleans up the old root
   - Mounts safe /proc and /sys
   - Brings up the loopback interface (lo) inside the network namespace
   - Execs the provided command (defaults to /bin/sh) inside the new root

Key files:
- src/main.rs: namespaces (UTS/PID/mount/net) + loop device attach + mount + pivot_root + /proc and /sys mounts + basic cgroups v2 + loopback setup + shell exec
- Dockerfile: prepares toolchain and Alpine minirootfs in the image layers, ensures /sbin/ip is available via busybox
- deploy.sh: builds, creates the ext4 image, copies the Alpine rootfs into it, then runs the binary
- Cargo.toml: uses nix and libc crates (Rust 2024 edition)

## Requirements

Host/kernel capabilities (whether inside Docker or on a bare-metal host):
- Linux kernel with:
  - CONFIG_USER_NS, CONFIG_PID_NS, CONFIG_UTS_NS, CONFIG_NET_NS, CONFIG_NAMESPACES
  - CONFIG_BLK_DEV_LOOP (loop device support); module "loop" must be available/loaded
  - ext4 filesystem support
  - procfs and sysfs support
  - cgroup v2 mounted at /sys/fs/cgroup (most modern distros default to this)
- Root privileges (or sufficient capabilities: SYS_ADMIN, SYS_RESOURCE, SYS_CHROOT, MKNOD, CAP_SETUID/CAP_SETGID for some setups). In practice: run as root or in a fully privileged container.
- Tools used by the helper script: dd, mkfs.ext4, mount, cp, umount, curl, tar.
- For networking setup inside the container: busybox ip or iproute2 available in the rootfs (Dockerfile symlinks /sbin/ip to busybox)
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
   Parent: Setting up cgroups for child <pid>
   Parent: Cgroup limits (PID=20, Mem=100MB, CPU=0.5) applied.
   Parent: Waiting for container process <pid> to finish...
   Child: Setting up isolated environment...
   Child: Finding free loop device...
   Child: Attaching /app/container_disk.img to /dev/loopX
   Child: Environment isolated. Launching command: /bin/sh...

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
- This runtime is intentionally minimal: basic cgroup v2 limits only (pids/memory/cpu); no user namespace mapping, no seccomp, no device isolation.
- Networking: only a separate network namespace with loopback brought up; no veth or external connectivity setup.
- The child process execs the provided command (defaults to /bin/sh). You can pass a custom command and arguments; see the section below. Changing src/main.rs is not required for simple cases.
- The disk image size is fixed at 500 MB in deploy.sh; adjust as needed.
- Loop device cleanup is basic; for production you’d use a more robust loop-control strategy.

## Development
- Code location: src/main.rs (about 150+ lines)
- Build: cargo build
- Run (Docker): docker build -t crater . && docker run --rm -it --privileged crater
- Run (host): see steps above

## License
This project is provided as-is for educational purposes. See LICENSE if present or treat as all-rights-reserved if absent.


## Running a custom command

Crater can exec any command you provide. If no command is given, it defaults to /bin/sh.

- On a Linux host (recommended for trying custom commands):
  - Run a simple command:
    sudo ./target/debug/Crater /bin/echo "hello from crater"
  - Run a shell with inline commands:
    sudo ./target/debug/Crater /bin/sh -lc "uname -a && id && cat /etc/os-release"

- Inside the provided Docker image (note about deploy.sh):
  - The included deploy.sh prepares the disk image and then runs the runtime without forwarding arguments, so by default you always land in /bin/sh.
  - If you want to pass a custom command when using docker run, the simplest approach is to tweak deploy.sh locally to forward arguments:
    - Change the last line of deploy.sh from:
        ./target/debug/Crater
      to:
        exec ./target/debug/Crater "$@"
    - Then you can run:
        docker run --rm -it --privileged crater:latest /bin/echo "hello from crater in docker"
  - Alternatively, you can override the entrypoint and reproduce the deploy steps manually before launching your command; this is more cumbersome but avoids editing the script.
