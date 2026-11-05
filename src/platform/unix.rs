//! Unix single-instance transport. Keep the handoff ahead of GPUI startup.
use crate::cli;
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
};

pub enum Instance {
    Forwarded,
    Primary(Option<Server>),
}

pub struct Server(UnixListener);

fn socket_path() -> PathBuf {
    cli::socket_path_in(std::env::var_os("XDG_RUNTIME_DIR"), || {
        (
            std::env::temp_dir(),
            std::env::var("USER").unwrap_or_default(),
        )
    })
}

pub fn start(standalone: bool, paths: &[PathBuf]) -> Instance {
    if standalone {
        return Instance::Primary(None);
    }
    let sock = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&sock)
        && stream
            .write_all(cli::encode_paths(paths).as_bytes())
            .is_ok()
    {
        return Instance::Forwarded;
    }
    let _ = std::fs::remove_file(&sock); // stale socket from a crashed instance
    Instance::Primary(UnixListener::bind(&sock).ok().map(Server))
}

impl Server {
    pub fn serve(self, mut send: impl FnMut(Vec<PathBuf>) -> bool) {
        for mut stream in self.0.incoming().flatten() {
            let mut buf = String::new();
            if stream.read_to_string(&mut buf).is_ok() && !send(cli::decode_paths(&buf)) {
                break;
            }
        }
    }
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn cleanup_removes_owned_endpoint() {
        let dir = TempDir::new("ipc-cleanup");
        let socket = dir.join("mdview.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform::ipc::tests::cleanup_child"])
            .env("MDVIEW_TEST_CLEANUP", "1")
            .env("XDG_RUNTIME_DIR", dir.to_path_buf())
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!socket.exists(), "cleanup must remove the socket");
    }

    #[test]
    fn stale_socket_replaced_without_initializing_gui() {
        use std::os::unix::fs::MetadataExt;
        let dir = TempDir::new("ipc-stale");
        let path = dir.join("mdview.sock");
        drop(UnixListener::bind(&path).unwrap());
        std::fs::hard_link(&path, dir.join("stale.sock")).unwrap();
        let inode = std::fs::metadata(&path).unwrap().ino();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "platform::ipc::tests::stale_socket_child"])
            .env("MDVIEW_TEST_STALE", "1")
            .env("XDG_RUNTIME_DIR", dir.to_path_buf())
            .status()
            .unwrap();
        assert!(status.success());
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), inode);
    }

    #[test]
    fn stale_socket_child() {
        if std::env::var_os("MDVIEW_TEST_STALE").is_some() {
            assert!(matches!(start(false, &[]), Instance::Primary(Some(_))));
        }
    }

    #[test]
    fn cleanup_child() {
        // Isolate environment changes in a child instead of racing parallel tests.
        if std::env::var_os("MDVIEW_TEST_CLEANUP").is_some() {
            cleanup();
        }
    }

    #[test]
    fn server_delivers_paths_and_raise_requests_until_receiver_closes() {
        let dir = TempDir::new("ipc-server");
        let path = dir.join("server.sock");
        let server = Server(UnixListener::bind(&path).unwrap());
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut count = 0;
            server.serve(|paths| {
                tx.send(paths).unwrap();
                count += 1;
                count < 2
            });
        });
        // An invalid UTF-8 message must neither reach the viewer nor stop it.
        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(&[0xff]).unwrap();
        drop(client);
        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(b"/a.md\n/b c.md").unwrap();
        drop(client);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            vec![PathBuf::from("/a.md"), PathBuf::from("/b c.md")]
        );
        drop(UnixStream::connect(&path).unwrap());
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_empty());
        // Returning false must stop the listener instead of accepting forever.
        thread.join().unwrap();
    }
}
