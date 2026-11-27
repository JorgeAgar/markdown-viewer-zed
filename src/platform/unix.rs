//! Unix single-instance transport. Keep the handoff ahead of GPUI startup.
//!
//! The warm path is one `connect` and one `write` (plus one `lstat` of the
//! private directory when `XDG_RUNTIME_DIR` is unset). Only a launch that
//! finds nobody listening takes `mdview.lock`, so two launches can't both
//! decide the socket is stale and unlink each other's live socket.
use crate::cli::{self, SocketDir};
use std::{
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const SOCKET: &str = "mdview.sock";
const LOCK: &str = "mdview.lock";
/// How long the listener waits for one client's whole message. Real clients
/// write it right after connecting; this only bounds a stalled one.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

unsafe extern "C" {
    /// POSIX `getuid(2)`, which can't fail. `uid_t` is `u32` on every Unix
    /// Rust supports. Declared here so it costs no dependency.
    safe fn getuid() -> u32;
}

pub enum Instance {
    Forwarded,
    Primary(Option<Server>),
}

pub struct Server {
    listener: UnixListener,
    timeout: Duration,
}

/// The socket this process bound, so [`cleanup`] only unlinks it while it's
/// still that socket. The listener copy keeps its inode alive, so the
/// `(dev, ino)` pair can't be reused by a newer instance's socket.
struct Owned {
    dir: PathBuf,
    dev: u64,
    ino: u64,
    _listener: UnixListener,
}

static OWNED: Mutex<Option<Owned>> = Mutex::new(None);

enum Claim {
    Forwarded,
    Bound(Server, Owned),
}

fn socket_dir() -> io::Result<PathBuf> {
    match cli::socket_dir_in(std::env::var_os("XDG_RUNTIME_DIR"), || {
        (std::env::temp_dir(), getuid())
    }) {
        SocketDir::Runtime(dir) => Ok(dir),
        SocketDir::Private(dir) => private_dir(&dir, getuid()).map(|()| dir),
    }
}

/// Makes sure `dir` is a real directory (not a symlink) owned by `uid` that
/// nobody else can enter, creating it with mode 0700 if it's missing. In a
/// shared temp dir anyone can create it first, so it's checked either way.
fn private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    // Usually it exists: one lstat on the warm path. If mkdir fails (say
    // another launch just made it), the second lstat and the checks decide.
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(_) => {
            let _ = fs::DirBuilder::new().mode(0o700).create(dir);
            fs::symlink_metadata(dir)?
        }
    };
    let unsafe_dir = |why: String| {
        let msg = format!("{} {why}", dir.display());
        Err(io::Error::new(io::ErrorKind::PermissionDenied, msg))
    };
    if !meta.is_dir() {
        unsafe_dir("is a symlink or not a directory".into())
    } else if meta.uid() != uid {
        unsafe_dir(format!("is owned by uid {}, not {uid}", meta.uid()))
    } else if meta.mode() & 0o077 != 0 {
        unsafe_dir(format!("has mode {:o}, not 700", meta.mode() & 0o7777))
    } else {
        Ok(())
    }
}

/// Holds `dir/mdview.lock` until the returned file is dropped. The file is
/// never deleted: unlinking a lock file lets two processes lock two files.
fn lock(dir: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(LOCK))?;
    file.lock()?;
    Ok(file)
}

pub fn start(standalone: bool, paths: &[PathBuf]) -> Instance {
    if standalone {
        return Instance::Primary(None);
    }
    let Some(message) = cli::encode_paths(paths) else {
        eprintln!("mdview: too many paths to hand off; opening a separate window");
        return Instance::Primary(None);
    };
    let claim = socket_dir().and_then(|dir| claim(&dir, &message));
    match claim {
        Ok(Claim::Forwarded) => Instance::Forwarded,
        Ok(Claim::Bound(server, owned)) => {
            *OWNED.lock().unwrap_or_else(|e| e.into_inner()) = Some(owned);
            Instance::Primary(Some(server))
        }
        Err(e) => {
            eprintln!("mdview: single-instance socket unavailable: {e}");
            Instance::Primary(None)
        }
    }
}

/// Hands `message` to the instance listening in `dir`, or becomes it.
fn claim(dir: &Path, message: &[u8]) -> io::Result<Claim> {
    let sock = dir.join(SOCKET);
    let send = |mut stream: UnixStream| stream.write_all(message).map(|()| Claim::Forwarded);
    if let Ok(claim) = UnixStream::connect(&sock).and_then(send) {
        return Ok(claim); // the warm path: no lock
    }
    let lock = lock(dir)?;
    match UnixStream::connect(&sock) {
        // Another launch bound the socket since the first try: it's the
        // running instance. Write outside the lock.
        Ok(stream) => {
            drop(lock);
            return send(stream);
        }
        // Nobody listens. Every bind and unlink happens under the lock, so
        // whatever is there is stale and stays that way until we bind. If
        // the unlink fails, bind says why.
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
            let _ = fs::remove_file(&sock);
        }
        // Missing, or there but unusable (e.g. no permission): bind either
        // succeeds or reports it. Never unlink what we couldn't test.
        Err(_) => {}
    }
    let listener = UnixListener::bind(&sock)?;
    fs::set_permissions(&sock, Permissions::from_mode(0o600))?;
    let meta = fs::symlink_metadata(&sock)?;
    let owned = Owned {
        dir: dir.to_path_buf(),
        dev: meta.dev(),
        ino: meta.ino(),
        _listener: listener.try_clone()?,
    };
    let server = Server {
        listener,
        timeout: READ_TIMEOUT,
    };
    Ok(Claim::Bound(server, owned))
}

impl Server {
    pub fn serve(self, mut send: impl FnMut(Vec<PathBuf>) -> bool) {
        for mut stream in self.listener.incoming().flatten() {
            let deadline = Instant::now() + self.timeout;
            if let Ok(paths) = read_message(&mut stream, deadline)
                && !send(paths)
            {
                break;
            }
        }
    }
}

/// Reads one [`cli::encode_paths`] message, giving up at `deadline`. The
/// header is checked against [`cli::MAX_MESSAGE`] before anything is
/// allocated, and the body buffer only grows as bytes arrive, so a lying
/// header can't make it allocate.
fn read_message(stream: &mut UnixStream, deadline: Instant) -> io::Result<Vec<PathBuf>> {
    let mut reader = Deadline { stream, deadline };
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = cli::body_len(header).ok_or(io::ErrorKind::InvalidData)?;
    let mut body = Vec::new();
    reader.take(len as u64).read_to_end(&mut body)?;
    if body.len() != len {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    cli::decode_paths(&body).ok_or_else(|| io::ErrorKind::InvalidData.into())
}

/// Reads that fail at `deadline`, however slowly the bytes trickle in (a
/// plain read timeout restarts with every byte).
struct Deadline<'a> {
    stream: &'a mut UnixStream,
    deadline: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if self.stream.set_read_timeout(Some(left)).is_err() {
            // macOS rejects SO_RCVTIMEO (EINVAL) once the client has closed
            // its end. Everything it sent is buffered by then: don't wait.
            self.stream.set_nonblocking(true)?;
        }
        self.stream.read(buf)
    }
}

/// Unlinks the socket this process bound, unless a newer instance has
/// replaced it (it may have, once this one stopped listening).
pub fn cleanup() {
    if let Some(owned) = OWNED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        owned.release();
    }
}

impl Owned {
    fn release(self) {
        // The lock keeps a starting instance from replacing the socket
        // between the check and the unlink.
        let _lock = lock(&self.dir);
        let sock = self.dir.join(SOCKET);
        if fs::symlink_metadata(&sock).is_ok_and(|m| (m.dev(), m.ino()) == (self.dev, self.ino)) {
            let _ = fs::remove_file(&sock);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;
    use std::{
        ffi::OsStr,
        os::unix::ffi::OsStrExt,
        sync::{Arc, Barrier, mpsc},
        thread,
    };

    const WAIT: Duration = Duration::from_secs(5);

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o7777
    }

    fn ino(path: &Path) -> u64 {
        fs::symlink_metadata(path).unwrap().ino()
    }

    fn message(paths: &[&str]) -> Vec<u8> {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        cli::encode_paths(&paths).unwrap()
    }

    fn bound(dir: &Path) -> (Server, Owned) {
        match claim(dir, &message(&[])).unwrap() {
            Claim::Bound(server, owned) => (server, owned),
            Claim::Forwarded => panic!("forwarded instead of binding"),
        }
    }

    fn server_with_timeout(path: &Path, timeout: Duration) -> Server {
        Server {
            listener: UnixListener::bind(path).unwrap(),
            timeout,
        }
    }

    /// Serves on a thread, forwarding every message, until `n` arrived.
    fn serve_n(server: Server, n: usize) -> mpsc::Receiver<Vec<PathBuf>> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut left = n;
            server.serve(|paths| {
                tx.send(paths).unwrap();
                left -= 1;
                left > 0
            });
        });
        rx
    }

    /// Runs one of the `*_child` tests in a fresh process, where changing
    /// the environment can't race the other tests.
    fn child(name: &str, env: &[(&str, &OsStr)], remove: &[&str]) {
        let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", &format!("platform::ipc::tests::{name}")]);
        cmd.env("MDVIEW_TEST_CHILD", name);
        for (k, v) in env {
            cmd.env(k, v);
        }
        for k in remove {
            cmd.env_remove(k);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{name} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn in_child(name: &str) -> bool {
        std::env::var_os("MDVIEW_TEST_CHILD").is_some_and(|v| v == name)
    }

    // ------------------------------------------------------------ claiming

    #[test]
    fn first_claim_binds_a_private_socket_and_later_ones_forward() {
        let dir = TempDir::new("ipc-claim");
        let (server, owned) = bound(&dir);
        let sock = dir.join(SOCKET);
        assert_eq!(mode(&sock), 0o600, "only the owner may connect");
        assert_eq!(
            (owned.dev, owned.ino),
            (fs::metadata(&sock).unwrap().dev(), ino(&sock))
        );
        let before = ino(&sock);
        let rx = serve_n(server, 2);
        for paths in [&["/a.md"][..], &["/b.md", "/c.md"]] {
            assert!(matches!(
                claim(&dir, &message(paths)).unwrap(),
                Claim::Forwarded
            ));
            let want: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
            assert_eq!(rx.recv_timeout(WAIT).unwrap(), want);
        }
        assert_eq!(
            ino(&sock),
            before,
            "a forwarding launch touched the live socket"
        );
    }

    #[test]
    fn live_socket_is_not_unlinked_while_its_server_is_busy() {
        // The server isn't accepting (e.g. it's still starting GPUI): the
        // connection waits in the backlog, and the socket must stay.
        let dir = TempDir::new("ipc-busy");
        let (server, _owned) = bound(&dir);
        let before = ino(&dir.join(SOCKET));
        for _ in 0..3 {
            assert!(matches!(
                claim(&dir, &message(&["/x.md"])).unwrap(),
                Claim::Forwarded
            ));
        }
        assert_eq!(ino(&dir.join(SOCKET)), before);
        let rx = serve_n(server, 3);
        for _ in 0..3 {
            assert_eq!(rx.recv_timeout(WAIT).unwrap(), [PathBuf::from("/x.md")]);
        }
    }

    /// Issue #56: simultaneous launches all failed to connect, then each
    /// unlinked the others' fresh socket and bound its own.
    #[test]
    fn simultaneous_launches_elect_exactly_one_server() {
        const N: usize = 12;
        let dir = Arc::new(TempDir::new("ipc-race"));
        // A stale socket, so every launch also takes the "unlink" branch.
        drop(UnixListener::bind(dir.join(SOCKET)).unwrap());
        let barrier = Arc::new(Barrier::new(N));
        let launches: Vec<_> = (0..N)
            .map(|i| {
                let (dir, barrier) = (dir.clone(), barrier.clone());
                thread::spawn(move || {
                    let msg = message(&[&format!("/{i}.md")]);
                    barrier.wait();
                    claim(&dir, &msg).unwrap()
                })
            })
            .collect();
        let mut servers = Vec::new();
        for (i, launch) in launches.into_iter().enumerate() {
            if let Claim::Bound(server, owned) = launch.join().unwrap() {
                servers.push((i, server, owned));
            }
        }
        assert_eq!(servers.len(), 1, "more than one instance bound the socket");
        let (winner, server, owned) = servers.pop().unwrap();
        assert_eq!(
            ino(&dir.join(SOCKET)),
            owned.ino,
            "the winner's socket was replaced"
        );
        let rx = serve_n(server, N - 1);
        let mut got: Vec<PathBuf> = (1..N)
            .flat_map(|_| rx.recv_timeout(WAIT).unwrap())
            .collect();
        got.sort();
        let mut want: Vec<PathBuf> = (0..N)
            .filter(|&i| i != winner)
            .map(|i| PathBuf::from(format!("/{i}.md")))
            .collect();
        want.sort();
        assert_eq!(
            got, want,
            "every other launch's paths must reach the winner"
        );
    }

    #[test]
    fn stale_socket_is_replaced_only_under_the_lock() {
        let dir = TempDir::new("ipc-lock");
        let sock = dir.join(SOCKET);
        drop(UnixListener::bind(&sock).unwrap());
        // A second link keeps the stale inode allocated, so the new socket
        // can't reuse its number.
        fs::hard_link(&sock, dir.join("stale.sock")).unwrap();
        let stale = ino(&sock);
        let held = lock(&dir).unwrap();
        assert_eq!(mode(&dir.join(LOCK)), 0o600);
        let dir2 = dir.to_path_buf();
        let launch = thread::spawn(move || claim(&dir2, &message(&[])).unwrap());
        thread::sleep(Duration::from_millis(300));
        assert!(!launch.is_finished(), "took over without the lock");
        assert_eq!(ino(&sock), stale, "touched the socket without the lock");
        drop(held);
        let Claim::Bound(_server, owned) = launch.join().unwrap() else {
            panic!("a stale socket must be replaced, not forwarded to");
        };
        assert_ne!(ino(&sock), stale);
        assert_eq!(ino(&sock), owned.ino);
        assert!(
            dir.join(LOCK).exists(),
            "the lock file must never be deleted"
        );
    }

    #[test]
    fn claim_reports_errors_instead_of_unlinking() {
        // A directory at the socket path can't be connected to or unlinked
        // with remove_file: report it and leave it alone.
        let dir = TempDir::new("ipc-err");
        fs::create_dir(dir.join(SOCKET)).unwrap();
        fs::write(dir.join(SOCKET).join("keep"), "").unwrap();
        assert!(claim(&dir, &message(&[])).is_err());
        assert!(dir.join(SOCKET).join("keep").exists());
        // So can a missing directory.
        assert!(claim(&dir.join("missing"), &message(&[])).is_err());
    }

    #[test]
    fn a_socket_we_may_not_connect_to_is_not_unlinked() {
        // Only "connection refused" proves a socket stale. Permission denied
        // (root ignores the mode, so this can't be tested as root) must be
        // reported, leaving the socket alone.
        if getuid() == 0 {
            return;
        }
        let dir = TempDir::new("ipc-eacces");
        let sock = dir.join(SOCKET);
        let _listener = UnixListener::bind(&sock).unwrap();
        fs::set_permissions(&sock, Permissions::from_mode(0o000)).unwrap();
        let before = ino(&sock);
        assert!(claim(&dir, &message(&[])).is_err());
        assert_eq!(ino(&sock), before, "replaced a socket it couldn't test");
    }

    // ------------------------------------------------------------- cleanup

    #[test]
    fn release_removes_only_the_socket_this_process_bound() {
        let dir = TempDir::new("ipc-release");
        let sock = dir.join(SOCKET);
        let (_server, owned) = bound(&dir);
        owned.release();
        assert!(!sock.exists(), "cleanup must remove its own socket");

        // This instance stopped listening and a newer one took over: the
        // older one's cleanup must not unlink the newer one's live socket.
        let (old_server, old) = bound(&dir);
        drop(old_server);
        fs::remove_file(&sock).unwrap();
        let newer = UnixListener::bind(&sock).unwrap();
        let newer_ino = ino(&sock);
        old.release();
        assert_eq!(ino(&sock), newer_ino, "unlinked another instance's socket");
        assert!(UnixStream::connect(&sock).is_ok());
        drop(newer);

        // Releasing when the socket is already gone is harmless.
        let (_server, owned) = bound(&dir);
        fs::remove_file(&sock).unwrap();
        owned.release();
        assert!(!sock.exists());
    }

    #[test]
    fn cleanup_removes_owned_endpoint() {
        let dir = TempDir::new("ipc-cleanup");
        child(
            "cleanup_child",
            &[("XDG_RUNTIME_DIR", dir.as_os_str())],
            &[],
        );
        assert!(!dir.join(SOCKET).exists(), "cleanup must remove the socket");
    }

    #[test]
    fn cleanup_child() {
        if in_child("cleanup_child") {
            assert!(matches!(start(false, &[]), Instance::Primary(Some(_))));
            assert!(socket_dir().unwrap().join(SOCKET).exists());
            cleanup();
            cleanup(); // a second call is a no-op
        }
    }

    #[test]
    fn cleanup_leaves_sockets_it_did_not_bind() {
        let dir = TempDir::new("ipc-foreign");
        let _listener = UnixListener::bind(dir.join(SOCKET)).unwrap();
        child(
            "cleanup_foreign_child",
            &[("XDG_RUNTIME_DIR", dir.as_os_str())],
            &[],
        );
        assert!(
            dir.join(SOCKET).exists(),
            "cleanup removed a socket it didn't bind"
        );
    }

    #[test]
    fn cleanup_foreign_child() {
        if in_child("cleanup_foreign_child") {
            cleanup();
        }
    }

    #[test]
    fn stale_socket_replaced_without_initializing_gui() {
        let dir = TempDir::new("ipc-stale");
        let path = dir.join(SOCKET);
        drop(UnixListener::bind(&path).unwrap());
        fs::hard_link(&path, dir.join("stale.sock")).unwrap();
        let inode = ino(&path);
        child(
            "stale_socket_child",
            &[("XDG_RUNTIME_DIR", dir.as_os_str())],
            &[],
        );
        assert_ne!(ino(&path), inode);
    }

    #[test]
    fn stale_socket_child() {
        if in_child("stale_socket_child") {
            assert!(matches!(start(false, &[]), Instance::Primary(Some(_))));
        }
    }

    // ------------------------------------------------- private socket dir

    #[test]
    fn private_dir_is_created_with_mode_0700() {
        let tmp = TempDir::new("ipc-private");
        let dir = tmp.join("mdview-x");
        private_dir(&dir, getuid()).unwrap();
        assert!(dir.is_dir());
        assert_eq!(mode(&dir), 0o700);
        private_dir(&dir, getuid()).unwrap(); // an existing private dir is fine
        let missing = private_dir(&tmp.join("no/parent"), getuid()).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn private_dir_rejects_traps() {
        let tmp = TempDir::new("ipc-traps");
        let uid = getuid();
        // Any group or other bit is refused, and nothing is changed.
        for bits in [
            0o701, 0o702, 0o704, 0o710, 0o720, 0o740, 0o755, 0o777, 0o1777,
        ] {
            let dir = tmp.join(format!("open-{bits:o}"));
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, Permissions::from_mode(bits)).unwrap();
            let err = private_dir(&dir, uid).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{bits:o}");
            assert!(err.to_string().contains("mode"), "{err}");
            assert_eq!(mode(&dir), bits);
        }
        // Someone else's directory (simulated: we can't chown without root).
        let other = tmp.join("other");
        fs::create_dir(&other).unwrap();
        fs::set_permissions(&other, Permissions::from_mode(0o700)).unwrap();
        let err = private_dir(&other, uid.wrapping_add(1)).unwrap_err();
        assert!(err.to_string().contains("owned by uid"), "{err}");
        // A symlink, even to a private directory we own.
        let link = tmp.join("link");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        let err = private_dir(&link, uid).unwrap_err();
        assert!(err.to_string().contains("symlink"), "{err}");
        // A dangling symlink: not followed to create a directory elsewhere.
        let dangling = tmp.join("dangling");
        std::os::unix::fs::symlink(tmp.join("target"), &dangling).unwrap();
        assert!(private_dir(&dangling, uid).is_err());
        assert!(!tmp.join("target").exists());
        // A plain file.
        let file = tmp.join("file");
        fs::write(&file, "").unwrap();
        assert!(private_dir(&file, uid).is_err());
    }

    #[test]
    fn fallback_socket_lives_in_a_private_dir() {
        // Short tags here: macOS's $TMPDIR is long and socket paths are
        // limited to 104 bytes.
        let tmp = TempDir::new("fb");
        child(
            "fallback_child",
            &[("TMPDIR", tmp.as_os_str())],
            &["XDG_RUNTIME_DIR"],
        );
        let dir = tmp.join(format!("mdview-{}", getuid()));
        assert_eq!(mode(&dir), 0o700);
        assert!(!dir.join(SOCKET).exists(), "cleanup must remove the socket");
        assert_eq!(mode(&dir.join(LOCK)), 0o600);
    }

    #[test]
    fn fallback_child() {
        if in_child("fallback_child") {
            let Instance::Primary(Some(_server)) = start(false, &[]) else {
                panic!("expected to bind the fallback socket");
            };
            let sock = std::env::temp_dir().join(format!("mdview-{}/{SOCKET}", getuid()));
            assert_eq!(mode(&sock), 0o600);
            cleanup();
        }
    }

    #[test]
    fn unsafe_fallback_dir_is_not_used() {
        // Another user could have planted this directory and a socket in it
        // to read the paths we send. Don't connect, don't bind: run alone.
        let tmp = TempDir::new("us");
        let dir = tmp.join(format!("mdview-{}", getuid()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, Permissions::from_mode(0o777)).unwrap();
        let planted = UnixListener::bind(dir.join(SOCKET)).unwrap();
        planted.set_nonblocking(true).unwrap();
        child(
            "unsafe_child",
            &[("TMPDIR", tmp.as_os_str())],
            &["XDG_RUNTIME_DIR"],
        );
        assert!(planted.accept().is_err(), "connected to a planted socket");
        assert!(!dir.join(LOCK).exists());

        // Same for a symlink to a directory the attacker controls.
        fs::remove_dir_all(&dir).unwrap();
        let target = tmp.join("attacker");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, &dir).unwrap();
        let planted = UnixListener::bind(target.join(SOCKET)).unwrap();
        planted.set_nonblocking(true).unwrap();
        child(
            "unsafe_child",
            &[("TMPDIR", tmp.as_os_str())],
            &["XDG_RUNTIME_DIR"],
        );
        assert!(planted.accept().is_err(), "followed a symlinked socket dir");
        assert!(!target.join(LOCK).exists());
    }

    #[test]
    fn unsafe_child() {
        if in_child("unsafe_child") {
            assert!(socket_dir().is_err());
            assert!(matches!(start(false, &[]), Instance::Primary(None)));
        }
    }

    // ------------------------------------------------------------- serving

    #[test]
    fn server_delivers_paths_and_raise_requests_until_receiver_closes() {
        let dir = TempDir::new("ipc-server");
        let path = dir.join("server.sock");
        let server = server_with_timeout(&path, WAIT);
        let rx = serve_n(server, 2);
        // Malformed messages must neither reach the viewer nor stop it.
        for bad in [
            &[][..],
            &[0xff],
            b"/a.md\n/b.md",
            &[5, 0, 0, 0, 9, 0, 0, 0, b'x'],
            &[4, 0, 0, 0, 1, 0],
        ] {
            let mut client = UnixStream::connect(&path).unwrap();
            client.write_all(bad).unwrap();
        }
        let paths = vec![
            PathBuf::from("/a.md"),
            PathBuf::from("/b c.md"),
            PathBuf::from(OsStr::from_bytes(b"/new\nline\r")),
            PathBuf::from(OsStr::from_bytes(b"/caf\xe9.md")),
        ];
        let mut client = UnixStream::connect(&path).unwrap();
        client
            .write_all(&cli::encode_paths(&paths).unwrap())
            .unwrap();
        drop(client);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), paths);
        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(&cli::encode_paths(&[]).unwrap()).unwrap();
        drop(client);
        assert!(rx.recv_timeout(WAIT).unwrap().is_empty());
        // Returning false stopped the listener instead of accepting forever.
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&path).is_ok() {
            assert!(Instant::now() < deadline, "the listener kept running");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Issue #56: one client that connects and never writes (or never
    /// finishes) used to block the listener, and every later handoff, forever.
    #[test]
    fn stalled_clients_time_out_without_blocking_the_listener() {
        let dir = TempDir::new("ipc-stall");
        let path = dir.join("server.sock");
        let timeout = Duration::from_millis(300);
        let rx = serve_n(server_with_timeout(&path, timeout), 1);
        let start = Instant::now();
        // Connects and sends nothing.
        let _silent = UnixStream::connect(&path).unwrap();
        // Sends half a header.
        let mut half = UnixStream::connect(&path).unwrap();
        half.write_all(&[8, 0]).unwrap();
        // Announces a body and sends only part of it.
        let mut partial = UnixStream::connect(&path).unwrap();
        partial.write_all(&message(&["/never.md"])[..9]).unwrap();
        // Trickles one byte at a time, each well within the timeout: the
        // deadline covers the whole message, not each read.
        let path2 = path.clone();
        let trickle = thread::spawn(move || {
            let mut s = UnixStream::connect(&path2).unwrap();
            for b in message(&["/slow.md"]) {
                if s.write_all(&[b]).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        thread::sleep(Duration::from_millis(50)); // queue the good one last
        let mut good = UnixStream::connect(&path).unwrap();
        good.write_all(&message(&["/ok.md"])).unwrap();
        drop(good);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), [PathBuf::from("/ok.md")]);
        let took = start.elapsed();
        assert!(took >= timeout * 4, "stalls weren't waited for: {took:?}");
        assert!(took < timeout * 4 + Duration::from_secs(2), "took {took:?}");
        trickle.join().unwrap();
    }

    #[test]
    fn oversized_messages_are_rejected_before_reading() {
        let dir = TempDir::new("ipc-big");
        let path = dir.join("server.sock");
        // A long timeout: rejecting must not wait for the client.
        let rx = serve_n(server_with_timeout(&path, Duration::from_secs(60)), 2);
        let start = Instant::now();
        for header in [cli::MAX_MESSAGE as u32 + 1, u32::MAX] {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(&header.to_le_bytes()).unwrap();
            s.write_all(&[0; 64]).unwrap();
            std::mem::forget(s); // keep the connection open
        }
        let mut good = UnixStream::connect(&path).unwrap();
        good.write_all(&message(&["/ok.md"])).unwrap();
        drop(good);
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), [PathBuf::from("/ok.md")]);
        assert!(start.elapsed() < WAIT);

        // A message of exactly MAX_MESSAGE still goes through.
        let big = PathBuf::from(OsStr::from_bytes(&vec![b'a'; cli::MAX_MESSAGE - 4]));
        let msg = cli::encode_paths(std::slice::from_ref(&big)).unwrap();
        let path2 = path.clone();
        let writer = thread::spawn(move || UnixStream::connect(&path2).unwrap().write_all(&msg));
        assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap(), [big]);
        writer.join().unwrap().unwrap();
    }

    #[test]
    fn a_closed_client_is_read_without_waiting() {
        // On macOS, setting a read timeout after the client closed fails
        // (EINVAL); the buffered message must still be read.
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(&message(&["/done.md"])).unwrap();
        drop(client);
        let paths = read_message(&mut server, Instant::now() + WAIT).unwrap();
        assert_eq!(paths, [PathBuf::from("/done.md")]);
        // An expired deadline fails at once, even with data waiting.
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(&message(&["/late.md"])).unwrap();
        let err = read_message(&mut server, Instant::now()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
