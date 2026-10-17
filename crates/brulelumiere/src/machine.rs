//! GRBL machine connection: serial port, job streaming, jog and framing —
//! the role of LightBurn's GRBL device (Send, Frame, jog controls).
//!
//! A worker thread owns the serial port. The UI sends it [`Cmd`]s over a
//! channel and reads progress/log back from a shared [`Status`]. Streaming
//! uses GRBL's simple send-line / wait-for-`ok` handshake.
//!
//! NOTE: this talks to real hardware; it is exercised through the UI with a
//! laser controller attached and cannot be covered by the unit tests here.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// List available serial port names.
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

pub enum Cmd {
    Line(String),
    Job(Vec<String>),
    Jog { dx: f64, dy: f64, feed: f64 },
    FeedHold,
    Resume,
    Disconnect,
}

#[derive(Default)]
pub struct Status {
    pub connected: bool,
    pub sending: bool,
    pub sent: usize,
    pub total: usize,
    pub log: Vec<String>,
}

impl Status {
    fn push(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        if self.log.len() > 300 {
            let excess = self.log.len() - 300;
            self.log.drain(0..excess);
        }
    }
}

pub struct Machine {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    handle: Option<JoinHandle<()>>,
}

impl Machine {
    /// Open `port` at `baud` and start the worker thread.
    pub fn connect(port: &str, baud: u32) -> Result<Machine, String> {
        let sp = serialport::new(port, baud)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Ouverture {port} impossible : {e}"))?;

        let status = Arc::new(Mutex::new(Status { connected: true, ..Default::default() }));
        let (tx, rx) = mpsc::channel::<Cmd>();
        let st = status.clone();
        let port_name = port.to_string();
        let handle = std::thread::spawn(move || worker(sp, rx, st, port_name));
        Ok(Machine { tx, status, handle: Some(handle) })
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    /// Snapshot of the current status for the UI.
    pub fn status(&self) -> (bool, bool, usize, usize, Vec<String>) {
        let s = self.status.lock().unwrap();
        (s.connected, s.sending, s.sent, s.total, s.log.clone())
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Disconnect);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn worker(port: Box<dyn serialport::SerialPort>, rx: Receiver<Cmd>, status: Arc<Mutex<Status>>, name: String) {
    let writer = port;
    let reader = match writer.try_clone() {
        Ok(r) => r,
        Err(e) => {
            status.lock().unwrap().push(format!("Clonage du port échoué : {e}"));
            return;
        }
    };
    let mut writer = writer;
    let mut reader = BufReader::new(reader);
    status.lock().unwrap().push(format!("Connecté à {name}"));

    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Disconnect => {
                status.lock().unwrap().push("Déconnexion");
                break;
            }
            Cmd::Line(line) => {
                send_line(&mut writer, &mut reader, &line, &status);
            }
            Cmd::Jog { dx, dy, feed } => {
                let line = format!("$J=G91 G21 X{dx} Y{dy} F{feed}");
                send_line(&mut writer, &mut reader, &line, &status);
            }
            Cmd::FeedHold => {
                let _ = writer.write_all(b"!");
                status.lock().unwrap().push("! (feed hold)");
            }
            Cmd::Resume => {
                let _ = writer.write_all(b"~");
                status.lock().unwrap().push("~ (resume)");
            }
            Cmd::Job(lines) => {
                {
                    let mut s = status.lock().unwrap();
                    s.sending = true;
                    s.sent = 0;
                    s.total = lines.len();
                }
                for (i, line) in lines.iter().enumerate() {
                    // Drain any pending cancel/disconnect between lines.
                    if let Ok(Cmd::Disconnect) = rx.try_recv() {
                        status.lock().unwrap().push("Job interrompu");
                        break;
                    }
                    send_line(&mut writer, &mut reader, line, &status);
                    status.lock().unwrap().sent = i + 1;
                }
                let mut s = status.lock().unwrap();
                s.sending = false;
                s.push("Job terminé");
            }
        }
    }
    status.lock().unwrap().connected = false;
}

/// Write one line and wait (briefly) for GRBL's `ok`/`error` acknowledgement.
fn send_line(
    writer: &mut Box<dyn serialport::SerialPort>,
    reader: &mut BufReader<Box<dyn serialport::SerialPort>>,
    line: &str,
    status: &Arc<Mutex<Status>>,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(';') {
        return; // skip comments/blank lines
    }
    if writer.write_all(format!("{trimmed}\n").as_bytes()).is_err() {
        status.lock().unwrap().push(format!("Écriture échouée : {trimmed}"));
        return;
    }
    let _ = writer.flush();

    // Wait for an acknowledgement, up to ~5 s.
    for _ in 0..50 {
        let mut resp = String::new();
        match reader.read_line(&mut resp) {
            Ok(0) => break,
            Ok(_) => {
                let r = resp.trim();
                if r.eq_ignore_ascii_case("ok") {
                    return;
                }
                if r.to_ascii_lowercase().starts_with("error") || r.to_ascii_lowercase().starts_with("alarm") {
                    status.lock().unwrap().push(format!("{trimmed} -> {r}"));
                    return;
                }
                // Other chatter (status reports, banner): keep waiting.
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => {
                status.lock().unwrap().push(format!("Lecture échouée : {e}"));
                return;
            }
        }
    }
}

/// A rectangular framing path (laser off) around a bounding box, as GRBL lines.
pub fn frame_lines(bbox: (f64, f64, f64, f64), feed: f64) -> Vec<String> {
    let (minx, miny, maxx, maxy) = bbox;
    vec![
        "G21".into(),
        "G90".into(),
        "M5".into(),
        format!("G0 X{minx} Y{miny} F{feed}"),
        format!("G0 X{maxx} Y{miny}"),
        format!("G0 X{maxx} Y{maxy}"),
        format!("G0 X{minx} Y{maxy}"),
        format!("G0 X{minx} Y{miny}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_traces_the_four_corners() {
        let lines = frame_lines((0.0, 0.0, 10.0, 20.0), 3000.0);
        assert!(lines.iter().any(|l| l.contains("X10 Y0")));
        assert!(lines.iter().any(|l| l.contains("X10 Y20")));
        assert!(lines.iter().any(|l| l.contains("X0 Y20")));
        // Starts laser-off.
        assert!(lines.iter().any(|l| l == "M5"));
    }

    #[test]
    fn list_ports_does_not_panic() {
        let _ = list_ports();
    }
}
