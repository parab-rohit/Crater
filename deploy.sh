#!/bin/sh
set -e

# 1. Build the Rust project
cargo build

# 2. Create a 500MB virtual disk if it doesn't exist
dd if=/dev/zero of=/app/container_disk.img bs=1M count=500
mkfs.ext4 /app/container_disk.img

# 3. Mount the blank disk to a temporary location to "fill" it
mkdir -p /mnt/tmp_disk
mount -o loop /app/container_disk.img /mnt/tmp_disk

# 4. Move Alpine files from the Docker image layers into the virtual disk
# We use cp -a to preserve permissions (important for /bin/sh)
cp -a /app/crater_rootfs/. /mnt/tmp_disk/

# 5. Unmount so the Rust program can take control of the file
umount /mnt/tmp_disk

# 6. Run the runtime
./target/debug/Crater