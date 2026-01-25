use std::fs::{self, File, OpenOptions};
use std::env;
use std::io::Write;
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::unistd::{chdir, pivot_root, fork, ForkResult, sethostname};
use nix::sys::wait::waitpid;
use nix::sys::stat::{mknod, Mode, SFlag, makedev};
use std::process::Command;
use std::os::unix::process::CommandExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use oci_spec::runtime::Spec;

// Define Linux Loop Device IOCTL constants manually to avoid bindgen issues
const LOOP_SET_FD: libc::c_ulong = 0x4C00;
const LOOP_CTL_GET_FREE: libc::c_ulong = 0x4C82;

fn main() {
    println!("Crater Runtime Starting...");

    let spec = match Spec::load("config.json"){
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("Failed to load config.json: {}", e);
            std::process::exit(1);
        }
    };

    let process = spec.process().as_ref().expect("Failed to get process from config.json");
    let args = process.args().as_ref().expect("Failed to get args from process");

    if args.is_empty() {
        panic!("config.json process.args must not be empty");
    }

    let cmd = args[0].clone();
    let cmd_args = args[1..].to_vec();

    let env_vars = process.env().as_ref().expect("Failed to get envs from process");

    let root_path_str = spec.root().as_ref()
        .map(|r| r.path().to_str().unwrap())
        .expect("Root path not defined in config.json");

    let rootfs = Path::new(root_path_str);

    println!("Configuration loaded. Command: {} {:?}",cmd,cmd_args);

    let flags = CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWNET;
    unshare(flags).expect("Unshare Failed!");
    println!("Successfully isolated namespaces!");

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            println!("Parent: Setting up cgroups for child {}", child);

            let cgroup_dir = format!("/sys/fs/cgroup/crater-{}", child);
            let cgroup_path = Path::new(&cgroup_dir);

            if fs::create_dir(cgroup_path).is_ok() {
                let _ = fs::write(cgroup_path.join("pids.max"),"20");
                let _ = fs::write(cgroup_path.join("cgroup.procs"),child.to_string());
                let _ = fs::write(cgroup_path.join("memory.max"),"104857600");
                let _ = fs::write(cgroup_path.join("cpu.max"),"50000 100000");
                println!("Parent: Cgroup limits (PID=20, Mem=100MB, CPU=0.5) applied.");
            } else {
                eprintln!("Parent: Warning - Failed to set cgroups. Resource limits not applied");
            }

            println!("Parent: Waiting for container process {} to finish...", child);
            waitpid(child, None).expect("Waitpid failed");
            if cgroup_path.exists() {
                let _ = fs::remove_dir(cgroup_path);
            }
            println!("Parent: Container finished. Exiting.");

        }
        Ok(ForkResult::Child) => {
            println!("Child: Setting up isolated environment...");
            sethostname("crater-container").expect("Failed to set hostname");
            mount(None::<&str>, "/", None::<&str>, MsFlags::MS_REC | MsFlags::MS_PRIVATE, None::<&str>)
                .expect("Failed to make mounts private");

            // let rootfs = Path::new("/app/crater_rootfs");
            let image_path = "/app/container_disk.img";

            // 3. ATTACH LOOP DEVICE (Manual Implementation)
            println!("Child: Finding free loop device...");
            let l_ctl = File::open("/dev/loop-control").expect("Failed to open /dev/loop-control");
            let dev_num = unsafe { libc::ioctl(l_ctl.as_raw_fd(), LOOP_CTL_GET_FREE) };
            if dev_num < 0 { panic!("Failed to get free loop device"); }

            let device_path = format!("/dev/loop{}", dev_num);
            println!("Child: Attaching {} to {}", image_path, device_path);

            if !Path::new(&device_path).exists(){
                let dev = makedev(7, dev_num as u64);
                mknod(
                    device_path.as_str(),
                    SFlag::S_IFBLK,
                    Mode::S_IRWXU,
                    dev,
                ).expect("Failed to create loop device node");
            }
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

            let proc_path = Path::new("/proc");
            if !proc_path.exists() {
                let _ = fs::create_dir(proc_path);
            }

            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                None::<&str>
            ).expect("Failed to mount /proc");

            let sys_path = Path::new("/sys");
            if !sys_path.exists() {
                let _ = fs::create_dir(sys_path);
            }
            mount(
                Some("sysfs"),
                "/sys",
                Some("sysfs"),
                MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                None::<&str>
            ).expect("Failed to mount /sys");

            if let Err(e) = Command::new("/bin/sh")
                .args(&["-c", "ip link set up dev lo || ifconfig lo up"])
                .output()
            {
                eprintln!("Child: Warning - Failed to set up loopback device: {}", e);
            }

            println!("Child: Environment isolated. Launching command: {}...", cmd);
            let mut child_cmd = Command::new(&cmd);
            child_cmd.args(&cmd_args);

            child_cmd.env_clear();
            for env_var in env_vars {
                if let Some((key, value)) = env_var.split_once('=') {
                    child_cmd.env(key, value);
                } else {
                    panic!("Invalid environment variable: {}", env_var);
                }
            }

            let _ = child_cmd.exec();
        }
        Err(e) => panic!("Fork failed: {}", e),
    }
}