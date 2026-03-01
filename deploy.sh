#!/bin/sh
trap "echo 'Deploy script exited. Sleeping for 1 hour to allow debugging...'; sleep 3600" EXIT

set -e

cargo build

dd if=/dev/zero of=/app/container_disk.img bs=1M count=500
mkfs.ext4 /app/container_disk.img

if command -v losetup > /dev/null 2>&1; then
  echo "Cleaning up stray loop devices for /app/container_disk.img..."
  losetup -j /app/container_disk.img | cut -d: -f1 | xargs -r losetup -d || true
fi

if [ ! -e /dev/loop-control ]; then
    mknod /dev/loop-control c 10 237 || true
fi
for i in $(seq 0 32); do
  if [ ! -e /dev/loop$i ];then
    mknod /dev/loop$i b 7 $i || true
  fi
done


echo "=== DEBUG: LOOP DEVICE INFO === "
ls -l /dev/loop* || echo "ls failed"

if command -v losetup > /dev/null 2>&1;then
  echo "Active loop devices (losetup -a):"
  losetup -a || echo "losetup -a failed"
  echo "Next free loop devices (losetup -f):"
  losetup -f || echo "losetup -f failed"
else
  echo "losetup not found"
fi
echo "==============================="

# 3. Mount the blank disk to a temporary location to "fill" it
mkdir -p /mnt/tmp_disk
mount -o loop /app/container_disk.img /mnt/tmp_disk

# 4. Move Alpine files from the Docker image layers into the virtual disk
# We use cp -a to preserve permissions (important for /bin/sh)
cp -a /app/crater_rootfs/. /mnt/tmp_disk/

# 5. Unmount so the Rust program can take control of the file
umount /mnt/tmp_disk

# 6. Run the runtime
cat <<EOF > config.json
{
  "ociVersion": "1.0.0",
  "hostname": "crater-oci-demo",
  "process": {
    "terminal": false,
    "user": {
      "uid": 0,
      "gid": 0
    },
    "capabilities": {
      "bounding": ["CAP_CHOWN", "CAP_NET_BIND_SERVICE", "CAP_SETUID", "CAP_SETGID"],
      "effective": ["CAP_CHOWN", "CAP_NET_BIND_SERVICE", "CAP_SETUID", "CAP_SETGID"],
      "permitted": ["CAP_CHOWN", "CAP_NET_BIND_SERVICE", "CAP_SETUID", "CAP_SETGID"]
    },
    "args": [
      "/bin/sh",
      "-c",
      "echo 'Container started, sleeping...'; sleep 60"
    ],
    "env": [
      "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      "TERM=xterm"
    ],
    "cwd": "/tmp"
  },
  "root": {
    "path": "/app/crater_rootfs"
  },
  "mounts": [
    {
      "destination": "/proc",
      "type": "proc",
      "source": "proc",
      "options": ["nosuid", "noexec", "nodev"]
    },
    {
      "destination": "/sys",
      "type": "sysfs",
      "source": "sysfs",
      "options": ["nosuid", "noexec", "nodev", "ro"]
    }
  ]
}
EOF
./target/debug/Crater create demo-container &

echo "Container created and waiting for the container start"
./target/debug/Crater start demo-container
#echo "Container paused. Sleeping for 1 hour to allow log inspection..."
#sleep 3600

#echo "--- TESTING KILL COMMAND ---"
#echo "1. Starting container in background..."
#./target/debug/Crater run demo-container &
#BG_PID=$!
#
#echo "2. Waiting 5 seconds for container to initialize..."
#sleep 5
#
#echo "3. Sending SIGKILL to container..."
#./target/debug/Crater kill demo-container SIGKILL
#
#echo "4. Waiting for main process to exit..."
#wait $BG_PID || true
#
#echo "Test Finished."
#
#echo "Container paused. Sleeping for 1 hour to allow log inspection..."
#sleep 3600