//! Windows: TCP (IPv4) Winsock арқылы; мінез-құлық Linux нұсқасымен бірдей.

use crate::str::*;
use crate::sys::*;
use crate::win;

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

static mut STARTED: bool = false;

unsafe fn init() {
    unsafe {
        if !STARTED {
            win::ws_keep();
            win::ws_startup();
            STARTED = true;
        }
    }
}

/// Сервер сокетін барлық интерфейстерде ашады, сокет нөмірін қайтарады.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen(port: i64) -> i64 {
    unsafe { listen_on(0, port) }
}

/// Тек 127.0.0.1 бойынша тыңдайды (сыртқы желіден қолжетімсіз).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_listen_local(port: i64) -> i64 {
    unsafe { listen_on(0x7f00_0001, port) }
}

unsafe fn listen_on(ip: u32, port: i64) -> i64 {
    unsafe {
        init();
        let s = win::ws_socket();
        if s == win::INVALID_SOCKET {
            fatal("сокет жасау мүмкін емес".as_bytes());
        }
        let a = addr(ip, port);
        if win::ws_bind(s, &a as *const _ as *const u8, 16) != 0 {
            let mut sb = crate::fmt::Sb::new();
            sb.push_bytes("портты тыңдау мүмкін емес (бос емес пе?): ".as_bytes());
            let mut b = [0u8; 24];
            sb.push_bytes(fmt_i64(port, &mut b));
            fatal(sb.bytes());
        }
        win::ws_listen(s, 128);
        s as i64
    }
}

/// Келесі қосылымды күтеді, клиент сокетін қайтарады (қате болса -1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_accept(server: i64) -> i64 {
    unsafe {
        out_flush();
        let r = win::ws_accept(server as usize);
        if r == win::INVALID_SOCKET { -1 } else { r as i64 }
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

/// Хост атауын IPv4 мекенжайға айналдырады: әуелі литерал (`1.2.3.4`, `localhost`), болмаса `GetAddrInfoW` (нақты DNS).
pub unsafe fn resolve(host: &[u8]) -> Option<u32> {
    unsafe {
        if let Some(ip) = parse_ip(host) {
            return Some(ip);
        }
        init(); // GetAddrInfoW де Winsock функциясы: WSAStartup алдында шақыру анықталмаған
        let mut w = [0u16; crate::misc::WCAP];
        crate::misc::to_wide(host, &mut w)?;
        let mut res: *mut win::AddrInfoW = core::ptr::null_mut();
        if win::getaddrinfo_ipv4(w.as_ptr(), &mut res) != 0 || res.is_null() {
            return None;
        }
        // sockaddr_in: [family:u16][port:u16][addr:u32]...
        let ip = u32::from_be(core::ptr::read_unaligned((*res).ai_addr.add(4) as *const u32));
        win::freeaddrinfo(res);
        Some(ip)
    }
}

/// Хостқа (IPv4 литералы, `localhost` немесе доменнің нақты DNS атауы) қосылады; сәтсіз болса -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_connect(host: Str, port: i64) -> i64 {
    unsafe {
        let ip = match resolve(bytes(host)) {
            Some(ip) => ip,
            None => return -1,
        };
        init();
        let s = win::ws_socket();
        if s == win::INVALID_SOCKET {
            return -1;
        }
        let a = addr(ip, port);
        if win::ws_connect(s, &a as *const _ as *const u8, 16) != 0 {
            win::ws_close(s);
            return -1;
        }
        s as i64
    }
}

/// Бір рет оқиды (64 КБ дейін). Қосылым жабылса — бос мәтін.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_read(fd: i64) -> Str {
    unsafe {
        let mut buf = [0u8; 65536];
        let r = win::ws_recv(fd as usize, buf.as_mut_ptr(), buf.len() as i32);
        if r <= 0 {
            return from_bytes(&[]);
        }
        from_bytes(&buf[..r as usize])
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_write(fd: i64, text: Str) {
    unsafe {
        let b = bytes(text);
        let mut off = 0;
        while off < b.len() {
            let r = win::ws_send(fd as usize, b.as_ptr().add(off), (b.len() - off).min(1 << 20) as i32);
            if r <= 0 {
                return;
            }
            off += r as usize;
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_close(fd: i64) {
    unsafe {
        win::ws_close(fd as usize);
    }
}

/// Сокеттің оқу/жазу күту уақытын (миллисекунд) белгілейді. 0 — шексіз.
/// Күту біткенде `желі_оқы` бос мәтін қайтарады (қосылым жабылғандай).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_net_timeout(fd: i64, ms: i64) {
    unsafe {
        // SO_RCVTIMEO = 0x1006, SO_SNDTIMEO = 0x1005
        win::ws_setsockopt_u32(fd as usize, 0x1006, ms as u32);
        win::ws_setsockopt_u32(fd as usize, 0x1005, ms as u32);
    }
}
