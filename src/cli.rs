//! Command-line arguments and the single-instance socket message.
//!
//! Both run before the socket check in `main()`, so they must stay trivial:
//! no I/O, no allocations beyond the path list itself.

use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "\
mdview — fast Markdown viewer

USAGE: mdview [-n] [FILE...]

  -n, --new   Start a separate instance instead of reusing the running one
  -h, --help  Show this help

KEYS: Ctrl+O open · Ctrl+W close tab · Ctrl+Tab / Ctrl+Shift+Tab switch tabs
      Ctrl+R reload · Ctrl+Q quit · ↑↓ PgUp PgDn Space Home End scroll
      macOS also: Cmd+O open · Cmd+W close · Cmd+R reload · Cmd+Q quit
";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `-h` / `--help` anywhere: print [`HELP`] and exit.
    Help,
    /// Open `paths` (as given, not normalized yet).
    Run {
        /// `-n` / `--new`: don't use or create the single-instance socket.
        standalone: bool,
        paths: Vec<PathBuf>,
    },
}

/// Parses the arguments after the program name. Every argument that isn't
/// one of the flags is a path, including unknown flags, `--` and non-UTF-8
/// arguments.
pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Command {
    let mut standalone = false;
    let mut paths = Vec::new();
    for arg in args {
        match arg.to_str() {
            Some("-n" | "--new") => standalone = true,
            Some("-h" | "--help") => return Command::Help,
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    Command::Run { standalone, paths }
}

/// `$XDG_RUNTIME_DIR`, the preferred home of the single-instance socket and
/// its lock file, if it's set to an absolute path. An unset, empty or
/// relative value is ignored (the XDG spec says to), so the socket is never
/// relative to the working directory. `platform::ipc` still checks that the
/// directory is private before using it.
#[cfg(unix)]
pub fn runtime_dir(var: Option<OsString>) -> Option<PathBuf> {
    var.map(PathBuf::from).filter(|dir| dir.is_absolute())
}

/// `$TMPDIR/mdview-$UID`, used when [`runtime_dir`] is missing or unsafe. The
/// temp dir is shared, so `platform::ipc` creates this directory itself
/// (mode 0700) and checks it, and the temp dir, before every use.
#[cfg(unix)]
pub fn fallback_dir(tmp: &std::path::Path, uid: u32) -> PathBuf {
    tmp.join(format!("mdview-{uid}"))
}

/// Largest message body the running instance accepts (16 MiB, the same cap as
/// the Windows pipe). A larger one is rejected before it's read.
#[cfg(unix)]
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// Most paths one message may carry. Without it a 16 MiB body of 5-byte
/// frames would decode into millions of allocations; a launch with more
/// paths opens its own window instead.
#[cfg(unix)]
pub const MAX_PATHS: usize = 65_536;

/// Whether `path` can be sent: Unix paths are never empty (an empty one
/// would be opened as the working directory) and never contain NUL.
#[cfg(unix)]
fn valid_path(bytes: &[u8]) -> bool {
    !bytes.is_empty() && !bytes.contains(&0)
}

/// The message a client sends to the running instance: a little-endian `u32`
/// body length, then for each path a little-endian `u32` byte count and the
/// path's raw bytes. An empty list (a 4-byte zero header) just raises the
/// window.
///
/// The bytes are the OS bytes (`OsStrExt::as_bytes`), not a display string,
/// so every Unix path survives: newlines, trailing `\r` and non-UTF-8 bytes
/// included. Returns `None` for anything [`decode_paths`] would reject: a
/// body over [`MAX_MESSAGE`], more than [`MAX_PATHS`] paths, or a path that
/// is empty or contains NUL.
#[cfg(unix)]
pub fn encode_paths(paths: &[PathBuf]) -> Option<Vec<u8>> {
    use std::os::unix::ffi::OsStrExt;
    if paths.len() > MAX_PATHS || !paths.iter().all(|p| valid_path(p.as_os_str().as_bytes())) {
        return None;
    }
    let body: usize = paths.iter().map(|p| 4 + p.as_os_str().len()).sum();
    if body > MAX_MESSAGE {
        return None;
    }
    let mut msg = Vec::with_capacity(4 + body);
    msg.extend_from_slice(&(body as u32).to_le_bytes());
    for path in paths {
        let bytes = path.as_os_str().as_bytes();
        msg.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        msg.extend_from_slice(bytes);
    }
    Some(msg)
}

/// The body length announced by a message header, or `None` if it's over
/// [`MAX_MESSAGE`].
#[cfg(unix)]
pub fn body_len(header: [u8; 4]) -> Option<usize> {
    Some(u32::from_le_bytes(header) as usize).filter(|&len| len <= MAX_MESSAGE)
}

/// Reads the body of a message written by [`encode_paths`] (without the
/// header). An empty body is the "raise the window" request. Returns `None`
/// if a frame is truncated, empty or contains NUL, or if there are more than
/// [`MAX_PATHS`] frames.
#[cfg(unix)]
pub fn decode_paths(mut body: &[u8]) -> Option<Vec<PathBuf>> {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    let mut paths = Vec::new();
    while let Some((len, rest)) = body.split_first_chunk::<4>() {
        let len = u32::from_le_bytes(*len) as usize;
        if len > rest.len() || paths.len() == MAX_PATHS {
            return None;
        }
        let (path, rest) = rest.split_at(len);
        if !valid_path(path) {
            return None;
        }
        paths.push(PathBuf::from(OsStr::from_bytes(path)));
        body = rest;
    }
    body.is_empty().then_some(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn run(standalone: bool, paths: &[&str]) -> Command {
        Command::Run {
            standalone,
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn no_args() {
        assert_eq!(parse_args(args(&[])), run(false, &[]));
    }

    #[test]
    fn paths_in_order() {
        assert_eq!(
            parse_args(args(&["a.md", "dir/b.md", "/abs/c.md", "a.md"])),
            run(false, &["a.md", "dir/b.md", "/abs/c.md", "a.md"])
        );
    }

    #[test]
    fn new_flag_anywhere() {
        for list in [
            &["-n", "a.md"][..],
            &["a.md", "-n"],
            &["--new", "a.md"],
            &["-n", "--new", "a.md", "-n"],
        ] {
            assert_eq!(parse_args(args(list)), run(true, &["a.md"]), "{list:?}");
        }
        assert_eq!(parse_args(args(&["-n"])), run(true, &[]));
    }

    #[test]
    fn help_wins_anywhere() {
        for list in [
            &["-h"][..],
            &["--help"],
            &["a.md", "-h"],
            &["-n", "--help", "b.md"],
            &["-h", "\u{0}weird"],
        ] {
            assert_eq!(parse_args(args(list)), Command::Help, "{list:?}");
        }
    }

    #[test]
    fn everything_else_is_a_path() {
        // Pinned current behavior: there are no other flags and no `--`
        // terminator; unknown flags are opened as (missing) files.
        assert_eq!(
            parse_args(args(&[
                "-x", "--", "-", "-nh", "--New", "-N", "--help=1", "-foo.md"
            ])),
            run(
                false,
                &["-x", "--", "-", "-nh", "--New", "-N", "--help=1", "-foo.md"]
            )
        );
        assert_eq!(parse_args(args(&["--", "-n"])), run(true, &["--"]));
        assert_eq!(parse_args(args(&["--", "-h"])), Command::Help);
        assert_eq!(parse_args(args(&[""])), run(false, &[""]));
        assert_eq!(parse_args(args(&[" -n"])), run(false, &[" -n"]));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_kept_byte_for_byte() {
        let raw = OsStr::from_bytes(b"caf\xe9.md").to_owned();
        let flagish = OsStr::from_bytes(b"-n\xff").to_owned();
        let cmd = parse_args(vec![raw.clone(), flagish.clone()]);
        assert_eq!(
            cmd,
            Command::Run {
                standalone: false,
                paths: vec![PathBuf::from(raw), PathBuf::from(flagish)],
            }
        );
    }

    #[test]
    fn help_text() {
        assert!(HELP.starts_with("mdview"));
        for needle in ["-n, --new", "-h, --help", "[FILE...]", "Ctrl+O", "Ctrl+W"] {
            assert!(HELP.contains(needle), "{needle}");
        }
        assert!(HELP.ends_with('\n'));
    }

    #[cfg(unix)]
    #[test]
    fn socket_dirs() {
        assert_eq!(
            runtime_dir(Some("/run/user/1000".into())),
            Some("/run/user/1000".into())
        );
        // An empty or relative XDG_RUNTIME_DIR would put the socket (and its
        // lock file) in the working directory: use the fallback instead.
        for bad in [None, Some(""), Some("run/user/1000"), Some("./x")] {
            assert_eq!(runtime_dir(bad.map(OsString::from)), None, "{bad:?}");
        }
        assert_eq!(
            fallback_dir("/tmp".as_ref(), 1000),
            PathBuf::from("/tmp/mdview-1000")
        );
        assert_eq!(
            fallback_dir("/var/folders/x/T/".as_ref(), 0),
            PathBuf::from("/var/folders/x/T/mdview-0")
        );
    }

    /// Encodes `paths` and splits off the header, checking it on the way.
    #[cfg(unix)]
    fn body(paths: &[PathBuf]) -> Vec<u8> {
        let msg = encode_paths(paths).expect("message within the cap");
        let (header, body) = msg.split_first_chunk::<4>().unwrap();
        assert_eq!(body_len(*header), Some(body.len()));
        body.to_vec()
    }

    #[cfg(unix)]
    fn bytes(b: &[u8]) -> PathBuf {
        PathBuf::from(OsStr::from_bytes(b))
    }

    #[cfg(unix)]
    #[test]
    fn encode_format() {
        assert_eq!(encode_paths(&[]).unwrap(), [0, 0, 0, 0]);
        assert_eq!(
            encode_paths(&[bytes(b"/a"), bytes(b"b")]).unwrap(),
            [11, 0, 0, 0, 2, 0, 0, 0, b'/', b'a', 1, 0, 0, 0, b'b']
        );
        // Lengths are little-endian and count bytes, not characters.
        let long = bytes(&[b'x'; 0x0102]);
        let msg = encode_paths(std::slice::from_ref(&long)).unwrap();
        assert_eq!(msg[..8], [0x06, 0x01, 0, 0, 0x02, 0x01, 0, 0]);
        assert_eq!(body(&[PathBuf::from("é")])[..4], [2, 0, 0, 0]);
    }

    #[cfg(unix)]
    #[test]
    fn decode_format() {
        assert_eq!(decode_paths(&[]), Some(vec![]));
        assert_eq!(
            decode_paths(&[2, 0, 0, 0, b'/', b'a', 1, 0, 0, 0, b'b']),
            Some(vec![bytes(b"/a"), bytes(b"b")])
        );
        // A truncated length or path is rejected, not opened as a shorter path.
        for bad in [
            &[1][..],
            &[1, 0, 0],
            &[1, 0, 0, 0],
            &[3, 0, 0, 0, b'a', b'b'],
            &[1, 0, 0, 0, b'a', 0],
            &[0xff, 0xff, 0xff, 0xff, b'a'],
        ] {
            assert_eq!(decode_paths(bad), None, "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn empty_and_nul_paths_are_refused() {
        // An empty path would open the listener's working directory, and NUL
        // can't occur in a Unix path: neither is a path anyone sent.
        for bad in [
            &[0, 0, 0, 0][..],
            &[1, 0, 0, 0, b'a', 0, 0, 0, 0],
            &[0, 0, 0, 0, 1, 0, 0, 0, b'a'],
            &[1, 0, 0, 0, 0],
            &[3, 0, 0, 0, b'/', 0, b'a'],
            &[2, 0, 0, 0, b'/', b'a', 2, 0, 0, 0, b'b', 0],
        ] {
            assert_eq!(decode_paths(bad), None, "{bad:?}");
        }
        for bad in [bytes(b""), bytes(b"\0"), bytes(b"/a\0b.md"), bytes(b"/a\0")] {
            assert_eq!(encode_paths(&[bytes(b"/ok"), bad.clone()]), None, "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn path_count_is_capped() {
        assert_eq!(MAX_PATHS, 65_536);
        let one = || bytes(b"a");
        let fits = vec![one(); MAX_PATHS];
        let msg = encode_paths(&fits).expect("MAX_PATHS paths fit");
        assert_eq!(decode_paths(&msg[4..]).map(|p| p.len()), Some(MAX_PATHS));
        // One more frame is refused on both ends, even though the body is
        // far below MAX_MESSAGE.
        let over = vec![one(); MAX_PATHS + 1];
        assert_eq!(encode_paths(&over), None);
        let mut body = msg[4..].to_vec();
        body.extend_from_slice(&[1, 0, 0, 0, b'a']);
        assert!(body.len() < MAX_MESSAGE);
        assert_eq!(decode_paths(&body), None);
    }

    #[cfg(unix)]
    #[test]
    fn header_is_capped() {
        assert_eq!(MAX_MESSAGE, 16 << 20, "documented as 16 MiB, like Windows");
        assert_eq!(body_len([0; 4]), Some(0));
        assert_eq!(body_len([1, 2, 0, 0]), Some(0x0201));
        assert_eq!(
            body_len((MAX_MESSAGE as u32).to_le_bytes()),
            Some(MAX_MESSAGE)
        );
        assert_eq!(body_len((MAX_MESSAGE as u32 + 1).to_le_bytes()), None);
        assert_eq!(body_len([0xff; 4]), None);
    }

    #[cfg(unix)]
    #[test]
    fn encode_refuses_oversized_messages() {
        // The body is MAX_MESSAGE exactly with one path of MAX_MESSAGE - 4.
        let fits = bytes(&vec![b'a'; MAX_MESSAGE - 4]);
        assert_eq!(body(std::slice::from_ref(&fits)).len(), MAX_MESSAGE);
        let over = bytes(&vec![b'a'; MAX_MESSAGE - 3]);
        assert_eq!(encode_paths(&[over]), None);
        // Each frame header counts toward the cap too: two paths that fill
        // it exactly, then one byte over.
        let half = MAX_MESSAGE / 2 - 4;
        let two = [bytes(&vec![b'a'; half]), bytes(&vec![b'b'; half])];
        assert_eq!(body(&two).len(), MAX_MESSAGE);
        let over = [bytes(&vec![b'a'; half]), bytes(&vec![b'b'; half + 1])];
        assert_eq!(encode_paths(&over), None);
    }

    /// Issue #25: the old text format turned these into other paths.
    #[cfg(unix)]
    #[test]
    fn round_trips_every_unix_path() {
        for paths in [
            vec![],
            vec![bytes(b"/a.md")],
            vec![bytes(b"/a.md"), bytes(b"/b.md"), bytes(b"/a.md")],
            vec![bytes(b"/with space/x y.md"), bytes(b"/tab\there.md")],
            vec![
                PathBuf::from("/\u{fc}n\u{ef}c\u{f6}d\u{e9}/日本語.md"),
                PathBuf::from("/📝.md"),
            ],
            vec![bytes(b"relative.md"), bytes(b"/trailing/slash/")],
            vec![bytes(b"/a\nb.md"), bytes(b"\n"), bytes(b"/c\n")],
            vec![bytes(b"/a\r"), bytes(b"/b"), bytes(b"/c\r\n"), bytes(b"\r")],
            vec![
                bytes(b"/caf\xe9.md"),
                bytes(b"/\xff\xfe\x80"),
                bytes(b"/\xc3"),
            ],
            vec![bytes(&(1..=255).collect::<Vec<u8>>())],
        ] {
            let msg = encode_paths(&paths).unwrap();
            assert_eq!(decode_paths(&msg[4..]), Some(paths.clone()), "{paths:?}");
        }
    }
}
