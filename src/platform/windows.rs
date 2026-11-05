//! Per-user named pipe IPC. No GPU, font or window initialization on handoff.
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::PathBuf,
};
use windows::{
    Win32::{
        Foundation::{ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::WindowsAndMessaging::AllowSetForegroundWindow,
    },
    core::{HSTRING, PCWSTR, PWSTR},
};

pub enum Instance {
    Forwarded,
    Primary(Option<Server>),
}
pub struct Server(File);
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

/// The token SID both scopes the endpoint and restricts its ACL to this user.
fn identity() -> windows::core::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = OwnedHandle::from_raw_handle(token.0);
        let mut size = 0;
        let _ = GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, None, 0, &mut size);
        let mut buf = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            size,
            &mut size,
        )?;
        let user = &*buf.as_ptr().cast::<TOKEN_USER>();
        let mut sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut sid)?;
        let value = sid.to_string();
        LocalFree(Some(HLOCAL(sid.0.cast())));
        Ok(value?)
    }
}

fn create(name: &HSTRING, sid: &str) -> windows::core::Result<Server> {
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(format!("D:P(A;;GA;;;{sid})")),
            1,
            &mut descriptor,
            None,
        )?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let handle = CreateNamedPipeW(
            name,
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            65536,
            0,
            Some(&attrs),
        );
        let error = windows::core::Error::from_win32();
        LocalFree(Some(HLOCAL(descriptor.0)));
        if handle.is_invalid() {
            return Err(error);
        }
        Ok(Server(File::from_raw_handle(handle.0)))
    }
}

fn handoff(name: &HSTRING, message: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(PathBuf::from(OsString::from_wide(name.as_ref())))?;
    unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(HANDLE(file.as_raw_handle()), &mut pid).is_ok() {
            let _ = AllowSetForegroundWindow(pid);
        }
    }
    file.write_all(&(message.len() as u32).to_le_bytes())?;
    file.write_all(message)
}

pub fn start(standalone: bool, paths: &[PathBuf]) -> Instance {
    if standalone {
        return Instance::Primary(None);
    }
    let sid = match identity() {
        Ok(sid) => sid,
        Err(e) => {
            eprintln!("mdview: single-instance identity: {e}");
            return Instance::Primary(None);
        }
    };
    let name = HSTRING::from(format!(r"\\.\pipe\mdview-{sid}"));
    let message = encode(paths);
    if handoff(&name, &message).is_ok() {
        return Instance::Forwarded;
    }
    match create(&name, &sid) {
        Ok(server) => Instance::Primary(Some(server)),
        Err(e) => {
            // Another launch can win the creation race, or be serving a client.
            unsafe {
                let _ = WaitNamedPipeW(PCWSTR(name.as_ptr()), 2000);
            }
            if handoff(&name, &message).is_ok() {
                return Instance::Forwarded;
            }
            eprintln!("mdview: single-instance pipe unavailable: {e}");
            Instance::Primary(None)
        }
    }
}

impl Server {
    pub fn serve(mut self, mut send: impl FnMut(Vec<PathBuf>) -> bool) {
        loop {
            let handle = HANDLE(self.0.as_raw_handle());
            let connected = unsafe { ConnectNamedPipe(handle, None) };
            if connected.is_err_and(|e| e.code() != ERROR_PIPE_CONNECTED.to_hresult()) {
                break;
            }
            let message = read_message(&mut self.0);
            unsafe {
                let _ = DisconnectNamedPipe(handle);
            }
            if let Ok(paths) = message
                && !send(paths)
            {
                break;
            }
        }
    }
}

fn encode(paths: &[PathBuf]) -> Vec<u8> {
    let mut message = Vec::new();
    for path in paths {
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        message.extend_from_slice(&(units.len() as u32).to_le_bytes());
        for unit in units {
            message.extend_from_slice(&unit.to_le_bytes());
        }
    }
    message
}

fn read_message(reader: &mut impl Read) -> io::Result<Vec<PathBuf>> {
    let mut len = [0; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut message = vec![0; len];
    reader.read_exact(&mut message)?;
    let mut paths = Vec::new();
    let mut rest = message.as_slice();
    while !rest.is_empty() {
        if rest.len() < 4 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let units = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
        rest = &rest[4..];
        if units > rest.len() / 2 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let (path, tail) = rest.split_at(units * 2);
        let units: Vec<u16> = path
            .as_chunks::<2>()
            .0
            .iter()
            .map(|u| u16::from_le_bytes([u[0], u[1]]))
            .collect();
        paths.push(PathBuf::from(OsString::from_wide(&units)));
        rest = tail;
    }
    Ok(paths)
}

pub fn cleanup() {} // Closing the process handles releases the pipe.

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_preserves_windows_paths() {
        let paths = vec![
            PathBuf::from(r"C:\a b\日本語.md"),
            PathBuf::from(r"\\server\share\a.md"),
            PathBuf::from("line\nbreak.md"),
            PathBuf::from(OsString::from_wide(&[0xD800, 46, 109, 100])),
        ];
        let msg = encode(&paths);
        let mut wire = (msg.len() as u32).to_le_bytes().to_vec();
        wire.extend(msg);
        assert_eq!(read_message(&mut wire.as_slice()).unwrap(), paths);
        assert!(read_message(&mut [0u8; 4].as_slice()).unwrap().is_empty());
    }
    #[test]
    fn malformed_protocol_is_rejected() {
        for wire in [
            vec![],
            vec![1, 0, 0, 0, 0],
            vec![4, 0, 0, 0, 1, 0, 0, 0],
            ((MAX_MESSAGE + 1) as u32).to_le_bytes().to_vec(),
        ] {
            assert!(read_message(&mut wire.as_slice()).is_err());
        }
    }
    #[test]
    fn pipe_handoff_and_exclusive_ownership() {
        let sid = identity().unwrap();
        let name = HSTRING::from(format!(r"\\.\pipe\mdview-test-{}", std::process::id()));
        let server = create(&name, &sid).unwrap();
        assert!(create(&name, &sid).is_err());
        let paths = vec![PathBuf::from(r"C:\test.md")];
        handoff(&name, &encode(&paths)).unwrap();
        let mut got = None;
        server.serve(|p| {
            got = Some(p);
            false
        });
        assert_eq!(got.unwrap(), paths);
    }
}
