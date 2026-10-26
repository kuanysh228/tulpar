//! HTTP/1.1 (және HTTPS, `tls.rs` арқылы) клиенті: URL бөлу, DNS (`net::resolve`), chunked декодтау.
//! Linux нұсқасымен (`runtime/src/http.rs`) бірдей қарапайым дизайн, бірақ TLS қолдауымен.

use crate::dict::{self, Dict};
use crate::fmt::Sb;
use crate::str::*;
use crate::sys::*;
use crate::tls::TlsConn;
use crate::win;

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

unsafe fn tcp_connect(host: &[u8], port: i64) -> Result<usize, Sb> {
    let ip = match crate::net::resolve(host) {
        Some(ip) => ip,
        None => return Err(fail("хостты шешу мүмкін емес (DNS): ".as_bytes(), host)),
    };
    let s = win::ws_socket();
    if s == win::INVALID_SOCKET {
        return Err(fail("сокет жасау мүмкін емес".as_bytes(), b""));
    }
    #[repr(C)]
    struct SockAddrIn {
        family: u16,
        port: u16,
        addr: u32,
        zero: [u8; 8],
    }
    let a = SockAddrIn { family: 2, port: (port as u16).to_be(), addr: ip.to_be(), zero: [0; 8] };
    if win::ws_connect(s, &a as *const _ as *const u8, 16) != 0 {
        win::ws_close(s);
        return Err(fail("қосылу мүмкін емес: ".as_bytes(), host));
    }
    Ok(s)
}

/// Жай TCP немесе TLS арқылы сұрау мәтінін жібереді, жауапты толықтай (қосылым жабылғанша) оқып қайтарады.
enum Conn {
    Plain(usize),
    Tls(TlsConn),
}

unsafe fn send_recv(conn: &mut Conn, req: &[u8]) -> Sb {
    let mut resp = Sb::new();
    match conn {
        Conn::Plain(s) => {
            send_plain(*s, req);
            recv_plain(*s, &mut resp);
        }
        Conn::Tls(t) => {
            t.send(req);
            t.recv_all(&mut resp);
        }
    }
    resp
}

unsafe fn send_plain(s: usize, b: &[u8]) {
    let mut off = 0;
    while off < b.len() {
        let r = win::ws_send(s, b.as_ptr().add(off), (b.len() - off).min(1 << 20) as i32);
        if r <= 0 {
            return;
        }
        off += r as usize;
    }
}

unsafe fn recv_plain(s: usize, out: &mut Sb) {
    let mut buf = [0u8; 65536];
    loop {
        let r = win::ws_recv(s, buf.as_mut_ptr(), buf.len() as i32);
        if r <= 0 {
            break;
        }
        out.push_bytes(&buf[..r as usize]);
    }
}

unsafe fn close_conn(conn: Conn) {
    match conn {
        Conn::Plain(s) => win::ws_close(s),
        Conn::Tls(t) => t.close(),
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
        i = start + size + 2;
    }
    sb.finish()
}

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
    let mut req = Sb::new();
    build_request(&mut req, method, u.host, u.port, u.https, u.path, headers, body);
    let sock = tcp_connect(u.host, u.port)?;
    let mut conn = if u.https {
        match crate::tls::connect(sock, u.host) {
            Ok(t) => Conn::Tls(t),
            Err(e) => {
                win::ws_close(sock);
                return Err(fail("TLS: ".as_bytes(), e.as_bytes()));
            }
        }
    } else {
        Conn::Plain(sock)
    };
    out_flush();
    let resp = send_recv(&mut conn, req.bytes());
    req.discard();
    close_conn(conn);
    let r = parse_response(resp.bytes());
    resp.discard();
    r.ok_or_else(|| fail("жауапты талдау мүмкін емес (жарамсыз HTTP)".as_bytes(), b""))
}

unsafe fn resolve_redirect(base: &[u8], loc: &[u8]) -> Str {
    if loc.starts_with(b"http://") || loc.starts_with(b"https://") {
        return from_bytes(loc);
    }
    let mut out = Sb::new();
    if let Ok(u) = parse_url(base) {
        out.push_bytes(if u.https { b"https://" } else { b"http://" });
        out.push_bytes(u.host);
        if u.port != if u.https { 443 } else { 80 } {
            out.push(b':');
            let mut b = [0u8; 24];
            out.push_bytes(fmt_u64(u.port as u64, &mut b));
        }
    }
    if !loc.starts_with(b"/") {
        out.push(b'/');
    }
    out.push_bytes(loc);
    out.finish()
}

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
