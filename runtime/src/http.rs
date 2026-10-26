//! HTTP/1.1 клиенті: URL бөлу, DNS (`dns.rs`), TCP (`net.rs`-пен бірдей syscall-дар), chunked декодтау.
//! `https://` Linux нұсқасында қолжетімсіз (TLS жоқ): анық хабарламамен тоқтайды.
//!
//! `alloc` жоқ (`no_std`, GlobalAlloc орнатылмаған): уақытша сызбалар `Sb` (қолмен өсетін буфер) арқылы,
//! тақырыптар тізімі емес, бірден GC `Dict` ретінде құрылады.

use crate::dict::{self, Dict};
use crate::fmt::Sb;
use crate::str::*;
use crate::sys::*;

const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_CLOSE: usize = 3;

#[repr(C)]
struct SockAddrIn {
    family: u16,
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

struct Url<'a> {
    https: bool,
    host: &'a [u8],
    port: i64,
    path: &'a [u8],
}

fn parse_url(u: &[u8]) -> Result<Url<'_>, &'static str> {
    let (https, rest) = if let Some(r) = u.strip_prefix(b"http://") {
        (false, r)
    } else if let Some(r) = u.strip_prefix(b"https://") {
        (true, r)
    } else {
        return Err("URL `http://` немесе `https://`-мен басталуы керек");
    };
    let authority_end = rest.iter().position(|&c| c == b'/').unwrap_or(rest.len());
    let (authority, path) = (&rest[..authority_end], &rest[authority_end..]);
    if authority.is_empty() {
        return Err("URL-де хост жоқ");
    }
    let (host, port) = match authority.iter().position(|&c| c == b':') {
        Some(i) => {
            let p = core::str::from_utf8(&authority[i + 1..]).ok().and_then(|s| s.parse::<i64>().ok());
            (&authority[..i], p.ok_or("портты талдау мүмкін емес")?)
        }
        None => (authority, if https { 443 } else { 80 }),
    };
    Ok(Url { https, host, port, path: if path.is_empty() { b"/" } else { path } })
}

unsafe fn fail(msg: &[u8], extra: &[u8]) -> Sb {
    let mut m = Sb::new();
    m.push_bytes(msg);
    m.push_bytes(extra);
    m
}

unsafe fn tcp_connect(host: &[u8], port: i64) -> Result<i64, Sb> {
    let ip = match crate::net::resolve(host) {
        Some(ip) => ip,
        None => return Err(fail("хостты шешу мүмкін емес (DNS): ".as_bytes(), host)),
    };
    let fd = sys3(SYS_SOCKET, 2, 1, 0);
    if fd < 0 {
        return Err(fail("сокет жасау мүмкін емес".as_bytes(), b""));
    }
    let a = SockAddrIn { family: 2, port: (port as u16).to_be(), addr: ip.to_be(), zero: [0; 8] };
    if sys3(SYS_CONNECT, fd as usize, &a as *const _ as usize, 16) < 0 {
        sys3(SYS_CLOSE, fd as usize, 0, 0);
        return Err(fail("қосылу мүмкін емес: ".as_bytes(), host));
    }
    Ok(fd as i64)
}

unsafe fn send_all(fd: i64, b: &[u8]) {
    let mut off = 0;
    while off < b.len() {
        let r = sys3(SYS_WRITE, fd as usize, b.as_ptr() as usize + off, b.len() - off);
        if r <= 0 {
            if r == -4 {
                continue;
            }
            return;
        }
        off += r as usize;
    }
}

unsafe fn recv_all(fd: i64, out: &mut Sb) {
    let mut buf = [0u8; 65536];
    loop {
        let r = sys3(SYS_READ, fd as usize, buf.as_mut_ptr() as usize, buf.len());
        if r == -4 {
            continue;
        }
        if r <= 0 {
            break;
        }
        out.push_bytes(&buf[..r as usize]);
    }
}

/// Сұрау мәтінін (жол + тақырыптар + дене) `out`-қа қосады. `custom` қолданушының тақырыптар сөздігі (болмаса нөл).
unsafe fn build_request(out: &mut Sb, method: &[u8], host: &[u8], port: i64, https: bool, path: &[u8], custom: Dict, body: &[u8]) {
    out.push_bytes(method);
    out.push(b' ');
    out.push_bytes(path);
    out.push_bytes(b" HTTP/1.1\r\nHost: ");
    out.push_bytes(host);
    if port != if https { 443 } else { 80 } {
        out.push(b':');
        let mut b = [0u8; 24];
        out.push_bytes(fmt_u64(port as u64, &mut b));
    }
    out.push_bytes(b"\r\nUser-Agent: tulpar\r\nAccept: */*\r\nConnection: close\r\n");
    let mut has_cl = false;
    if !custom.is_null() {
        let mut pos = 0;
        loop {
            pos = dict::tlp_dict_next(custom, pos);
            if pos < 0 {
                break;
            }
            let k = bytes(dict::tlp_dict_key_at(custom, pos) as Str);
            let v = bytes(dict::tlp_dict_val_at(custom, pos) as Str);
            if !(k.eq_ignore_ascii_case(b"content-length") || k.eq_ignore_ascii_case(b"host") || k.eq_ignore_ascii_case(b"connection")) {
                has_cl |= k.eq_ignore_ascii_case(b"transfer-encoding");
                out.push_bytes(k);
                out.push_bytes(b": ");
                out.push_bytes(v);
                out.push_bytes(b"\r\n");
            }
            pos += 1;
        }
    }
    if !body.is_empty() && !has_cl {
        out.push_bytes(b"Content-Length: ");
        let mut b = [0u8; 24];
        out.push_bytes(fmt_u64(body.len() as u64, &mut b));
        out.push_bytes(b"\r\n");
    }
    out.push_bytes(b"\r\n");
    out.push_bytes(body);
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// chunked денені шешіп, GC `Str`-ге жазады.
unsafe fn dechunk(b: &[u8]) -> Str {
    let mut sb = Sb::new();
    let mut i = 0;
    while i < b.len() {
        let line_end = match find(&b[i..], b"\r\n") {
            Some(p) => i + p,
            None => break,
        };
        let size_hex = core::str::from_utf8(&b[i..line_end]).ok().map(|s| s.trim());
        let size = match size_hex.and_then(|s| usize::from_str_radix(s, 16).ok()) {
            Some(s) => s,
            None => break,
        };
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        if start + size > b.len() {
            sb.push_bytes(&b[start..]);
            break;
        }
        sb.push_bytes(&b[start..start + size]);
        i = start + size + 2; // деректен кейінгі \r\n
    }
    sb.finish()
}

/// Жауапты талдайды: (код, тақырыптар GC `Dict`, дене GC `Str`). HTTP/1.0 да, chunked да түсінеді.
unsafe fn parse_response(raw: &[u8]) -> Option<(i64, Dict, Str)> {
    let head_end = find(raw, b"\r\n\r\n")?;
    let head = &raw[..head_end];
    let raw_body = &raw[head_end + 4..];
    let mut lines = head.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let status_line = lines.next()?;
    let sp1 = status_line.iter().position(|&c| c == b' ')?;
    let rest = &status_line[sp1 + 1..];
    let sp2 = rest.iter().position(|&c| c == b' ').unwrap_or(rest.len());
    let code: i64 = core::str::from_utf8(&rest[..sp2]).ok()?.parse().ok()?;

    let hd = dict::tlp_dict_new(1, 0);
    let mut chunked = false;
    for l in lines {
        if l.is_empty() {
            continue;
        }
        let colon = match l.iter().position(|&c| c == b':') {
            Some(i) => i,
            None => continue,
        };
        let mut v = &l[colon + 1..];
        while v.first() == Some(&b' ') {
            v = &v[1..];
        }
        let mut kbuf = [0u8; 128];
        let klen = colon.min(128);
        for i in 0..klen {
            kbuf[i] = l[i].to_ascii_lowercase();
        }
        if &kbuf[..klen] == b"transfer-encoding" && v.eq_ignore_ascii_case(b"chunked") {
            chunked = true;
        }
        let ks = from_bytes(&kbuf[..klen]) as u64;
        let vs = from_bytes(v) as u64;
        dict::tlp_dict_set(hd, ks, vs);
    }
    let body = if chunked { dechunk(raw_body) } else { from_bytes(raw_body) };
    Some((code, hd, body))
}

/// Бір рет сұрайды, қайта бағыттауды (3xx) ермейді. `(код, тақырыптар, дене)` үш сөзін `TK::TupleLit`
/// жайымымен бірдей GC объектісіне жазады: компилятор мұны `Ty::Tuple([бүтін, сөздік[мәтін,мәтін], мәтін])` деп таниды.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_http_request(url: Str, method: Str, body: Str, headers: Dict) -> *mut u64 {
    let (code, hd, bs) = match request_once(bytes(url), bytes(method), bytes(body), headers) {
        Ok(v) => v,
        Err(m) => fatal(m.bytes()),
    };
    let obj = crate::gc::alloc(24, false) as *mut u64;
    *obj = code as u64;
    *obj.add(1) = hd as u64;
    *obj.add(2) = bs as u64;
    obj
}

unsafe fn request_once(url: &[u8], method: &[u8], body: &[u8], headers: Dict) -> Result<(i64, Dict, Str), Sb> {
    let u = parse_url(url).map_err(|e| {
        let mut m = Sb::new();
        m.push_bytes(e.as_bytes());
        m
    })?;
    if u.https {
        return Err(fail(
            "HTTPS Linux нұсқасында әзірге қолжетімсіз (TLS жоқ); Windows нұсқасында (`tulpar build --target windows`) жұмыс істейді"
                .as_bytes(),
            b"",
        ));
    }
    let mut req = Sb::new();
    build_request(&mut req, method, u.host, u.port, u.https, u.path, headers, body);
    let fd = tcp_connect(u.host, u.port)?;
    out_flush();
    send_all(fd, req.bytes());
    req.discard();
    let mut resp = Sb::new();
    recv_all(fd, &mut resp);
    sys3(SYS_CLOSE, fd as usize, 0, 0);
    let r = parse_response(resp.bytes());
    resp.discard();
    r.ok_or_else(|| fail("жауапты талдау мүмкін емес (жарамсыз HTTP)".as_bytes(), b""))
}

/// `Location` тақырыбы қатысты болса, оны ағымдағы URL-ге қатысты толық URL-ге айналдырады.
unsafe fn resolve_redirect(base: &[u8], loc: &[u8]) -> Str {
    if loc.starts_with(b"http://") || loc.starts_with(b"https://") {
        return from_bytes(loc);
    }
    let mut out = Sb::new();
    match parse_url(base) {
        Ok(u) => {
            out.push_bytes(if u.https { b"https://" } else { b"http://" });
            out.push_bytes(u.host);
            if u.port != if u.https { 443 } else { 80 } {
                out.push(b':');
                let mut b = [0u8; 24];
                out.push_bytes(fmt_u64(u.port as u64, &mut b));
            }
        }
        Err(_) => {}
    }
    if !loc.starts_with(b"/") {
        out.push(b'/');
    }
    out.push_bytes(loc);
    out.finish()
}

/// Жай GET: қайта бағыттауды (3xx, `Location`) 10-ға дейін ереді, денені мәтін ретінде қайтарады.
/// Байланыс қатесінде немесе соңғы жауап коды >= 400 болса — бағдарламаны қате хабарламасымен тоқтатады.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tlp_http_get(url: Str) -> Str {
    let none: Dict = core::ptr::null_mut();
    let mut cur: Str = url;
    for _ in 0..10 {
        let (code, hd, body) = match request_once(bytes(cur), b"GET", b"", none) {
            Ok(v) => v,
            Err(m) => fatal(m.bytes()),
        };
        if (300..400).contains(&code) {
            let loc = dict::tlp_dict_get_or(hd, from_str("location") as u64, 0) as Str;
            if !loc.is_null() {
                cur = resolve_redirect(bytes(cur), bytes(loc));
                continue;
            }
        }
        if code >= 400 {
            let mut m = Sb::new();
            m.push_bytes("HTTP қатесі ".as_bytes());
            let mut b = [0u8; 24];
            m.push_bytes(fmt_u64(code as u64, &mut b));
            m.push_bytes(b": ");
            m.push_bytes(bytes(cur));
            fatal(m.bytes());
        }
        return body;
    }
    fatal("тым көп қайта бағыттау (10-нан асты)".as_bytes())
}
