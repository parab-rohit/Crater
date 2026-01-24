use std::fs::{self, File, OpenOptions};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::unistd::{chdir, pivot_root, fork, ForkResult, sethostname};
use nix::sys::wait::waitpid;
use std::process::Command;
use std::os::unix::process::CommandExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;

// Define Linux Loop Device IOCTL constants manually to avoid bindgen issues
const LOOP_SET_FD: libc::c_ulong = 0x4C00;
const LOOP_CTL_GET_FREE: libc::c_ulong = 0x4C82;

fn main() {
    println!("Crater Runtime Starting...");
    let flags = CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS;
    unshare(flags).expect("Unshare Failed!");
    println!("Successfully isolated namespaces!");

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            println!("Parent: Waiting for container process {} to finish...", child);
            waitpid(child, None).expect("Waitpid failed");
            println!("Parent: Container finished. Exiting.");
        }
        Ok(ForkResult::Child) => {
            println!("Child: Setting up isolated environment...");
            sethostname("crater-container").expect("Failed to set hostname");
            mount(None::<&str>, "/", None::<&str>, MsFlags::MS_REC | MsFlags::MS_PRIVATE, None::<&str>)
                .expect("Failed to make mounts private");

            let rootfs = Path::new("/app/crater_rootfs");
            let image_path = "/app/container_disk.img";

            // 3. ATTACH LOOP DEVICE (Manual Implementation)
            println!("Child: Finding free loop device...");
            let l_ctl = File::open("/dev/loop-control").expect("Failed to open /dev/loop-control");
            let dev_num = unsafe { libc::ioctl(l_ctl.as_raw_fd(), LOOP_CTL_GET_FREE) };
            if dev_num < 0 { panic!("Failed to get free loop device"); }

            let device_path = format!("/dev/loop{}", dev_num);
            println!("Child: Attaching {} to {}", image_path, device_path);

            let backing_file = OpenOptions::new().read(true).write(true).open(image_path)
                .expect("Failed to open backing image file");
            let loop_dev = OpenOptions::new().read(true).write(true).open(&device_path)
                .expect("Failed to open loop device");

            let res = unsafe { libc::ioctl(loop_dev.as_raw_fd(), LOOP_SET_FD, backing_file.as_raw_fd()) };
            if res < 0 { panic!("Failed to SET_FD for loop device"); }

            // 4. MOUNT THE LOOP DEVICE
            mount(
                Some(device_path.as_str()),
                rootfs,
                Some("ext4"),
                MsFlags::empty(),
                None::<&str>,
            ).expect("Failed to mount loop device");

            // 5. Pivot Root
            mount(Some(rootfs), rootfs, None::<&str>, MsFlags::MS_BIND | MsFlags::MS_REC, None::<&str>)
                .expect("Failed to bind mount rootfs");

            let put_old = rootfs.join("old_root");
            fs::create_dir_all(&put_old).expect("Failed to create old_root");

            pivot_root(rootfs, &put_old).expect("pivot_root failed");
            chdir("/").expect("chdir to / failed");

            // 6. Cleanup
            umount2("/old_root", MntFlags::MNT_DETACH).ok();
            fs::remove_dir("/old_root").ok();

            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                None::<&str>
            ).expect("Failed to mount /proc");

            println!("Child: Environment isolated. Launching shell...");
            let mut child_cmd = Command::new("/bin/sh");
            let _ = child_cmd.exec();
        }
        Err(e) => panic!("Fork failed: {}", e),
    }
}