use std::fs::{self, File, OpenOptions};
use std::env;
use std::io::{Read, Write};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::sched::{unshare, CloneFlags, setns};
use nix::unistd::{chdir, pivot_root, fork, ForkResult, sethostname, Pid, pipe, mkfifo};
use nix::fcntl::{open, OFlag};
use nix::sys::stat::{mknod, Mode, SFlag, makedev};
use std::process::{Command, Stdio};
use std::os::unix::process::CommandExt;
use std::os::unix::io::{ AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};
use nix::sys::signal::{self, Signal, SIGKILL};
use oci_spec::runtime::Spec;
use serde::{Serialize, Deserialize };

// Define Linux Loop Device IOCTL constants manually to avoid bindgen issues
const LOOP_SET_FD: libc::c_ulong = 0x4C00;
const LOOP_CTL_GET_FREE: libc::c_ulong = 0x4C82;

#[derive(Serialize, Deserialize)]
struct ContainerState {
    ociVersion: String,
    id: String,
    state: String,
    pid: i32,
    bundle: String,
}

fn get_state(id: &str) -> ContainerState {
    let state_dir = get_container_dir(id);
    let pid_path = state_dir.join("pid");

    if !pid_path.exists() {
        eprintln!("container not found");
        std::process::exit(1);
    }
    let pid = fs::read_to_string(pid_path).unwrap().trim().parse::<i32>().unwrap();

    let status = if Path::new(&format!("/proc/{}",pid)).exists() {
        "running"
    } else {
        "stopped"
    };

    let state = ContainerState {
        ociVersion: "1.0.2".to_string(),
        id: id.to_string(),
        state: status.to_string(),
        pid,
        bundle: format!("/run/crater/{}",id).to_string(),
    };
    state
}

fn print_container_state(id: &str) {
    let state = get_state(id);
    println!("{}", serde_json::to_string_pretty(&state).unwrap());
}


fn list_containers(){
    let base_dir = Path::new("/var/run/crater");
    if !base_dir.exists() {
        println!("ID\t\tPID\t\tSTATUS\t\tBUNDLE");
        return;
    }
    println!("{:<20} {:<10} {:<15} {:<20}", "ID", "PID", "STATUS", "BUNDLE");
    println!("{}", "-".repeat(70));
    if let Ok(entries) = fs::read_dir(base_dir) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if let Ok(state) = try_get_state(&id) {
                println!("{:<20} {:<10} {:<15} {:<20}",
                state.id, state.pid, state.state, state.bundle)
            };
        }
    }
}

fn try_get_state(id: &str) -> Result<ContainerState,String> {
    let state_dir = get_container_dir(id);
    let pid_path = state_dir.join("pid");
    if !pid_path.exists(){
        return Err("no PID file".to_string());
    }
    let pid_str = fs::read_to_string(pid_path).map_err(|e| e.to_string())?;
    let pid = pid_str.trim().parse::<i32>().map_err(|e| e.to_string())?;
    let status = if Path::new(&format!("/proc/{}", pid)).exists() {
        "running"
    } else {
        "stopped"
    };
    Ok(ContainerState {
        ociVersion: "1.0.2".to_string(),
        id: id.to_string(),
        state: status.to_string(),
        pid,
        bundle: format!("/run/crater/{}",id).to_string(),
    })


}
fn parse_mount_opts(options: &Option<Vec<String>>) -> (MsFlags, String) {
    let mut flags= MsFlags::empty();
    let mut data = Vec::new();
    if let Some(opts) = options {
        for opt in opts {
            match opt.as_str() {
                "defaults" => {},
                "ro" => flags |= MsFlags::MS_RDONLY,
                "rw" => {},
                "nosuid" => flags |= MsFlags::MS_NOSUID,
                "noexec" => flags |= MsFlags::MS_NOEXEC,
                "nodev" => flags |= MsFlags::MS_NODEV,
                "bind" => flags |= MsFlags::MS_BIND,
                "rbind" => flags |= MsFlags::MS_BIND | MsFlags::MS_REC,
                s => data.push(s.to_string()),
            }
        }
    }
    (flags, data.join(","))
}
fn main() {
    let args: Vec<String> = env::args().collect();
    let command = args.get(1).map(|s| s.as_str());

    match command {
        Some("create") => {
            if args.len() < 3 {
                eprintln!("Usage {}: cargo run <container_id>", args.get(0).unwrap_or(&String::from("crater")));
                std::process::exit(1);
            }
            let container_id = &args[2];
            create_container(container_id);
        }
        Some("start") => {
            if args.len() < 3 {
                eprintln!("Usage {}: cargo start <container_id>", args.get(0).unwrap_or(&String::from("crater")));
            }
            println!("Start container {}", args[2]);
            start_container(&args[2]);
        }
        // Some("run") => {
        //     if args.len() < 3 {
        //         eprintln!("Usage: {} run <container_id>", args.get(0).unwrap_or(&String::from("crater")));
        //         std::process::exit(1);
        //     }
        //     let container_id = &args[2];
        //     run_container(container_id);
        // }
        Some("exec") => {
            if args.len() < 4 {
                eprintln!("Usage: {} exec <container_id> <command> [args...]", args.get(0).unwrap_or(&String::from("crater")));
                std::process::exit(1);
            }
            let container_id = &args[2];
            let command = &args[3];
            let cmd_args : Vec<&str> = if args.len() > 4 { args[4..].iter().map(|s| s.as_str()).collect() } else { vec![] };
            exec_in_container(container_id, command, cmd_args);
        }
        Some("kill") => {
            if args.len() < 3 {
                eprintln!("Usage: {} kill <container_id> <signal>", args.get(0).unwrap_or(&String::from("crater")));
                std::process::exit(1);
            }
            let container_id = &args[2];
            let signal = args.get(3).map(|s| s.as_str()).unwrap_or("SIGKILL");
            kill_container(container_id, signal);
        }
        Some("state") => {
            if args.len() < 3 {
                eprintln!("Usage: {} state <container_id>", args.get(0).unwrap_or(&String::from("crater")));
                std::process::exit(1);
            }
            let container_id = &args[2];
            print_container_state(&container_id);
        }
        Some("delete") => {
            if args.len() < 3 {
                eprintln!("Usage: {} delete <container_id>", args.get(0).unwrap_or(&String::from("crater")));
                std::process::exit(1);
            }
            let container_id = &args[2];
            let force = args.contains(&"--force".to_string());

            let state = get_state(&container_id);
            if state.state == "running" && !force {
                eprintln!("Container {} is running. use --force option to delete the container", container_id);
                std::process::exit(1);
            }
            if state.state == "running" && force {
                kill_container(&container_id,"SIGKILL" );
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            cleanup_container_resources(&container_id);
        }
        Some("list") => {
            list_containers();
        }
        _ => {
            eprintln!("Usage: {} run <container_id>", args.get(0).unwrap_or(&String::from("crater")));
            std::process::exit(1);
        }
    }
}

fn exec_in_container(container_id: &str, command: &str, cmd_args: Vec<&str>) {
    let state_dir = get_container_dir(container_id);
    let pid_path = state_dir.join("pid");

    if !pid_path.exists() {
        eprintln!("Container {} is not in crated state or pid file is missing", container_id);
        std::process::exit(1);
    }
    let pid_str = fs::read_to_string(pid_path).expect("Failed to read pid file");
    let target_pid = pid_str.trim().parse::<i32>().expect("Invalid PID in state file");
    let namespaces = ["mnt","pid","net","ipc","uts"];

    for ns in namespaces {
        let ns_path = format!("/proc/{}/ns/{}", target_pid, ns);
        let fd = open(ns_path.as_str(), OFlag::O_RDONLY, Mode::empty())
            .expect(&format!("Failed to open {} namespace", ns));
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd) };
        setns(borrowed_fd, CloneFlags::empty()).expect(&format!("Failed to set {} namespace", ns));
        let _ = nix::unistd::close(fd);
    }
    chdir("/").expect("Failed to change directory to root");
    let mut child_cmd = Command::new(command);
    child_cmd.args(cmd_args);
    let err = child_cmd.exec();
    eprintln!("Command exited with error: {}", err);
    std::process::exit(1);
}
fn cleanup_container_resources(id: &str) {
    println!("Cleaning up for container {}", id);
    let rootfs = Path::new("/app/crater_rootfs");
    let _ = umount2(rootfs, MntFlags::MNT_DETACH);
    delete_state(id);
    println!("Containe {} successfully deleted", id);
}

fn start_container(container_id: &str) {
    let state_dir = get_container_dir(container_id);
    let fifo_path = state_dir.join("sync.fifo");

    if !fifo_path.exists() {
        eprintln!("Container {} is not in crated state or fifo is missing", container_id);
        std::process::exit(1);
    }
    println!("Crater: sending start signal to container {}", container_id);

    let mut fifo = OpenOptions::new()
        .write(true)
        .open(&fifo_path)
        .expect("Unable to open fifo for writing");

    fifo.write_all(b"GO\n").expect("Unable to write to fifo");
    println!("Crater: sent start signal to container {}", container_id);

}

fn get_container_dir(id: &str) -> PathBuf {
    Path::new("/var/run/crater").join(id)
}

fn create_container(container_id: &str) {
    println!("Crater: Creating container {}", container_id);
    let state_dir = get_container_dir(container_id);
    fs::create_dir_all(&state_dir).expect("Failed to create state directory");

    let fifo_path = state_dir.join("sync.fifo");
    if fifo_path.exists() {
        fs::remove_file(&fifo_path).expect("Failed to remove old fifo");
    }

    mkfifo(&fifo_path,Mode::S_IRWXU).expect("failed to create fifo");
    let spec = match Spec::load("config.json"){
        Ok(spec) => spec,
        Err(e) => {
            eprintln!("Failed to load config.json: {}", e);
            std::process::exit(1);
        }
    };



    let root_path_str = spec.root().as_ref()
        .map(|r| r.path().to_str().unwrap())
        .expect("Root path not defined in config.json");

    let rootfs = Path::new(root_path_str);

    // println!("Configuration loaded. Command: {} {:?}",cmd,cmd_args);

    let flags = CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS | CloneFlags::CLONE_NEWNET;
    println!("Successfully isolated namespaces!");
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            println!("Parent: Setting up cgroups for child {}", child);

            if let Err(e) = save_state(container_id, child) {
                eprintln!("Failed to save state: {}", e);
            }

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
            println!("Container created with the pid {}. Exiting.", child);
            std::process::exit(0);

            // println!("Parent: Waiting for container process {} to finish...", child);
            // let wait_result = waitpid(child, None);
            // if cgroup_path.exists() {
            //     let _ = fs::remove_dir(cgroup_path);
            // }
            // delete_state(container_id);
            // println!("Parent: Container finished. Exiting.");
            //
            // match wait_result {
            //     Ok(status) => println!("Child exited with status : {:?}", status),
            //     Err(e) => println!("Error waiting for child: {}", e)
            // }

        }
        Ok(ForkResult::Child) => {
            println!("Child: Setting up isolated environment...");
            unshare(flags).expect("Unshare Failed!");
            let mut fifo = File::open(&fifo_path).expect("Failed to open fifo");
            let process = spec.process().as_ref().expect("Failed to get process from config.json");
            let args = process.args().as_ref().expect("Failed to get args from process");

            if args.is_empty() {
                panic!("config.json process.args must not be empty");
            }

            let cmd = args[0].clone();
            let cmd_args = args[1..].to_vec();

            let env_vars = process.env().as_ref().expect("Failed to get envs from process");

            // sethostname(hostname).expect("Failed to set hostname");
            sethostname(spec.hostname().as_ref().expect("Failed to get hostname")).expect("unable to set hostname");
            setup_rootfs_and_mounts(&spec);
            println!("Child: Environment is ready. waiting for the 'start' signal...");

            let mut buf = [0u8;2];
            fifo.read_exact(&mut buf).expect("Failed to read fifo");

            let _ = fs::remove_file(&fifo_path);

            println!("Child: Signal received! Executing command: {}", cmd);
            // if let Err(e) = Command::new("/bin/sh")
            //     .args(&["-c", "ip link set up dev lo || ifconfig lo up"])
            //     .output()
            // {
            //     eprintln!("Child: Warning - Failed to set up loopback device: {}", e);
            // }
            //
            let cwd = process.cwd();


            // println!("Child: Environment isolated. Launching command: {}...", cmd);
            let mut child_cmd = Command::new(&cmd);
            child_cmd.args(&cmd_args);
            child_cmd.current_dir(cwd);

            let user = process.user();
            println!("Child: Running as user: uid={} gid={}", user.uid(), user.gid());
            child_cmd.uid(user.uid());
            child_cmd.gid(user.gid());

            child_cmd.env_clear();
            for env_var in env_vars {
                if let Some((key, value)) = env_var.split_once('=') {
                    child_cmd.env(key, value);
                } else {
                    panic!("Invalid environment variable: {}", env_var);
                }
            }

            // println!("Child setup is complete..waiting for start signal");
            //
            // let mut buff = [0u8;1];
            //
            // nix::unistd::read(sync_read.as_raw_fd(), &mut buff).expect("Failed to read loop");
            // println!("Child: Signal received!..executing process");

            let err = child_cmd.exec();
            eprintln!("Child: Command exited with error: {}", err);
            std::process::exit(1);
            // let proc_path = Path::new("/proc");
            // if !proc_path.exists() {
            //     let _ = fs::create_dir(proc_path);
            // }
            //
            // mount(
            //     Some("proc"),
            //     "/proc",
            //     Some("proc"),
            //     MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
            //     None::<&str>
            // ).expect("Failed to mount /proc");
            //
            // let sys_path = Path::new("/sys");
            // if !sys_path.exists() {
            //     let _ = fs::create_dir(sys_path);
            // }
            // mount(
            //     Some("sysfs"),
            //     "/sys",
            //     Some("sysfs"),
            //     MsFlags::MS_RDONLY | MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
            //     None::<&str>
            // ).expect("Failed to mount /sys");


        }
        Err(e) => panic!("Fork failed: {}", e),
    }
}

fn setup_rootfs_and_mounts(spec: &Spec) {
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

    if let Some(mounts) = spec.mounts() {
        for m in mounts {
            let dest_str = m.destination().to_str().expect("Mount destination must be valid UTF-8");
            let dest = dest_str.trim_start_matches('/');
            let target = rootfs.join(dest);

            if !target.exists() {
                let _ = fs::create_dir_all(&target);
            }

            let (flags, data) = parse_mount_opts(&m.options());
            let fstype = m.typ().as_deref();

            mount(
                m.source().as_deref(),
                &target,
                fstype,
                flags,
                Some(data.as_str())
            ).unwrap_or_else(|e| eprintln!("Warning: Failed to mount {:?}: {}", target, e));
        }
    }


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
}

fn save_state(container_id: &str, pid: Pid) -> std::io::Result<()> {
    let state_dir = Path::new("/var/run/crater").join(container_id);
    fs::create_dir_all(&state_dir)?;
    fs::write(state_dir.join("pid"),pid.as_raw().to_string())
}

fn delete_state(container_id: &str) {
    let state_dir = Path::new("/var/run/crater").join(container_id);
    let _ = fs::remove_dir_all(state_dir);
}

fn kill_container(container_id: &str, signal_str: &str) {
    let state_dir = Path::new("/var/run/crater").join(container_id);
    let pid_path = state_dir.join("pid");

    if !pid_path.exists() {
        eprintln!("Container {} not found (is it running?)", container_id);
        std::process::exit(1);
    }

    let pid_content = fs::read_to_string(&pid_path).expect("Failed to read PID file");
    let pid_int: i32 = pid_content.trim().parse().expect("Invalid PID in state file");
    let pid = Pid::from_raw(pid_int);

    let signal = match signal_str.to_uppercase().as_str() {
        "SIGTERM" | "TERM" | "15" => Signal::SIGTERM,
        "SIGKILL" | "KILL" | "9" => Signal::SIGKILL,
        "SIGINT" | "INT" | "2" => Signal::SIGINT,
        _ => {
            eprintln!("Unsupported signal: {}. Defaulting to SIGTERM.", signal_str);
            Signal::SIGTERM
        }
    };

    if let Err(e) = signal::kill(pid, signal) {
        eprintln!("Failed to send signal {:?} to container {}: {}", signal, container_id, e);
        std::process::exit(1);
    }
    println!("Sent signal {:?} to container {}", signal, container_id);
}

// fn run_container(container_id: &str) {
// if args.len() < 3 || args[1] != "run" {
//         eprintln!("Usage: {} run <container_id>", args.get(0).unwrap_or(&String::from("crater")));
//         std::process::exit(1);
//     }
//
//     let container_id = &args[2];
//     run_container(container_id);
// }

fn run_container(container_id: &str) {
    println!("Crater Runtime Starting. Container ID: {}", container_id);

    let (sync_read, sync_write) = pipe().expect("Failed to create pipe");






}