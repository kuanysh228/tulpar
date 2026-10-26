//! TLS клиенті: Windows-тың меншікті Schannel стегі (`secur32.dll`, `win.rs`) арқылы.
//!
//! Бұл модуль ешбір криптография жазбайды — дескан алмасуды, сертификат тізбегін тексеруді, шифрлауды
//! толығымен Windows өзі (`schannel.dll`) жасайды: сенім қоймасы (trust store), хост атауын тексеру
//! әдепкі бойынша қосылған (`SCH_CRED_MANUAL_CRED_VALIDATION` қойылмайды). Бұл модуль тек TCP сокетпен
//! шикі байттар алмасуды дескан хаттамасына (SSPI) сәйкес ұйымдастырады.

use crate::sys::*;
use crate::win;

/// Қолмен өсетін байт буфері (алдыңғы жағынан тұтыну мүмкіндігімен): Schannel-дің "артық" (SECBUFFER_EXTRA)
/// байттарын сақтау үшін. `Sb`-ден (`fmt.rs`) айырмасы — осында.
struct Buf {
    p: *mut u8,
    len: usize,
    cap: usize,
}

impl Buf {
    fn new() -> Buf {
        Buf { p: gc::mem_alloc(4096), len: 0, cap: 4096 }
    }
    fn extend(&mut self, b: &[u8]) {
        if self.len + b.len() > self.cap {
            let mut ncap = self.cap * 2;
            while ncap < self.len + b.len() {
                ncap *= 2;
            }
            let np = gc::mem_alloc(ncap);
            unsafe {
                memcpy(np, self.p, self.len);
            }
            gc::mem_free(self.p);
            self.p = np;
            self.cap = ncap;
        }
        unsafe {
            memcpy(self.p.add(self.len), b.as_ptr(), b.len());
        }
        self.len += b.len();
    }
    /// Алдыңғы `n` байтты тастайды, қалғанын басына жылжытады.
    fn consume_front(&mut self, n: usize) {
        if n >= self.len {
            self.len = 0;
            return;
        }
        unsafe {
            memmove(self.p, self.p.add(n), self.len - n);
        }
        self.len -= n;
    }
    fn free(self) {
        gc::mem_free(self.p);
    }
}

mod gc {
    pub use crate::gc::{mem_alloc, mem_free};
}

pub struct TlsConn {
    sock: usize,
    cred: win::SecHandle,
    ctx: win::SecHandle,
    sizes: win::StreamSizes,
    /// socket-тен оқылған, бірақ әлі decrypt етілмеген шифртекст
    pending: Buf,
}

/// `hostname` ASCII/UTF-8 байттары (нокта-ноталы символдар жеткілікті, сертификат CN/SAN осыған тексеріледі).
pub unsafe fn connect(sock: usize, hostname: &[u8]) -> Result<TlsConn, &'static str> {
    unsafe {
        win::secur_keep();
        let mut cred = win::acquire_client_credentials().ok_or("Schannel: тіркелгі деректерін алу мүмкін емес")?;
        let mut wname = [0u16; 256];
        let n = hostname.len().min(255);
        for i in 0..n {
            wname[i] = hostname[i] as u16; // хост атаулары ASCII (IDN қолдауы жоқ)
        }
        wname[n] = 0;

        let mut ctx = win::SecHandle::default();
        let mut io = Buf::new();
        let mut out_buf = win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_TOKEN, pv_buffer: core::ptr::null_mut() };
        let mut out_desc = win::SecBufferDesc { ul_version: 0, c_buffers: 1, p_buffers: &mut out_buf };
        let mut rc = win::initialize_security_context(&mut cred, &mut ctx, true, wname.as_ptr(), None, &mut out_desc);

        loop {
            if (rc == win::SEC_I_CONTINUE_NEEDED || rc == win::SEC_E_OK) && out_buf.cb_buffer != 0 && !out_buf.pv_buffer.is_null() {
                send_raw(sock, core::slice::from_raw_parts(out_buf.pv_buffer, out_buf.cb_buffer as usize));
                win::free_context_buffer(out_buf.pv_buffer);
            }
            if rc == win::SEC_E_OK {
                break;
            }
            if rc != win::SEC_I_CONTINUE_NEEDED && rc != win::SEC_E_INCOMPLETE_MESSAGE {
                win::free_credentials_handle(&mut cred);
                return Err("TLS дескан алмасуы сәтсіз (сертификат тексерілмеді немесе қосылым қабылданбады)");
            }
            if (rc == win::SEC_E_INCOMPLETE_MESSAGE || io.len == 0) && !recv_into(sock, &mut io) {
                win::free_credentials_handle(&mut cred);
                io.free();
                return Err("TLS дескан алмасуы кезінде қосылым жабылды");
            }
            let mut in_bufs = [
                win::SecBuffer { cb_buffer: io.len as u32, buffer_type: win::SECBUFFER_TOKEN, pv_buffer: io.p },
                win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_EMPTY, pv_buffer: core::ptr::null_mut() },
            ];
            let mut in_desc = win::SecBufferDesc { ul_version: 0, c_buffers: 2, p_buffers: in_bufs.as_mut_ptr() };
            out_buf = win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_TOKEN, pv_buffer: core::ptr::null_mut() };
            out_desc = win::SecBufferDesc { ul_version: 0, c_buffers: 1, p_buffers: &mut out_buf };
            rc = win::initialize_security_context(&mut cred, &mut ctx, false, core::ptr::null(), Some(&mut in_desc), &mut out_desc);

            if rc == win::SEC_E_OK || rc == win::SEC_I_CONTINUE_NEEDED {
                if in_bufs[1].buffer_type == win::SECBUFFER_EXTRA && in_bufs[1].cb_buffer > 0 {
                    let extra = in_bufs[1].cb_buffer as usize;
                    io.consume_front(io.len - extra);
                } else {
                    io.len = 0;
                }
            }
        }

        // Schannel өзі де дескан алмасуында сертификатты тексереді (әдепкі бойынша), бірақ бұл — тәуелсіз екінші
        // тексеру: сенім қоймасы (trust store) мен хост атауын crypt32 арқылы тікелей растаймыз.
        win::crypt_keep();
        match win::query_remote_cert(&mut ctx) {
            Some(cert) => {
                if let Err(_) = win::verify_server_cert(cert, hostname) {
                    win::delete_security_context(&mut ctx);
                    win::free_credentials_handle(&mut cred);
                    io.free();
                    return Err("сервердің сертификаты сенімсіз немесе хост атауына сәйкес келмейді");
                }
            }
            None => {
                win::delete_security_context(&mut ctx);
                win::free_credentials_handle(&mut cred);
                io.free();
                return Err("сервердің сертификатын алу мүмкін емес");
            }
        }

        let sizes = match win::query_stream_sizes(&mut ctx) {
            Some(s) => s,
            None => {
                win::delete_security_context(&mut ctx);
                win::free_credentials_handle(&mut cred);
                io.free();
                return Err("Schannel: ағын өлшемдерін алу мүмкін емес");
            }
        };
        Ok(TlsConn { sock, cred, ctx, sizes, pending: io })
    }
}

fn send_raw(sock: usize, b: &[u8]) {
    let mut off = 0;
    while off < b.len() {
        let r = unsafe { win::ws_send(sock, b.as_ptr().add(off), (b.len() - off).min(1 << 20) as i32) };
        if r <= 0 {
            return;
        }
        off += r as usize;
    }
}

impl TlsConn {
    /// Ашық мәтінді шифрлап жібереді (қажет болса максимал хабар өлшеміне бөледі).
    pub unsafe fn send(&mut self, plain: &[u8]) {
        unsafe {
            let max = self.sizes.cb_maximum_message as usize;
            let mut off = 0;
            while off < plain.len() {
                let chunk = &plain[off..(off + max).min(plain.len())];
                let total = self.sizes.cb_header as usize + chunk.len() + self.sizes.cb_trailer as usize;
                let buf = crate::gc::mem_alloc(total);
                memcpy(buf.add(self.sizes.cb_header as usize), chunk.as_ptr(), chunk.len());
                let mut bufs = [
                    win::SecBuffer { cb_buffer: self.sizes.cb_header, buffer_type: win::SECBUFFER_STREAM_HEADER, pv_buffer: buf },
                    win::SecBuffer {
                        cb_buffer: chunk.len() as u32,
                        buffer_type: win::SECBUFFER_DATA,
                        pv_buffer: buf.add(self.sizes.cb_header as usize),
                    },
                    win::SecBuffer {
                        cb_buffer: self.sizes.cb_trailer,
                        buffer_type: win::SECBUFFER_STREAM_TRAILER,
                        pv_buffer: buf.add(self.sizes.cb_header as usize + chunk.len()),
                    },
                    win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_EMPTY, pv_buffer: core::ptr::null_mut() },
                ];
                let mut desc = win::SecBufferDesc { ul_version: 0, c_buffers: 4, p_buffers: bufs.as_mut_ptr() };
                if win::encrypt_message(&mut self.ctx, &mut desc) == win::SEC_E_OK {
                    let total_sent = bufs[0].cb_buffer as usize + bufs[1].cb_buffer as usize + bufs[2].cb_buffer as usize;
                    send_raw(self.sock, core::slice::from_raw_parts(buf, total_sent));
                }
                gc::mem_free(buf);
                off += chunk.len();
            }
        }
    }

    /// Қосылым серверден жабылғанша (`close_notify` немесе socket EOF) бүкіл ашық мәтінді оқып, `out`-қа қосады.
    pub unsafe fn recv_all(&mut self, out: &mut crate::fmt::Sb) {
        unsafe {
            loop {
                loop {
                    if self.pending.len == 0 {
                        break;
                    }
                    let mut bufs = [
                        win::SecBuffer { cb_buffer: self.pending.len as u32, buffer_type: win::SECBUFFER_DATA, pv_buffer: self.pending.p },
                        win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_EMPTY, pv_buffer: core::ptr::null_mut() },
                        win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_EMPTY, pv_buffer: core::ptr::null_mut() },
                        win::SecBuffer { cb_buffer: 0, buffer_type: win::SECBUFFER_EMPTY, pv_buffer: core::ptr::null_mut() },
                    ];
                    let mut desc = win::SecBufferDesc { ul_version: 0, c_buffers: 4, p_buffers: bufs.as_mut_ptr() };
                    let rc = win::decrypt_message(&mut self.ctx, &mut desc);
                    if rc == win::SEC_E_INCOMPLETE_MESSAGE {
                        break; // socket-тен тағы оқу керек
                    }
                    if rc == win::SEC_I_CONTEXT_EXPIRED {
                        self.pending.len = 0;
                        return; // сервер жұмысты таза тоқтатты (close_notify)
                    }
                    if rc != win::SEC_E_OK {
                        return; // хаттама қатесі: бар денемізбен шектелеміз
                    }
                    let mut extra = 0usize;
                    for b in &bufs {
                        if b.buffer_type == win::SECBUFFER_DATA && !b.pv_buffer.is_null() {
                            out.push_bytes(core::slice::from_raw_parts(b.pv_buffer, b.cb_buffer as usize));
                        }
                        if b.buffer_type == win::SECBUFFER_EXTRA {
                            extra = b.cb_buffer as usize;
                        }
                    }
                    self.pending.consume_front(self.pending.len - extra);
                }
                match recv_into(self.sock, &mut self.pending) {
                    true => continue,
                    false => return, // socket жабылды: таза аяқталмаса да, барымызды қайтарамыз
                }
            }
        }
    }

    pub unsafe fn close(mut self) {
        unsafe {
            win::delete_security_context(&mut self.ctx);
            win::free_credentials_handle(&mut self.cred);
            win::ws_close(self.sock);
            self.pending.free();
        }
    }
}

unsafe fn recv_into(sock: usize, buf: &mut Buf) -> bool {
    unsafe {
        let mut tmp = [0u8; 16384];
        let r = win::ws_recv(sock, tmp.as_mut_ptr(), tmp.len() as i32);
        if r <= 0 {
            return false;
        }
        buf.extend(&tmp[..r as usize]);
        true
    }
}
