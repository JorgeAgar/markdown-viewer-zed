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

/// Where the single-instance socket lives: `$XDG_RUNTIME_DIR/mdview.sock`,
/// else `$TMPDIR/mdview-$USER.sock`. The fallback is only computed when
/// needed.
pub fn socket_path_in(
    runtime_dir: Option<OsString>,
    fallback: impl FnOnce() -> (PathBuf, String),
) -> PathBuf {
    match runtime_dir {
        Some(dir) => PathBuf::from(dir).join("mdview.sock"),
        None => {
            let (tmp, user) = fallback();
            tmp.join(format!("mdview-{user}.sock"))
        }
    }
}

/// The message a client sends to the running instance: the paths, separated
/// by `\n`. An empty message just raises the window.
///
/// The format is text, so it can't carry every path: non-UTF-8 bytes become
/// U+FFFD, and a path containing `\n` is split in two.
pub fn encode_paths(paths: &[PathBuf]) -> String {
    let msg: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    msg.join("\n")
}

/// Reads a message written by [`encode_paths`]. Empty lines are skipped, and
/// so is a `\r` right before a `\n`.
pub fn decode_paths(msg: &str) -> Vec<PathBuf> {
    msg.lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[cfg(unix)]
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    fn encode(paths: &[&Path]) -> String {
        encode_paths(&paths.iter().map(|p| p.to_path_buf()).collect::<Vec<_>>())
    }

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

    #[test]
    fn socket_path() {
        let never = || -> (PathBuf, String) { panic!("fallback computed needlessly") };
        assert_eq!(
            socket_path_in(Some("/run/user/1000".into()), never),
            Path::new("/run/user/1000/mdview.sock")
        );
        assert_eq!(
            socket_path_in(None, || ("/tmp".into(), "jo".into())),
            Path::new("/tmp/mdview-jo.sock")
        );
        assert_eq!(
            socket_path_in(None, || ("/tmp".into(), String::new())),
            Path::new("/tmp/mdview-.sock")
        );
        // An empty XDG_RUNTIME_DIR is used as is (a relative socket path).
        assert_eq!(
            socket_path_in(Some("".into()), never),
            Path::new("mdview.sock")
        );
    }

    #[test]
    fn encode_format() {
        assert_eq!(encode(&[]), "");
        assert_eq!(encode(&[Path::new("/a.md")]), "/a.md");
        assert_eq!(
            encode(&[Path::new("/a.md"), Path::new("/b c/d.md")]),
            "/a.md\n/b c/d.md"
        );
    }

    #[test]
    fn decode_format() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(decode_paths(""), Vec::<PathBuf>::new());
        assert_eq!(decode_paths("\n\n"), Vec::<PathBuf>::new());
        assert_eq!(decode_paths("/a.md"), [p("/a.md")]);
        assert_eq!(decode_paths("/a.md\n"), [p("/a.md")]);
        assert_eq!(decode_paths("\n/a.md\n\n/b.md\n"), [p("/a.md"), p("/b.md")]);
        assert_eq!(decode_paths("/a.md\r\n/b.md\r\n"), [p("/a.md"), p("/b.md")]);
        assert_eq!(decode_paths("  /a.md "), [p("  /a.md ")]);
    }

    #[test]
    fn round_trips() {
        for paths in [
            &[][..],
            &["/a.md"],
            &["/a.md", "/b.md", "/a.md"],
            &["/with space/x y.md", "/tab\there.md"],
            &["/ünïcödé/日本語.md", "/emoji/📝.md"],
            &["relative.md", "/trailing/slash/"],
            &["/with\rcarriage.md"],
        ] {
            let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
            assert_eq!(decode_paths(&encode_paths(&paths)), paths, "{paths:?}");
        }
    }

    /// Known limits of the text protocol, pinned so a change is deliberate.
    #[test]
    fn unrepresentable_paths() {
        // A newline in a path splits it into two paths.
        assert_eq!(
            decode_paths(&encode(&[Path::new("/a\nb.md")])),
            [PathBuf::from("/a"), PathBuf::from("b.md")]
        );
        // A trailing '\r' is lost (`str::lines` strips "\r\n").
        assert_eq!(
            decode_paths(&encode(&[Path::new("/a\r"), Path::new("/b")])),
            [PathBuf::from("/a"), PathBuf::from("/b")]
        );
        // Non-UTF-8 bytes are replaced, so the server opens another path.
        #[cfg(unix)]
        {
            let raw = Path::new(OsStr::from_bytes(b"/caf\xe9.md"));
            let msg = encode(&[raw]);
            assert_eq!(msg, "/caf\u{FFFD}.md");
            assert_ne!(decode_paths(&msg), [raw.to_path_buf()]);
        }
    }
}
