//! DNS: тек A жазбалары (IPv4), UDP арқылы, `/etc/resolv.conf`-тағы resolver адресін қолданып.
//! glibc/`getaddrinfo` жоқ (libc жоқ): сұрау пакеті қолмен құрылады да, syscall-дармен UDP арқылы жіберіледі.

use crate::sys::*;

const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_CLOSE: usize = 3;
const SYS_SETSOCKOPT: usize = 54;

#[repr(C)]
struct SockAddrIn {
    family: u16,
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

fn parse_ipv4_literal(b: &[u8]) -> Option<u32> {
    let mut parts = [0u32; 4];
    let mut n = 0;
    let mut cur: Option<u32> = None;
    for &c in b {
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
            parts[n] = cur.take()?;
            n += 1;
        } else {
            return None;
        }
    }
    if n != 3 {
        return None;
    }
    parts[3] = cur?;
    Some((parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3])
}

/// `/etc/resolv.conf`-тан бірінші `nameserver` жолын оқиды; таппаса 8.8.8.8 (Google DNS) қолданады.
unsafe fn resolver_ip() -> u32 {
    let path = b"/etc/resolv.conf\0";
    let fd = sys3(2, path.as_ptr() as usize, 0, 0); // open O_RDONLY
    if fd < 0 {
        return 0x0808_0808;
    }
    let mut buf = [0u8; 4096];
    let mut n = 0usize;
    loop {
        let r = sys3(SYS_READ, fd as usize, (buf.as_mut_ptr() as usize) + n, buf.len() - n);
        if r <= 0 || n >= buf.len() {
            break;
        }
        n += r as usize;
    }
    sys3(SYS_CLOSE, fd as usize, 0, 0);
    let text = &buf[..n];
    let mut i = 0;
    while i < text.len() {
        let eol = text[i..].iter().position(|&c| c == b'\n').map(|p| i + p).unwrap_or(text.len());
        let line = {
            let mut l = &text[i..eol];
            while l.first() == Some(&b' ') || l.first() == Some(&b'\t') {
                l = &l[1..];
            }
            l
        };
        if line.starts_with(b"nameserver") {
            let rest = &line[b"nameserver".len()..];
            let rest = rest.iter().position(|&c| c != b' ' && c != b'\t').map(|p| &rest[p..]).unwrap_or(&[]);
            let end = rest.iter().position(|&c| c == b' ' || c == b'\t' || c == b'\r').unwrap_or(rest.len());
            if let Some(ip) = parse_ipv4_literal(&rest[..end]) {
                return ip;
            }
        }
        i = eol + 1;
    }
    0x0808_0808
}

/// A жазбасын сұрайды (host байттары, нокта-ноталы); сәтсіз болса `None`.
pub unsafe fn resolve_a(host: &[u8]) -> Option<u32> {
    if host.len() > 253 || host.is_empty() {
        return None;
    }
    let ns = resolver_ip();

    // сұрау пакеті: тақырып (12 байт) + QNAME + QTYPE(A=1) + QCLASS(IN=1)
    let mut q = [0u8; 12 + 256];
    let mut ts = [0i64; 2];
    sys3(228, 1, ts.as_mut_ptr() as usize, 0); // CLOCK_MONOTONIC: id ретінде нанасекунд биттерінің төменгі жартысы
    let id = ts[1] as u16;
    q[0] = (id >> 8) as u8;
    q[1] = id as u8;
    q[2] = 0x01; // RD (recursion desired)
    q[5] = 1; // QDCOUNT = 1
    let mut p = 12usize;
    for label in host.split(|&c| c == b'.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q[p] = label.len() as u8;
        p += 1;
        q[p..p + label.len()].copy_from_slice(label);
        p += label.len();
    }
    q[p] = 0; // QNAME аяқталуы
    p += 1;
    q[p] = 0;
    q[p + 1] = 1; // QTYPE = A
    q[p + 2] = 0;
    q[p + 3] = 1; // QCLASS = IN
    let qlen = p + 4;

    let fd = sys3(SYS_SOCKET, 2, 2, 17); // AF_INET, SOCK_DGRAM, IPPROTO_UDP
    if fd < 0 {
        return None;
    }
    let a = SockAddrIn { family: 2, port: 53u16.to_be(), addr: ns.to_be(), zero: [0; 8] };
    if sys3(SYS_CONNECT, fd as usize, &a as *const _ as usize, 16) < 0 {
        sys3(SYS_CLOSE, fd as usize, 0, 0);
        return None;
    }
    if sys3(SYS_WRITE, fd as usize, q.as_ptr() as usize, qlen) < 0 {
        sys3(SYS_CLOSE, fd as usize, 0, 0);
        return None;
    }

    let mut r = [0u8; 512];
    let mut rn: isize = 0;
    // 2 секундқа дейін күтеміз (жауап жоқ болса DNS серверден жауап мүлде келмейді)
    let tv: [i64; 2] = [2, 0];
    sys6(SYS_SETSOCKOPT, fd as usize, 1, 20, tv.as_ptr() as usize, 16, 0); // SOL_SOCKET=1, SO_RCVTIMEO=20
    for _ in 0..3 {
        rn = sys3(SYS_READ, fd as usize, r.as_mut_ptr() as usize, r.len());
        if rn != -4 {
            break;
        }
    }
    sys3(SYS_CLOSE, fd as usize, 0, 0);
    if rn < 12 {
        return None;
    }
    let resp = &r[..rn as usize];
    if resp[0] != q[0] || resp[1] != q[1] {
        return None; // басқа сұраудың жауабы
    }
    let ancount = ((resp[6] as usize) << 8) | resp[7] as usize;
    if ancount == 0 {
        return None;
    }

    // атауды (сұрақ немесе жауап жазбасының NAME өрісін) өткізіп жібереді: қысқарту көрсеткіштерін де түсінеді
    fn skip_name(b: &[u8], mut i: usize) -> Option<usize> {
        loop {
            if i >= b.len() {
                return None;
            }
            let len = b[i];
            if len == 0 {
                return Some(i + 1);
            }
            if len & 0xC0 == 0xC0 {
                return Some(i + 2); // қысқарту көрсеткіші: 2 байт, мазмұнын оқу қажет емес
            }
            i += 1 + len as usize;
        }
    }

    let mut off = skip_name(resp, 12)? + 4; // сұрақ бөлімі: QNAME + QTYPE(2) + QCLASS(2)
    for _ in 0..ancount {
        off = skip_name(resp, off)?;
        if off + 10 > resp.len() {
            return None;
        }
        let rtype = ((resp[off] as u16) << 8) | resp[off + 1] as u16;
        let rclass = ((resp[off + 2] as u16) << 8) | resp[off + 3] as u16;
        let rdlen = ((resp[off + 8] as usize) << 8) | resp[off + 9] as usize;
        off += 10;
        if off + rdlen > resp.len() {
            return None;
        }
        if rtype == 1 && rclass == 1 && rdlen == 4 {
            return Some(u32::from_be_bytes([resp[off], resp[off + 1], resp[off + 2], resp[off + 3]]));
        }
        off += rdlen;
    }
    None
}
