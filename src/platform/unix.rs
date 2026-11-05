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
