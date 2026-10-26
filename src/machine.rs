// SPDX-License-Identifier: Apache-2.0
//! A GodspeedOS release image booted in QEMU, and its shell driven over the serial port.
//!
//! Only what an operator does: the shell's own commands typed at `gsh>`, its questions answered, and a file
//! offered by a web server on the host for `download` to fetch. Nothing is baked into the image or its disk, and no
//! OS code is linked or copied here (SPECIFICATION.md 10, 11).
//!
//! The machine is x86-64 on QEMU's `pc` board: UEFI firmware from QEMU's own `edk2-x86_64-code.fd`, the image on
//! the legacy IDE controller where the firmware boots it and GodspeedOS does not look for data, a blank data disk
//! alone on an AHCI controller, and an e1000 on QEMU's user network, where the host is 10.0.2.2.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How the machine is made.
#[derive(Clone, Debug)]
pub struct Config {
    /// `qemu-system-x86_64`.
    pub qemu: PathBuf,
    /// The UEFI firmware.
    pub firmware: PathBuf,
    /// The release image. A copy is booted, so the file given is never written.
    pub image: PathBuf,
    /// Where the copy, the data disk and the serial log go.
    pub work: PathBuf,
    /// Extra `-device` values.
    pub devices: Vec<String>,
    /// Cores.
    pub smp: u32,
}

/// The firmware QEMU ships, next to its binary (`share/edk2-x86_64-code.fd`), when it can be found.
pub fn default_firmware(qemu: &Path) -> Option<PathBuf> {
    let found = which(qemu)?;
    let dir = found.parent()?;
    [
        dir.join("share/edk2-x86_64-code.fd"),
        dir.join("../share/qemu/edk2-x86_64-code.fd"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

fn which(p: &Path) -> Option<PathBuf> {
    if p.components().count() > 1 {
        return p.exists().then(|| p.to_path_buf());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in [p.to_path_buf(), p.with_extension("exe")] {
            let c = dir.join(&name);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// `qemu --version`'s first line, for the report.
pub fn qemu_version(qemu: &Path) -> String {
    Command::new(qemu)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::to_string))
        .unwrap_or_else(|| "QEMU (version unknown)".to_string())
}

/// One file served over plain HTTP on the host, for the machine's `download`. Serves exactly one path, for as
/// long as it lives, and answers anything else with 404.
pub struct FileServer {
    /// The port, on every host address; the machine reaches it at 10.0.2.2.
    pub port: u16,
    /// The name it is served under.
    pub name: String,
}

impl FileServer {
    /// Serve `bytes` as `/<name>`.
    pub fn start(name: &str, bytes: Vec<u8>) -> std::io::Result<FileServer> {
        let listener = TcpListener::bind(("0.0.0.0", 0))?;
        let port = listener.local_addr()?.port();
        let path = format!("/{name}");
        let bytes = Arc::new(bytes);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let bytes = Arc::clone(&bytes);
                let path = path.clone();
                std::thread::spawn(move || serve_one(stream, &path, &bytes));
            }
        });
        Ok(FileServer {
            port,
            name: name.to_string(),
        })
    }

    /// The link the machine downloads from.
    pub fn link(&self) -> String {
        format!("http://10.0.2.2:{}/{}", self.port, self.name)
    }
}

fn serve_one(mut s: TcpStream, path: &str, bytes: &[u8]) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
    let mut req = Vec::new();
    let mut tmp = [0u8; 1024];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 16 * 1024 {
        match s.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => req.extend_from_slice(&tmp[..n]),
        }
    }
    let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method == "GET" && target == path {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(bytes);
    } else {
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
}

/// A running machine.
pub struct Machine {
    child: Child,
    serial: TcpStream,
    /// Everything the serial port has said, as it arrives.
    log: Arc<Mutex<Vec<u8>>>,
    /// How much of the log the shell session has consumed.
    cursor: usize,
    /// The serial log's file.
    pub log_path: PathBuf,
    /// The image copy that was booted.
    pub image_copy: PathBuf,
}

impl Machine {
    /// Boot it. The serial port is a TCP socket QEMU serves and this connects to before the firmware starts, so no
    /// output is lost.
    pub fn boot(cfg: &Config) -> Result<Machine, String> {
        std::fs::create_dir_all(&cfg.work).map_err(|e| format!("{}: {e}", cfg.work.display()))?;
        let image_copy = cfg.work.join("os.img");
        std::fs::copy(&cfg.image, &image_copy).map_err(|e| format!("copying {}: {e}", cfg.image.display()))?;
        let data = cfg.work.join("data.img");
        std::fs::write(&data, vec![0u8; 16 * 1024 * 1024]).map_err(|e| format!("{}: {e}", data.display()))?;
        let port = {
            let l = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("no free port: {e}"))?;
            l.local_addr().map_err(|e| e.to_string())?.port()
        };
        let fw = cfg.firmware.to_string_lossy().replace('\\', "/");
        let img = image_copy.to_string_lossy().replace('\\', "/");
        let dat = data.to_string_lossy().replace('\\', "/");
        let mut cmd = Command::new(&cfg.qemu);
        cmd.args([
            "-drive",
            &format!("if=pflash,format=raw,readonly=on,file={fw}"),
            "-drive",
            &format!("format=raw,file={img},if=ide"),
            "-device",
            "ich9-ahci,id=ahci",
            "-drive",
            &format!("id=data,format=raw,file={dat},if=none"),
            "-device",
            "ide-hd,drive=data,bus=ahci.0",
            "-cpu",
            "qemu64,+rdrand",
            "-smp",
            &cfg.smp.to_string(),
            "-m",
            "512M",
            "-serial",
            &format!("tcp:127.0.0.1:{port},server=on,wait=on"),
            "-display",
            "none",
            "-no-reboot",
            "-device",
            "e1000,netdev=n0",
            "-netdev",
            "user,id=n0",
        ]);
        for d in &cfg.devices {
            cmd.args(["-device", d]);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null());
        let err = std::fs::File::create(cfg.work.join("qemu.stderr")).map_err(|e| e.to_string())?;
        cmd.stderr(err);
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", cfg.qemu.display()))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let serial = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => {
                    if let Ok(Some(st)) = child.try_wait() {
                        let why = std::fs::read_to_string(cfg.work.join("qemu.stderr")).unwrap_or_default();
                        return Err(format!(
                            "QEMU ended ({st}) before its serial port opened: {}",
                            why.trim()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(format!("QEMU's serial port never opened: {e}"));
                }
            }
        };
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_path = cfg.work.join("serial.log");
        let mut file = std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
        let mut reader = serial.try_clone().map_err(|e| e.to_string())?;
        let sink = Arc::clone(&log);
        std::thread::spawn(move || {
            let mut tmp = [0u8; 8192];
            loop {
                match reader.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = file.write_all(&tmp[..n]);
                        sink.lock().unwrap().extend_from_slice(&tmp[..n]);
                    }
                }
            }
        });
        Ok(Machine {
            child,
            serial,
            log,
            cursor: 0,
            log_path,
            image_copy,
        })
    }

    /// Everything said so far, as text: escape sequences and carriage returns removed.
    pub fn text(&self) -> String {
        clean(&self.log.lock().unwrap())
    }

    /// The cleaned text of everything said from raw byte `from` onward. `from` is a value `len()` returned
    /// earlier; cleaning the raw slice (as `seen_since` does) is correct, whereas slicing `text()` by a raw
    /// offset is not - cleaning removes bytes, so the offsets do not line up.
    pub fn text_from(&self, from: usize) -> String {
        let log = self.log.lock().unwrap();
        clean(&log[from.min(log.len())..])
    }

    /// Advance the session cursor past everything said so far, so the next `command`/`wait_for` matches only
    /// what comes after. Use it to discard stale prompts left in the buffer (a kill storm prints many).
    pub fn drain(&mut self) {
        self.cursor = self.log.lock().unwrap().len();
    }

    /// Wait until `needle` appears in what the serial port says after the session's cursor, and move the cursor past
    /// it. Returns what was said up to and including it, cleaned, or `None` at the deadline (the cursor then stays).
    pub fn wait_for(&mut self, needle: &str, secs: u64) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            {
                let log = self.log.lock().unwrap();
                let fresh = &log[self.cursor..];
                if let Some(at) = fresh.windows(needle.len()).position(|w| w == needle.as_bytes()) {
                    let end = self.cursor + at + needle.len();
                    let said = clean(&log[self.cursor..end]);
                    self.cursor = end;
                    return Some(said);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether `needle` has appeared anywhere in the log since byte `from`, waiting up to `secs` for it. Does not
    /// move the session's cursor: a driver's line can arrive in the middle of a command's output.
    pub fn seen_since(&self, from: usize, needle: &str, secs: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let found = {
                let log = self.log.lock().unwrap();
                clean(&log[from.min(log.len())..]).contains(needle)
            };
            if found {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// How many bytes the log holds.
    pub fn len(&self) -> usize {
        self.log.lock().unwrap().len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Type `line` at the shell and press Enter.
    pub fn type_line(&mut self, line: &str) -> Result<(), String> {
        self.serial
            .write_all(format!("{line}\r").as_bytes())
            .map_err(|e| format!("the serial port closed: {e}"))
    }

    /// Type a command and wait for the next prompt; what was said in between, or `None` at the deadline.
    pub fn command(&mut self, line: &str, secs: u64) -> Option<String> {
        self.type_line(line).ok()?;
        self.wait_for("gsh>", secs)
    }

    /// Stop the machine.
    pub fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Bytes from the serial port as text: ANSI escape sequences and carriage returns removed, so a line can be
/// matched as it would be read.
pub fn clean(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    while let Some(&n) = chars.peek() {
                        chars.next();
                        if n.is_ascii_alphabetic() || n == '~' {
                            break;
                        }
                    }
                }
            }
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}
