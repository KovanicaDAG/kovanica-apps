//! Production `--parent-death-reaper` entry of the GUI binary.
//!
//! This does not open a window. It skips when the `gui` feature did not build
//! the binary. It is not physical-desktop evidence and it is not an L1 run.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn proc_start(pid: i32) -> Option<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rsplit_once(')')?.1;
    let mut fields = rest.split_whitespace();
    fields.next()?;
    fields.nth(18)?.parse().ok()
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

struct KillPid(u32);
impl Drop for KillPid {
    fn drop(&mut self) {
        if self.0 == 0 {
            return;
        }
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
            libc::kill(self.0 as i32, libc::SIGKILL);
        }
    }
}

fn spawn_parent_child(binary: &Path, pid_file: &Path) -> (Child, i32) {
    let mut parent = Command::new("python3")
        .arg("-c")
        .arg(
            "import os, sys, time\nbinary, pid_file = sys.argv[1], sys.argv[2]\npid = os.fork()\nif pid == 0:\n    os.setsid()\n    os.execv(binary, [binary])\nopen(pid_file, 'w').write(str(pid))\nwhile True:\n    time.sleep(0.05)\n",
        )
        .arg(binary)
        .arg(pid_file)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(text) = fs::read_to_string(pid_file) {
            if let Ok(pid) = text.trim().parse::<i32>() {
                if alive(pid as u32) {
                    return (parent, pid);
                }
            }
        }
        if Instant::now() >= deadline {
            let _ = parent.kill();
            let _ = parent.wait();
            panic!("parent did not publish a child");
        }
        match parent.try_wait() {
            Ok(Some(_)) => panic!("parent exited before publishing a child"),
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => panic!("could not wait for the parent"),
        }
    }
}

fn spawn_leader(binary: &Path) -> Child {
    let mut command = Command::new(binary);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().unwrap()
}

fn reaper(bin: &str, parent: i32, parent_started: u64, node: i32, node_started: u64) -> Child {
    Command::new(bin)
        .arg("--parent-death-reaper")
        .arg(parent.to_string())
        .arg(parent_started.to_string())
        .arg(node.to_string())
        .arg(node_started.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn finish(child: &mut Child) -> (std::process::ExitStatus, String) {
    let status = child.wait().unwrap();
    let mut stderr = String::new();
    if let Some(pipe) = child.stderr.as_mut() {
        pipe.read_to_string(&mut stderr).unwrap();
    }
    (status, stderr)
}

#[test]
fn production_binary_reaper_stops_only_the_adopted_child() {
    let Some(bin) = option_env!("CARGO_BIN_EXE_kovanica-node-desktop") else {
        eprintln!("skipping production reaper test: GUI binary was not built");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let hold = script(
        tmp.path(),
        "hold.sh",
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
    );
    let stubborn = script(
        tmp.path(),
        "stubborn.py",
        "#!/usr/bin/env python3\nimport signal, time\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nwhile True:\n    time.sleep(0.05)\n",
    );
    let bystander_bin = script(
        tmp.path(),
        "bystander.sh",
        "#!/bin/sh\nwhile true; do sleep 0.05; done\n",
    );

    let (mut parent, node) = spawn_parent_child(&hold, &tmp.path().join("child.pid"));
    let _node_guard = KillPid(node as u32);
    let parent_pid = parent.id() as i32;
    let parent_started = proc_start(parent_pid).expect("parent start");
    let node_started = proc_start(node).expect("node start");
    let mut bystander = Command::new(&bystander_bin).spawn().unwrap();
    let by_pid = bystander.id();
    let mut leader = spawn_leader(&bystander_bin);
    let leader_pid = leader.id() as i32;
    let leader_started = proc_start(leader_pid).expect("leader start");

    let mut unrelated = reaper(bin, parent_pid, parent_started, leader_pid, leader_started);
    let (status, stderr) = finish(&mut unrelated);
    assert!(status.success(), "{status} {stderr}");
    assert!(
        stderr.contains("not adopting"),
        "unrelated group leader was adopted: {stderr}"
    );
    assert!(alive(leader.id()));
    assert!(alive(node as u32));
    assert!(alive(by_pid));

    let mut watcher = reaper(bin, parent_pid, parent_started, node, node_started);
    thread::sleep(Duration::from_millis(200));
    assert!(
        alive(node as u32),
        "reaper stopped the child while its parent was alive"
    );
    unsafe {
        libc::kill(parent_pid, libc::SIGKILL);
    }
    let _ = parent.wait();
    let began = Instant::now();
    let (status, stderr) = finish(&mut watcher);
    assert!(
        began.elapsed() < Duration::from_secs(5),
        "a child that exits on SIGTERM waited out the 10s grace"
    );
    assert!(status.success(), "{status} {stderr}");
    assert!(stderr.contains("stop outcome Exited"), "{stderr}");
    assert!(!stderr.contains("stop outcome Killed"), "{stderr}");
    assert!(!alive(node as u32), "adopted child survived parent death");
    assert!(alive(by_pid), "bystander was signaled");
    assert!(alive(leader.id()), "unrelated group leader was signaled");
    let _ = bystander.kill();
    let _ = bystander.wait();
    let _ = leader.kill();
    let _ = leader.wait();

    let (mut parent, node) = spawn_parent_child(&stubborn, &tmp.path().join("stubborn.pid"));
    let _node_guard = KillPid(node as u32);
    let parent_pid = parent.id() as i32;
    let parent_started = proc_start(parent_pid).expect("parent start");
    let node_started = proc_start(node).expect("node start");
    let mut bystander = spawn_leader(&bystander_bin);
    let by_pid = bystander.id();
    let mut watcher = reaper(bin, parent_pid, parent_started, node, node_started);
    thread::sleep(Duration::from_millis(200));
    assert!(alive(node as u32), "reaper stopped a stubborn child early");
    unsafe {
        libc::kill(parent_pid, libc::SIGKILL);
    }
    let _ = parent.wait();
    let began = Instant::now();
    let (status, stderr) = finish(&mut watcher);
    let elapsed = began.elapsed();
    assert!(status.success(), "{status} {stderr}");
    assert!(stderr.contains("stop outcome Killed"), "{stderr}");
    assert!(
        elapsed >= Duration::from_secs(10),
        "production reaper did not keep the 10 second grace: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "production reaper did not escalate after the grace: {elapsed:?}"
    );
    assert!(!alive(node as u32));
    assert!(alive(by_pid), "escalation killed a bystander");
    let _ = bystander.kill();
    let _ = bystander.wait();
}
