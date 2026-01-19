use nix::mount::{mount, MsFlags};
use nix::sched::{unshare, CloneFlags};
use nix::unistd::{fork, ForkResult};
use nix::sys::wait::waitpid;
use std::process::Command;
use std::os::unix::process::CommandExt;

fn main() {
    let flags = CloneFlags::CLONE_NEWUTS | CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS;
    unshare(flags).expect("Unshare Failed!");
    println!("Successfully isolated namespaces!");

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            // Parent waits for the container (bash) to exit
            println!("Parent: Waiting for container process {} to finish...", child);
            waitpid(child, None).expect("Waitpid failed");
            println!("Parent: Container finished. Exiting.");
        }
        Ok(ForkResult::Child) => {
            println!("Child: Setting up isolated environment...");
            nix::unistd::sethostname("create-container").expect("Failed to set hostname");
            mount(
                Some("proc"),
                "/proc",
                Some("proc"),
                MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                None::<&str>,
            ).expect("Failed to mount /proc. If this fails try unmount /proc first with: sudo umount /proc");
            println!("Child: /proc remounted. Launching bash...");
            let mut child_cmd = Command::new("/bin/bash");


            let err = child_cmd.exec();
            eprintln!("Failed to exec bash: {}", err);
        }
        Err(e) => panic!("Fork failed: {}", e),
    }
}