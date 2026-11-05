//! Native Windows binary handoff. A fake server keeps these tests GUI-free.
#![cfg(windows)]
#[allow(dead_code)]
#[path = "../src/platform/windows.rs"]
mod ipc;

use std::{
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

fn run(args: &[&std::ffi::OsStr]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mdview"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            panic!("CLI launched GUI or hung instead of handing off");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn binary_help_and_handoff_preserve_paths() {
    let ipc::Instance::Primary(Some(server)) = ipc::start(false, &[]) else {
        panic!("isolated test runner must own the pipe");
    };
    let help = run(&["--help".as_ref()]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("USAGE"));
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        server.serve(|paths| {
            tx.send(paths).unwrap();
            false
        })
    });
    let path = PathBuf::from("C:\\mdview test\\日本語.md");
    let result = run(&[path.as_os_str()]);
    assert!(result.status.success(), "{:?}", result.stderr);
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), vec![path]);
    thread.join().unwrap();
    ipc::cleanup();
}
