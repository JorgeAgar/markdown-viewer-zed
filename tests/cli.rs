//! End-to-end tests of the `mdview` binary.
//!
//! The default tests never open a window: they cover `--help` and the warm
//! path, where mdview hands its paths to a running instance over the Unix
//! socket and exits. A fake "running instance" (a `UnixListener` in a temp
//! `XDG_RUNTIME_DIR`) receives them, and the display variables are removed so
//! a regression can't pop up a window.
//!
//! The window tests (`window_*`) open real windows for a moment, so they only
//! run with `MDVIEW_E2E=1` and a display (`WAYLAND_DISPLAY` or `DISPLAY`):
//!
//! ```sh
//! MDVIEW_E2E=1 cargo test --test cli
//! ```
//!
//! Wire format (src/main.rs): the client connects to
//! `$XDG_RUNTIME_DIR/mdview.sock` (or `$TMPDIR/mdview-$USER.sock`), writes the
//! absolute paths joined by '\n', and closes the connection.

use std::{
    io::{BufRead, BufReader, Read},
    os::unix::{fs::MetadataExt, net::UnixListener},
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_mdview");
/// The warm path should take milliseconds; this only catches hangs and
/// accidental GUI startup, with plenty of slack for a loaded CI machine.
const WARM_TIMEOUT: Duration = Duration::from_secs(5);

// ------------------------------------------------------------------- --help

#[test]
fn help_prints_usage_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let rt = TempDir::new("rt");
        let listener = UnixListener::bind(rt.join("mdview.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let out = run(mdview(&rt).arg(flag), WARM_TIMEOUT);
        assert!(out.status.success(), "mdview {flag}: {}", describe(&out));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.starts_with("mdview") && stdout.contains("USAGE") && stdout.contains("--new"),
            "mdview {flag} printed unexpected help:\n{stdout}"
        );
        assert!(
            out.stderr.is_empty(),
            "mdview {flag} wrote to stderr: {}",
            describe(&out)
        );
        // Help is answered before the socket check: nothing is sent.
        assert!(
            listener.accept().is_err(),
            "mdview {flag} connected to the running instance"
        );
    }
}

#[test]
fn help_wins_over_paths() {
    let rt = TempDir::new("rt");
    let listener = UnixListener::bind(rt.join("mdview.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let out = run(mdview(&rt).args(["a.md", "-h", "b.md"]), WARM_TIMEOUT);
    assert!(out.status.success(), "{}", describe(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("USAGE"));
    assert!(listener.accept().is_err(), "paths were sent despite -h");
}

// ---------------------------------------------------------------- warm path

#[test]
fn warm_path_sends_normalized_paths() {
    let work = TempDir::new("work");
    std::fs::create_dir(work.join("sub")).unwrap();
    std::fs::write(work.join("sub/a.md"), "# a").unwrap();
    std::fs::write(work.join("b.md"), "# b").unwrap();

    let got = handoff(&work.join("sub"), &["a.md", "../b.md"]);
    assert_eq!(
        got,
        [canon(&work.join("sub/a.md")), canon(&work.join("b.md"))],
        "relative paths must be sent as absolute, canonical paths"
    );
}

#[test]
fn warm_path_keeps_order_and_absolute_paths() {
    let work = TempDir::new("work");
    let names = ["3.md", "1.md", "2.md"];
    for n in names {
        std::fs::write(work.join(n), n).unwrap();
    }
    let abs = canon(&work.join("1.md"));
    let got = handoff(
        work.path(),
        &["3.md", abs.to_str().unwrap(), "2.md", "./3.md"],
    );
    let want: Vec<PathBuf> = ["3.md", "1.md", "2.md", "3.md"]
        .iter()
        .map(|n| canon(&work.join(n)))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn warm_path_sends_nonexistent_files_as_absolute_paths() {
    // The running instance shows an error tab for them, so they must still be
    // sent, resolved against the client's cwd (not the server's).
    let work = TempDir::new("work");
    std::fs::create_dir(work.join("sub")).unwrap();
    let got = handoff(
        &work.join("sub"),
        &["missing.md", "../gone/also-missing.md"],
    );
    assert_eq!(got.len(), 2, "{got:?}");
    for p in &got {
        assert!(p.is_absolute(), "{} isn't absolute", p.display());
    }
    assert_eq!(
        lexical(&got[0]),
        lexical(&canon(work.path()).join("sub/missing.md"))
    );
    assert_eq!(
        lexical(&got[1]),
        lexical(&canon(work.path()).join("gone/also-missing.md"))
    );
}

#[test]
fn warm_path_handles_spaces_and_unicode() {
    let work = TempDir::new("work");
    let dir = work.join("my notes ✓");
    std::fs::create_dir(&dir).unwrap();
    let names = [
        "ñandú café.md",
        "日本語.md",
        "emoji 🦀.md",
        "  leading space.md",
    ];
    for n in names {
        std::fs::write(dir.join(n), n).unwrap();
    }
    let got = handoff(&dir, &names);
    let want: Vec<PathBuf> = names.iter().map(|n| canon(&dir.join(n))).collect();
    assert_eq!(got, want);
}

#[test]
fn warm_path_with_no_files_still_hands_off() {
    // `mdview` alone, with an instance running, just focuses that window.
    let rt = TempDir::new("rt");
    let (status, msgs) = handoff_raw(&rt, rt.path(), &[], &[]);
    assert!(status.success());
    assert_eq!(msgs, [""], "expected one empty message");
}

#[test]
fn warm_path_falls_back_to_tmpdir_without_xdg_runtime_dir() {
    let tmp = TempDir::new("tmpdir");
    std::fs::write(tmp.join("x.md"), "x").unwrap();
    let listener = UnixListener::bind(tmp.join("mdview-mdview-test.sock")).unwrap();
    let mut cmd = Command::new(BIN);
    cmd.current_dir(&tmp)
        .arg("x.md")
        .env_remove("XDG_RUNTIME_DIR")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env("TMPDIR", tmp.path())
        .env("USER", "mdview-test");
    let (out, msgs) = serve(listener, cmd, 1);
    assert!(out.success());
    assert_eq!(msgs, [canon(&tmp.join("x.md")).display().to_string()]);
}

#[test]
fn unknown_flags_are_not_silently_dropped() {
    // mdview has no other flags today, so `--frobnicate` is treated as a file
    // name. Rejecting it with an error would also be fine; what must not
    // happen is a crash, a hang or a window.
    let rt = TempDir::new("rt");
    let listener = UnixListener::bind(rt.join("mdview.sock")).unwrap();
    let mut cmd = mdview(&rt);
    cmd.current_dir(&rt).arg("--frobnicate");
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            let _ = tx.send(buf);
        }
    });
    let out = wait_with_timeout(child, WARM_TIMEOUT);
    match rx.recv_timeout(Duration::from_millis(500)) {
        Ok(msg) => {
            assert!(out.status.success(), "{}", describe(&out));
            assert_eq!(
                msg,
                canon(rt.path()).join("--frobnicate").display().to_string()
            );
        }
        Err(_) => {
            assert_eq!(
                out.status.code(),
                Some(2),
                "rejecting a flag should exit 2: {}",
                describe(&out)
            );
            assert!(String::from_utf8_lossy(&out.stderr).contains("--frobnicate"));
        }
    }
}

#[test]
fn stale_socket_is_replaced() {
    // A crashed instance leaves mdview.sock behind with nobody listening.
    // mdview must not hang on it or treat it as a running instance: it removes
    // it and becomes the server itself. With no display it then fails to open
    // a window, which is fine here.
    let rt = TempDir::new("rt");
    let sock = rt.join("mdview.sock");
    drop(UnixListener::bind(&sock).unwrap());
    let stale_ino = std::fs::metadata(&sock).unwrap().ino();

    let out = run(
        mdview(&rt).current_dir(&rt).arg("a.md"),
        Duration::from_secs(20),
    );
    assert!(
        !out.status.success(),
        "without a display the cold path should fail: {}",
        describe(&out)
    );
    let ino = std::fs::metadata(&sock).map(|m| m.ino()).ok();
    assert_ne!(ino, Some(stale_ino), "the stale socket was left in place");
}

// ------------------------------------------------------------- window tests

/// One window at a time: parallel windows would compete for the GPU and make
/// the timings meaningless.
static WINDOW: Mutex<()> = Mutex::new(());

fn window_tests_enabled() -> bool {
    let on = std::env::var_os("MDVIEW_E2E").is_some_and(|v| v != "0" && !v.is_empty());
    let display = ["WAYLAND_DISPLAY", "DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|v| !v.is_empty()));
    if on && !display {
        eprintln!("MDVIEW_E2E is set but there's no WAYLAND_DISPLAY or DISPLAY: skipping");
    }
    on && display
}

const WINDOW_TIMEOUT: Duration = Duration::from_secs(60);

/// `MDVIEW_TRACE_STARTUP=quit mdview -n FILES…` must render a first frame and
/// exit 0. Returns the reported first-frame time.
fn first_frame(files: &[&Path]) -> Duration {
    let _guard = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    let mut cmd = Command::new(BIN);
    cmd.arg("-n")
        .args(files)
        .env("MDVIEW_TRACE_STARTUP", "quit");
    let out = run(&mut cmd, WINDOW_TIMEOUT);
    let what = format!("mdview -n {files:?}");
    assert!(out.status.success(), "{what}: {}", describe(&out));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let line = stderr
        .lines()
        .find_map(|l| l.strip_prefix("first frame: "))
        .unwrap_or_else(|| panic!("{what}: no 'first frame:' line: {}", describe(&out)));
    parse_debug_duration(line).unwrap_or_else(|| panic!("{what}: bad duration {line:?}"))
}

#[test]
fn window_renders_every_example() {
    if !window_tests_enabled() {
        return;
    }
    let examples = repo().join("examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&examples)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    for f in &files {
        let t = first_frame(&[f.as_path()]);
        eprintln!("{}: first frame {t:?}", f.display());
    }
    let all: Vec<&Path> = files.iter().map(PathBuf::as_path).collect();
    first_frame(&all);
}

#[test]
fn window_survives_odd_inputs() {
    if !window_tests_enabled() {
        return;
    }
    let dir = TempDir::new("odd");
    let empty = dir.join("empty.md");
    std::fs::write(&empty, "").unwrap();
    let binary = dir.join("binary.md");
    std::fs::write(
        &binary,
        (0..=255u8).cycle().take(64 * 1024).collect::<Vec<u8>>(),
    )
    .unwrap();
    let missing = dir.join("does-not-exist.md");
    let subdir = dir.join("a-directory.md");
    std::fs::create_dir(&subdir).unwrap();
    let deep = dir.join("deep.md");
    std::fs::write(
        &deep,
        format!(
            "{}deep quote\n\n{}deep list\n\n{}\n",
            "> ".repeat(500),
            (0..200)
                .map(|i| format!("{}- item\n", "  ".repeat(i)))
                .collect::<String>(),
            "[".repeat(5000)
        ),
    )
    .unwrap();

    for f in [&empty, &binary, &missing, &subdir, &deep] {
        first_frame(&[f.as_path()]);
    }
    first_frame(&[]); // no files: an empty window
    first_frame(&[empty.as_path(), missing.as_path(), binary.as_path()]);
}

#[test]
fn window_opens_a_huge_file_quickly() {
    if !window_tests_enabled() {
        return;
    }
    let dir = TempDir::new("huge");
    let huge = dir.join("huge.md");
    let demo = std::fs::read_to_string(repo().join("examples/demo.md")).unwrap();
    let mut text = String::new();
    let mut i = 0;
    while text.len() < 8 << 20 {
        text.push_str(&format!("\n# Section {i}\n\n"));
        text.push_str(&demo);
        i += 1;
    }
    std::fs::write(&huge, &text).unwrap();
    // Only a few screens are laid out (virtualized list), so even 8 MB must
    // show its first frame in reasonable time. Generous bound: catches
    // "lays out the whole document" regressions, not small slowdowns.
    let t = first_frame(&[&huge]);
    eprintln!("8 MB file: first frame {t:?}");
    assert!(t < Duration::from_secs(20), "first frame took {t:?}");
}

#[test]
fn window_instance_accepts_handoff_and_replaces_stale_socket() {
    if !window_tests_enabled() {
        return;
    }
    let _guard = WINDOW.lock().unwrap_or_else(|e| e.into_inner());
    let rt = TempDir::new("rt");
    let sock = rt.join("mdview.sock");
    drop(UnixListener::bind(&sock).unwrap()); // stale socket
    let a = repo().join("examples/demo.md");
    let b = repo().join("examples/second.md");

    // Instance 1: no -n, so it owns the socket. Keeps running.
    let mut server = KillOnDrop(
        display_env(&mut mdview(&rt))
            .arg(&a)
            .env("MDVIEW_TRACE_STARTUP", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stderr = server.0.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + WINDOW_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(l) if l.starts_with("first frame") => break,
            Ok(_) => {}
            Err(_) => panic!("the first instance never showed a frame"),
        }
    }

    // Instance 2: must hand off and exit, not open a second window (it has no
    // MDVIEW_TRACE_STARTUP=quit, so a window would never exit by itself).
    let start = Instant::now();
    let out = run(display_env(&mut mdview(&rt)).arg(&b), WARM_TIMEOUT);
    assert!(out.status.success(), "handoff failed: {}", describe(&out));
    eprintln!("handoff to a real instance took {:?}", start.elapsed());
    assert!(
        server.0.try_wait().unwrap().is_none(),
        "the first instance exited"
    );
}

/// Restores the display variables removed by `mdview()`. A relative
/// WAYLAND_DISPLAY is resolved against the real XDG_RUNTIME_DIR, since the
/// test points XDG_RUNTIME_DIR at a temp dir.
fn display_env(cmd: &mut Command) -> &mut Command {
    if let Some(d) = std::env::var_os("DISPLAY") {
        cmd.env("DISPLAY", d);
    }
    if let Some(w) = std::env::var_os("WAYLAND_DISPLAY") {
        let w = PathBuf::from(w);
        let w = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(real) if w.is_relative() => PathBuf::from(real).join(w),
            _ => w,
        };
        cmd.env("WAYLAND_DISPLAY", w);
    }
    cmd
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// ------------------------------------------------------------------ helpers

/// A command for the warm path: socket in `rt`, no display.
fn mdview(rt: &TempDir) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env("XDG_RUNTIME_DIR", rt.path())
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env_remove("MDVIEW_TRACE_STARTUP");
    cmd
}

/// Runs `mdview ARGS` in `cwd` against a fake running instance and returns
/// the paths it received.
fn handoff(cwd: &Path, args: &[&str]) -> Vec<PathBuf> {
    let rt = TempDir::new("rt");
    let (status, msgs) = handoff_raw(&rt, cwd, args, &[]);
    assert!(status.success());
    assert_eq!(msgs.len(), 1, "expected exactly one connection: {msgs:?}");
    msgs[0].split('\n').map(PathBuf::from).collect()
}

fn handoff_raw(
    rt: &TempDir,
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> (ExitStatus, Vec<String>) {
    let listener = UnixListener::bind(rt.join("mdview.sock")).unwrap();
    let mut cmd = mdview(rt);
    cmd.current_dir(cwd).args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    serve(listener, cmd, 1)
}

/// Accepts `expect` connections while `cmd` runs; returns its status and what
/// each connection sent.
fn serve(listener: UnixListener, mut cmd: Command, expect: usize) -> (ExitStatus, Vec<String>) {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for _ in 0..expect {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut buf = String::new();
            s.set_read_timeout(Some(WARM_TIMEOUT)).unwrap();
            s.read_to_string(&mut buf).unwrap();
            if tx.send(buf).is_err() {
                return;
            }
        }
    });
    let start = Instant::now();
    let child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let out = wait_with_timeout(child, WARM_TIMEOUT);
    assert!(
        out.status.success(),
        "warm path failed ({:?}): {}",
        start.elapsed(),
        describe(&out)
    );
    let msgs: Vec<String> = (0..expect)
        .map(|_| {
            rx.recv_timeout(Duration::from_secs(2))
                .expect("mdview exited without sending anything to the running instance")
        })
        .collect();
    (out.status, msgs)
}

fn run(cmd: &mut Command, timeout: Duration) -> Output {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("can't run {BIN}: {e}"));
    wait_with_timeout(child, timeout)
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> Output {
    let (tx, rx) = mpsc::channel();
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let readers = thread::spawn(move || {
        let mut o = Vec::new();
        let mut e = Vec::new();
        let t = stderr.take().map(|mut s| {
            thread::spawn(move || {
                let mut b = Vec::new();
                let _ = s.read_to_end(&mut b);
                b
            })
        });
        if let Some(mut s) = stdout.take() {
            let _ = s.read_to_end(&mut o);
        }
        if let Some(t) = t {
            e = t.join().unwrap();
        }
        let _ = tx.send((o, e));
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let (o, e) = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
            panic!(
                "mdview didn't exit within {timeout:?} (killed).\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&o),
                String::from_utf8_lossy(&e)
            );
        }
        thread::sleep(Duration::from_millis(5));
    };
    let (stdout, stderr) = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = readers.join();
    Output {
        status,
        stdout,
        stderr,
    }
}

fn describe(out: &Output) -> String {
    format!(
        "status {}\n--- stdout\n{}--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Parses `Duration`'s Debug output: "85.3ms", "1.2s", "950µs".
fn parse_debug_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let v: f64 = num.parse().ok()?;
    let secs = match unit {
        "s" => v,
        "ms" => v / 1e3,
        "µs" | "us" => v / 1e6,
        "ns" => v / 1e9,
        _ => return None,
    };
    Some(Duration::from_secs_f64(secs))
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|e| panic!("canonicalize {}: {e}", p.display()))
}

/// Resolves `.` and `..` without touching the file system.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// A temp directory removed on drop. std only: the tests add no
/// dependencies.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "mdview-test-{}-{}-{tag}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn join(&self, p: impl AsRef<Path>) -> PathBuf {
        self.0.join(p)
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn helper_parses_durations() {
    assert_eq!(
        parse_debug_duration("85.5ms"),
        Some(Duration::from_micros(85_500))
    );
    assert_eq!(
        parse_debug_duration("1.5s"),
        Some(Duration::from_millis(1500))
    );
    assert_eq!(
        parse_debug_duration("950µs"),
        Some(Duration::from_micros(950))
    );
}
