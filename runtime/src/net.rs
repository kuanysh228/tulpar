//! TCP: сервер мен клиент (IPv4), тікелей syscall-дармен.

use crate::fmt::Sb;
use crate::str::*;
use crate::sys::*;

const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_ACCEPT: usize = 43;
const SYS_SENDTO: usize = 44;
const SYS_BIND: usize = 49;
const SYS_LISTEN: usize = 50;
const SYS_SETSOCKOPT: usize = 54;
const SYS_CLOSE: usize = 3;

#[repr(C)]
struct SockAddrIn {
    family: u16,
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

fn addr(ip: u32, port: i64) -> SockAddrIn {
    SockAddrIn { family: 2, port: (port as u16).to_be(), addr: ip.to_be(), zero: [0; 8] }
}

/// Сервер сокетін барлық интерфейстерде ашады, fd қайтарады.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen(port: i64) -> i64 {
    listen_on(0, port)
}

/// Тек 127.0.0.1 бойынша тыңдайды (сыртқы желіден қолжетімсіз).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen_local(port: i64) -> i64 {
    listen_on(0x7f00_0001, port)
}

unsafe fn listen_on(ip: u32, port: i64) -> i64 {
    let fd = sys3(SYS_SOCKET, 2, 1, 0);
    if fd < 0 {
        fatal("сокет жасау мүмкін емес".as_bytes());
    }
    let one: i32 = 1;
    sys6(SYS_SETSOCKOPT, fd as usize, 1, 2, &one as *const _ as usize, 4, 0); // SO_REUSEADDR
    let a = addr(ip, port);
    if sys3(SYS_BIND, fd as usize, &a as *const _ as usize, 16) < 0 {
        let mut sb = Sb::new();
        sb.push_bytes("портты тыңдау мүмкін емес (бос емес пе?): ".as_bytes());
        let mut b = [0u8; 24];
        sb.push_bytes(fmt_i64(port, &mut b));
        fatal(sb.bytes());
    }
    sys3(SYS_LISTEN, fd as usize, 128, 0);
    fd as i64
}

/// Келесі қосылымды күтеді, клиент fd-ін қайтарады (қате болса -1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_accept(server: i64) -> i64 {
    out_flush();
    loop {
        let r = sys3(SYS_ACCEPT, server as usize, 0, 0);
        if r == -4 {
            continue;
        }
        return if r < 0 { -1 } else { r as i64 };
    }
}

fn parse_ip(h: &[u8]) -> Option<u32> {
    if h == b"localhost" {
        return Some(0x7f00_0001);
    }
    let mut parts = [0u32; 4];
    let mut n = 0;
    let mut cur: Option<u32> = None;
    for &c in h {
        if c.is_ascii_digit() {
            let v = cur.unwrap_or(0) * 10 + (c - b'0') as u32;
            if v > 255 {
                return None;
            }
            cur = Some(v);
        } else if c == b'.' {
            if n >= 3 || cur.is_none() {
                return None;
            }
            parts[n] = cur.take().unwrap();
            n += 1;
        } else {
            return None;
        }
    }
    if n != 3 || cur.is_none() {
        return None;
    }
    parts[3] = cur.unwrap();
    Some((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3])
}

/// IPv4 мекенжайына (`127.0.0.1` немесе `localhost`) қосылады; сәтсіз болса -1. DNS жоқ.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_connect(host: Str, port: i64) -> i64 {
    let ip = match parse_ip(bytes(host)) {
        Some(ip) => ip,
        None => return -1,
    };
    let fd = sys3(SYS_SOCKET, 2, 1, 0);
    if fd < 0 {
        return -1;
    }
    let a = addr(ip, port);
    if sys3(SYS_CONNECT, fd as usize, &a as *const _ as usize, 16) < 0 {
        sys3(SYS_CLOSE, fd as usize, 0, 0);
        return -1;
    }
    fd as i64
}

/// Бір рет оқиды (64 КБ дейін). Қосылым жабылса — бос мәтін.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_read(fd: i64) -> Str {
    let mut buf = [0u8; 65536];
    loop {
        let r = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        if r == -4 {
            continue;
        }
        if r <= 0 {
            return from_bytes(&[]);
        }
        return from_bytes(&buf[..r as usize]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_write(fd: i64, text: Str) {
    let b = bytes(text);
    let mut off = 0;
    while off < b.len() {
        // MSG_NOSIGNAL: жабылған сокетке жазғанда SIGPIPE болмасын
        let r = sys6(SYS_SENDTO, fd as usize, b.as_ptr() as usize + off, b.len() - off, 0x4000, 0, 0);
        if r == -4 {
            continue;
        }
        if r <= 0 {
            return;
        }
        off += r as usize;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_close(fd: i64) {
    sys3(SYS_CLOSE, fd as usize, 0, 0);
}
