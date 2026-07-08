//! GDB Remote Serial Protocol server (the rp2040js gdbserver surface).
//!
//! Supports: register read/write (`g`/`G`/`p`/`P`), memory (`m`/`M`),
//! step/continue (`s`/`c`/`vCont`), Ctrl-C interrupt, and the handshake
//! queries GDB sends. Breakpoints work the classic way: GDB writes BKPT
//! opcodes via `M`; when one hits, the server rewinds PC and reports S05.
//!
//! ```text
//! cargo run --release --example gdbserver -- firmware.elf 3333
//! gdb-multiarch -ex "target remote :3333" firmware.elf
//! ```

use crate::Rt1060;
use crate::cortex_m::{BreakCause, PC};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// Target description: tells GDB this is an M-profile core with exactly
/// r0-r15 + xPSR in the `g` packet (else it assumes the legacy FPA layout).
const TARGET_XML: &str = r#"<?xml version="1.0"?>
<!DOCTYPE target SYSTEM "gdb-target.dtd">
<target version="1.0">
<architecture>arm</architecture>
<feature name="org.gnu.gdb.arm.m-profile">
<reg name="r0" bitsize="32"/><reg name="r1" bitsize="32"/>
<reg name="r2" bitsize="32"/><reg name="r3" bitsize="32"/>
<reg name="r4" bitsize="32"/><reg name="r5" bitsize="32"/>
<reg name="r6" bitsize="32"/><reg name="r7" bitsize="32"/>
<reg name="r8" bitsize="32"/><reg name="r9" bitsize="32"/>
<reg name="r10" bitsize="32"/><reg name="r11" bitsize="32"/>
<reg name="r12" bitsize="32"/>
<reg name="sp" bitsize="32" type="data_ptr"/>
<reg name="lr" bitsize="32"/>
<reg name="pc" bitsize="32" type="code_ptr"/>
<reg name="xpsr" bitsize="32"/>
</feature>
</target>"#;

fn checksum(payload: &[u8]) -> u8 {
    payload.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Encode a register value as GDB's little-endian hex octets.
fn encode_u32_le(value: u32) -> String {
    value
        .to_le_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn decode_u32_le(hex: &str) -> Option<u32> {
    if hex.len() != 8 {
        return None;
    }
    let mut bytes = [0u8; 4];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(u32::from_le_bytes(bytes))
}

/// One GDB client session driving the simulator.
pub struct GdbServer {
    stream: TcpStream,
    rx: Vec<u8>,
    /// Instructions per continue-chunk between socket polls.
    pub chunk: u64,
}

impl GdbServer {
    /// Block until a debugger connects on `addr` (e.g. "127.0.0.1:3333").
    pub fn accept(addr: &str) -> std::io::Result<(Self, std::net::SocketAddr)> {
        let listener = TcpListener::bind(addr)?;
        let local = listener.local_addr()?;
        let (stream, _) = listener.accept()?;
        stream.set_nodelay(true).ok();
        Ok((
            GdbServer {
                stream,
                rx: Vec::new(),
                chunk: 100_000,
            },
            local,
        ))
    }

    pub fn from_stream(stream: TcpStream) -> Self {
        stream.set_nodelay(true).ok();
        GdbServer {
            stream,
            rx: Vec::new(),
            chunk: 100_000,
        }
    }

    fn send_packet(&mut self, payload: &str) -> std::io::Result<()> {
        let framed = format!("${}#{:02x}", payload, checksum(payload.as_bytes()));
        self.stream.write_all(framed.as_bytes())
    }

    /// Pull one complete packet out of the receive buffer, if present.
    /// Returns the payload; acks are sent, Ctrl-C surfaces as "\x03".
    fn next_packet(&mut self) -> std::io::Result<Option<String>> {
        loop {
            // Interrupt byte outside a packet.
            if let Some(&first) = self.rx.first() {
                match first {
                    0x03 => {
                        self.rx.remove(0);
                        return Ok(Some("\x03".into()));
                    }
                    b'+' | b'-' => {
                        self.rx.remove(0);
                        continue;
                    }
                    _ => {}
                }
            }
            if let Some(start) = self.rx.iter().position(|&b| b == b'$')
                && let Some(hash) = self.rx[start..].iter().position(|&b| b == b'#')
            {
                let hash = start + hash;
                if self.rx.len() >= hash + 3 {
                    let payload: Vec<u8> = self.rx[start + 1..hash].to_vec();
                    let sum = hex_val(self.rx[hash + 1]).unwrap_or(0) << 4
                        | hex_val(self.rx[hash + 2]).unwrap_or(0);
                    self.rx.drain(..hash + 3);
                    if sum == checksum(&payload) {
                        self.stream.write_all(b"+")?;
                        return Ok(Some(String::from_utf8_lossy(&payload).into_owned()));
                    }
                    self.stream.write_all(b"-")?;
                    continue;
                }
            }
            return Ok(None);
        }
    }

    fn fill_rx(&mut self, blocking: bool) -> std::io::Result<bool> {
        self.stream.set_nonblocking(!blocking)?;
        let mut buf = [0u8; 4096];
        match self.stream.read(&mut buf) {
            Ok(0) => Ok(false), // peer closed
            Ok(n) => {
                self.rx.extend_from_slice(&buf[..n]);
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Serve the session until the debugger detaches or disconnects.
    pub fn run(&mut self, sim: &mut Rt1060) -> std::io::Result<()> {
        loop {
            let packet = loop {
                if let Some(p) = self.next_packet()? {
                    break p;
                }
                if !self.fill_rx(true)? {
                    return Ok(());
                }
            };
            if !self.handle(sim, &packet)? {
                return Ok(());
            }
        }
    }

    /// Handle one packet; false = session over.
    fn handle(&mut self, sim: &mut Rt1060, packet: &str) -> std::io::Result<bool> {
        match packet {
            "\x03" | "?" => self.send_packet("S05")?,
            p if p.starts_with("qSupported") => {
                self.send_packet("PacketSize=4000;vContSupported+;qXfer:features:read+")?
            }
            p if p.starts_with("qXfer:features:read:target.xml:") => {
                let spec = &p["qXfer:features:read:target.xml:".len()..];
                let (off, len) = spec.split_once(',').unwrap_or(("0", "0"));
                let off = usize::from_str_radix(off, 16).unwrap_or(0);
                let len = usize::from_str_radix(len, 16).unwrap_or(0);
                let xml = TARGET_XML.as_bytes();
                if off >= xml.len() {
                    self.send_packet("l")?;
                } else {
                    let end = (off + len).min(xml.len());
                    let marker = if end == xml.len() { 'l' } else { 'm' };
                    let chunk = String::from_utf8_lossy(&xml[off..end]);
                    self.send_packet(&format!("{marker}{chunk}"))?;
                }
            }
            "qAttached" => self.send_packet("1")?,
            p if p.starts_with('H') => self.send_packet("OK")?,
            "vCont?" => self.send_packet("vCont;c;s")?,
            "g" => {
                let mut regs = String::new();
                for i in 0..16 {
                    regs.push_str(&encode_u32_le(sim.core.regs[i]));
                }
                regs.push_str(&encode_u32_le(sim.core.xpsr()));
                self.send_packet(&regs)?;
            }
            p if p.starts_with('G') => {
                let hex = &p[1..];
                for i in 0..16 {
                    if let Some(v) = decode_u32_le(&hex[8 * i..8 * i + 8]) {
                        sim.core.regs[i] = v;
                    }
                }
                self.send_packet("OK")?;
            }
            p if p.starts_with('p') => {
                let n = usize::from_str_radix(&p[1..], 16).unwrap_or(99);
                let v = match n {
                    0..=15 => sim.core.regs[n],
                    16 | 25 => sim.core.xpsr(),
                    _ => 0,
                };
                self.send_packet(&encode_u32_le(v))?;
            }
            p if p.starts_with('P') => {
                if let Some((reg, val)) = p[1..].split_once('=') {
                    let n = usize::from_str_radix(reg, 16).unwrap_or(99);
                    if let (true, Some(v)) = (n < 16, decode_u32_le(val)) {
                        sim.core.regs[n] = if n == 15 { v & !1 } else { v };
                    }
                }
                self.send_packet("OK")?;
            }
            p if p.starts_with('m') => {
                if let Some((addr, len)) = p[1..].split_once(',') {
                    let addr = u32::from_str_radix(addr, 16).unwrap_or(0);
                    let len = u32::from_str_radix(len, 16).unwrap_or(0).min(0x4000);
                    let hex: String = (0..len)
                        .map(|i| format!("{:02x}", sim.bus.read8(addr + i)))
                        .collect();
                    self.send_packet(&hex)?;
                } else {
                    self.send_packet("E01")?;
                }
            }
            p if p.starts_with('M') => {
                if let Some((spec, data)) = p[1..].split_once(':')
                    && let Some((addr, _len)) = spec.split_once(',')
                {
                    let addr = u32::from_str_radix(addr, 16).unwrap_or(0);
                    for i in 0..data.len() / 2 {
                        if let Ok(b) = u8::from_str_radix(&data[2 * i..2 * i + 2], 16) {
                            sim.bus.write8(addr + i as u32, b);
                        }
                    }
                }
                self.send_packet("OK")?;
            }
            "s" | "vCont;s" | "vCont;s:1" => {
                sim.step();
                if let Some(BreakCause::Bkpt(_)) = sim.core.break_cause {
                    sim.core.regs[PC] = sim.core.regs[PC].wrapping_sub(2);
                }
                sim.core.break_cause = None;
                self.send_packet("S05")?;
            }
            "c" | "vCont;c" | "vCont;c:1" => {
                self.continue_until_stop(sim)?;
            }
            "D" => {
                self.send_packet("OK")?;
                return Ok(false);
            }
            "k" => return Ok(false),
            _ => self.send_packet("")?, // unsupported → empty per RSP
        }
        Ok(true)
    }

    /// Continue execution, polling the socket for Ctrl-C between chunks.
    fn continue_until_stop(&mut self, sim: &mut Rt1060) -> std::io::Result<()> {
        loop {
            let cause = sim.run(self.chunk);
            match cause {
                Some(BreakCause::Bkpt(_)) => {
                    // GDB planted this via M: rewind so the resumed PC
                    // points at the original (restored) instruction.
                    sim.core.regs[PC] = sim.core.regs[PC].wrapping_sub(2);
                    sim.core.break_cause = None;
                    self.send_packet("S05")?;
                    return Ok(());
                }
                Some(_) => {
                    sim.core.break_cause = None;
                    self.send_packet("S05")?;
                    return Ok(());
                }
                None => {
                    // Poll for an interrupt byte without blocking.
                    if !self.fill_rx(false)? {
                        return Ok(());
                    }
                    if self.rx.first() == Some(&0x03) {
                        self.rx.remove(0);
                        self.send_packet("S02")?;
                        return Ok(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_and_codec() {
        assert_eq!(checksum(b"OK"), 0x9a);
        assert_eq!(encode_u32_le(0x1234_5678), "78563412");
        assert_eq!(decode_u32_le("78563412"), Some(0x1234_5678));
    }

    #[test]
    fn full_session_over_tcp() {
        use crate::memory::map;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let mut sim = Rt1060::new();
            sim.bus.log_unmapped = false;
            // movs r0,#42 ; nop ; bkpt 0
            sim.bus.write16(map::DTCM_BASE, 0x202a);
            sim.bus.write16(map::DTCM_BASE + 2, 0xbf00);
            sim.bus.write16(map::DTCM_BASE + 4, 0xbe00);
            sim.core.regs[PC] = map::DTCM_BASE;
            let (stream, _) = listener.accept().unwrap();
            let mut gdb = GdbServer::from_stream(stream);
            gdb.run(&mut sim).unwrap();
            sim
        });

        let mut client = TcpStream::connect(addr).unwrap();
        fn send(client: &mut TcpStream, payload: &str) {
            let framed = format!("${}#{:02x}", payload, checksum(payload.as_bytes()));
            client.write_all(framed.as_bytes()).unwrap();
        }
        let mut buf = Vec::new();
        let recv = |client: &mut TcpStream, buf: &mut Vec<u8>| -> String {
            // Read until a complete packet is buffered, then strip framing.
            loop {
                if let Some(start) = buf.iter().position(|&b| b == b'$')
                    && let Some(off) = buf[start..].iter().position(|&b| b == b'#')
                    && buf.len() >= start + off + 3
                {
                    let payload =
                        String::from_utf8_lossy(&buf[start + 1..start + off]).into_owned();
                    buf.drain(..start + off + 3);
                    return payload;
                }
                let mut tmp = [0u8; 1024];
                let n = client.read(&mut tmp).unwrap();
                assert!(n > 0, "server hung up early");
                buf.extend_from_slice(&tmp[..n]);
            }
        };

        send(&mut client, "qSupported:multiprocess+");
        assert!(recv(&mut client, &mut buf).contains("vContSupported+"));
        send(&mut client, "?");
        assert_eq!(recv(&mut client, &mut buf), "S05");
        // Step one instruction: r0 becomes 42.
        send(&mut client, "s");
        assert_eq!(recv(&mut client, &mut buf), "S05");
        send(&mut client, "g");
        let regs = recv(&mut client, &mut buf);
        assert_eq!(&regs[..8], "2a000000"); // r0 = 42 LE
        // Memory read of the code we planted.
        send(&mut client, &format!("m{:x},2", 0x2000_0000u32));
        assert_eq!(recv(&mut client, &mut buf), "2a20");
        // Continue: runs into BKPT, reports S05, PC rewound onto it.
        send(&mut client, "c");
        assert_eq!(recv(&mut client, &mut buf), "S05");
        send(&mut client, &format!("p{:x}", 15));
        assert_eq!(recv(&mut client, &mut buf), encode_u32_le(0x2000_0004));
        // Detach ends the session.
        send(&mut client, "D");
        assert_eq!(recv(&mut client, &mut buf), "OK");
        let sim = server.join().unwrap();
        assert_eq!(sim.core.regs[0], 42);
    }
}
