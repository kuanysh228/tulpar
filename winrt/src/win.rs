//! kernel32 импорттары. PE жазушы (`compiler/src/pe.rs`) `tlp_imp_k32` (атаулар тізімі) мен `tlp_iat_k32`
//! (импорт адрестері кестесі) таңбаларын ELF-тен тауып, импорт каталогын құрады; Windows жүктеушісі кестені толтырады.

/// DLL импорт тобы: `K` — функция индекстері, `$iat` — Windows жүктеуші толтыратын кесте (соңында нөл),
/// `$imp` — "dll\0функция\0функция\0..." (кесте индекстерімен бірдей ретпен). PE жазушы екеуін де таңба бойынша табады.
macro_rules! import_table {
    ($dll:literal, $iat:ident, $imp:ident; $($name:ident),* $(,)?) => {
        #[allow(non_camel_case_types, dead_code)]
        #[repr(usize)]
        pub enum K { $($name),*, End }

        const NAMES: &str = concat!($dll, "\0", $(stringify!($name), "\0"),*);
        const NLEN: usize = NAMES.len();

        const fn to_array(s: &str) -> [u8; NLEN] {
            let b = s.as_bytes();
            let mut a = [0u8; NLEN];
            let mut i = 0;
            while i < NLEN {
                a[i] = b[i];
                i += 1;
            }
            a
        }

        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".data.tlp_iat")]
        pub static mut $iat: [usize; K::End as usize + 1] = [0; K::End as usize + 1];

        #[unsafe(no_mangle)]
        pub static $imp: [u8; NLEN] = to_array(NAMES);

        /// `$imp` сілтемесі: сызықтық жинау (`--gc-sections`) аттар тізімін алып тастамауы үшін
        pub fn keep() {
            unsafe {
                core::ptr::read_volatile(&$imp[0]);
            }
        }

        #[inline(always)]
        pub unsafe fn imp<T: Copy>(i: K) -> T {
            unsafe { transmute_copy(&(*core::ptr::addr_of!($iat))[i as usize]) }
        }
    };
}

mod k32_table {
    use core::mem::transmute_copy;
    import_table!(
        "kernel32.dll", tlp_iat_k32, tlp_imp_k32;
        ExitProcess, GetStdHandle, WriteFile, ReadFile, CloseHandle, VirtualAlloc, VirtualFree, GetCommandLineW,
        SetConsoleOutputCP, SetConsoleCP, GetSystemTimePreciseAsFileTime, QueryPerformanceCounter,
        AddVectoredExceptionHandler, SetThreadStackGuarantee, CreateFileW, DeleteFileW, MoveFileExW, CreateDirectoryW,
        GetFileAttributesW,
    );
}

mod secur_table {
    use core::mem::transmute_copy;
    import_table!(
        "secur32.dll", tlp_iat_secur, tlp_imp_secur;
        AcquireCredentialsHandleW, InitializeSecurityContextW, DeleteSecurityContext, FreeCredentialsHandle,
        FreeContextBuffer, QueryContextAttributesW, EncryptMessage, DecryptMessage,
    );
}

mod crypt_table {
    use core::mem::transmute_copy;
    import_table!(
        "crypt32.dll", tlp_iat_crypt, tlp_imp_crypt;
        CertGetCertificateChain, CertVerifyCertificateChainPolicy, CertFreeCertificateChain, CertFreeCertificateContext,
    );
}

mod ws2_table {
    use core::mem::transmute_copy;
    import_table!(
        "ws2_32.dll", tlp_iat_ws2, tlp_imp_ws2;
        WSAStartup, socket, bind, listen, accept, connect, recv, send, closesocket, setsockopt,
        GetAddrInfoW, FreeAddrInfoW,
    );
}

use k32_table::{imp, K};

/// kernel32 аттар тізімін сілтеу
pub fn keep_imports() {
    k32_table::keep();
}

pub unsafe fn exit_process(code: u32) -> ! {
    unsafe {
        let f: extern "win64" fn(u32) -> ! = imp(K::ExitProcess);
        f(code)
    }
}
pub unsafe fn get_std_handle(n: u32) -> usize {
    unsafe {
        let f: extern "win64" fn(u32) -> usize = imp(K::GetStdHandle);
        f(n)
    }
}
pub unsafe fn write_file(h: usize, p: *const u8, n: u32, written: *mut u32) -> i32 {
    unsafe {
        let f: extern "win64" fn(usize, *const u8, u32, *mut u32, usize) -> i32 = imp(K::WriteFile);
        f(h, p, n, written, 0)
    }
}
pub unsafe fn read_file(h: usize, p: *mut u8, n: u32, read: *mut u32) -> i32 {
    unsafe {
        let f: extern "win64" fn(usize, *mut u8, u32, *mut u32, usize) -> i32 = imp(K::ReadFile);
        f(h, p, n, read, 0)
    }
}
pub unsafe fn close_handle(h: usize) {
    unsafe {
        let f: extern "win64" fn(usize) -> i32 = imp(K::CloseHandle);
        f(h);
    }
}
pub unsafe fn virtual_alloc(addr: usize, size: usize, ty: u32, prot: u32) -> usize {
    unsafe {
        let f: extern "win64" fn(usize, usize, u32, u32) -> usize = imp(K::VirtualAlloc);
        f(addr, size, ty, prot)
    }
}
pub unsafe fn virtual_free(addr: usize, size: usize, ty: u32) -> i32 {
    unsafe {
        let f: extern "win64" fn(usize, usize, u32) -> i32 = imp(K::VirtualFree);
        f(addr, size, ty)
    }
}
pub unsafe fn get_command_line() -> *const u16 {
    unsafe {
        let f: extern "win64" fn() -> *const u16 = imp(K::GetCommandLineW);
        f()
    }
}
pub unsafe fn set_console_cp(cp: u32) {
    unsafe {
        let o: extern "win64" fn(u32) -> i32 = imp(K::SetConsoleOutputCP);
        let i: extern "win64" fn(u32) -> i32 = imp(K::SetConsoleCP);
        o(cp);
        i(cp);
    }
}
pub unsafe fn system_time_100ns() -> u64 {
    unsafe {
        let f: extern "win64" fn(*mut u64) = imp(K::GetSystemTimePreciseAsFileTime);
        let mut t = 0u64;
        f(&mut t);
        t
    }
}
pub unsafe fn perf_counter() -> u64 {
    unsafe {
        let f: extern "win64" fn(*mut u64) -> i32 = imp(K::QueryPerformanceCounter);
        let mut t = 0u64;
        f(&mut t);
        t
    }
}
pub unsafe fn add_exception_handler(h: unsafe extern "win64" fn(*const u8) -> i32) {
    unsafe {
        let f: extern "win64" fn(u32, unsafe extern "win64" fn(*const u8) -> i32) -> usize = imp(K::AddVectoredExceptionHandler);
        f(1, h);
        let g: extern "win64" fn(*mut u32) -> i32 = imp(K::SetThreadStackGuarantee);
        let mut guarantee = 64 * 1024u32;
        g(&mut guarantee);
    }
}
pub unsafe fn create_file(path: *const u16, access: u32, share: u32, disposition: u32) -> usize {
    unsafe {
        let f: extern "win64" fn(*const u16, u32, u32, usize, u32, u32, usize) -> usize = imp(K::CreateFileW);
        f(path, access, share, 0, disposition, 0x80, 0)
    }
}
pub unsafe fn delete_file(path: *const u16) {
    unsafe {
        let f: extern "win64" fn(*const u16) -> i32 = imp(K::DeleteFileW);
        f(path);
    }
}
pub unsafe fn move_file(from: *const u16, to: *const u16) -> bool {
    unsafe {
        let f: extern "win64" fn(*const u16, *const u16, u32) -> i32 = imp(K::MoveFileExW);
        f(from, to, 1) != 0
    }
}
pub unsafe fn create_directory(path: *const u16) {
    unsafe {
        let f: extern "win64" fn(*const u16, usize) -> i32 = imp(K::CreateDirectoryW);
        f(path, 0);
    }
}
pub unsafe fn file_attributes(path: *const u16) -> u32 {
    unsafe {
        let f: extern "win64" fn(*const u16) -> u32 = imp(K::GetFileAttributesW);
        f(path)
    }
}

// ------------------------------------------------------------------ Winsock (ws2_32)

pub const INVALID_SOCKET: usize = usize::MAX;

pub fn ws_keep() {
    ws2_table::keep();
}

pub unsafe fn ws_startup() {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(u16, *mut u8) -> i32 = imp(K::WSAStartup);
        let mut data = [0u8; 512]; // WSADATA
        f(0x0202, data.as_mut_ptr());
    }
}
pub unsafe fn ws_socket() -> usize {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(i32, i32, i32) -> usize = imp(K::socket);
        f(2, 1, 6)
    }
}
pub unsafe fn ws_bind(s: usize, addr: *const u8, len: i32) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, *const u8, i32) -> i32 = imp(K::bind);
        f(s, addr, len)
    }
}
pub unsafe fn ws_listen(s: usize, backlog: i32) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, i32) -> i32 = imp(K::listen);
        f(s, backlog)
    }
}
pub unsafe fn ws_accept(s: usize) -> usize {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, usize, usize) -> usize = imp(K::accept);
        f(s, 0, 0)
    }
}
pub unsafe fn ws_connect(s: usize, addr: *const u8, len: i32) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, *const u8, i32) -> i32 = imp(K::connect);
        f(s, addr, len)
    }
}
pub unsafe fn ws_recv(s: usize, buf: *mut u8, len: i32) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, *mut u8, i32, i32) -> i32 = imp(K::recv);
        f(s, buf, len, 0)
    }
}
pub unsafe fn ws_send(s: usize, buf: *const u8, len: i32) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, *const u8, i32, i32) -> i32 = imp(K::send);
        f(s, buf, len, 0)
    }
}
pub unsafe fn ws_close(s: usize) {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize) -> i32 = imp(K::closesocket);
        f(s);
    }
}
/// `SOL_SOCKET` деңгейіндегі DWORD параметрі
pub unsafe fn ws_setsockopt_u32(s: usize, opt: i32, val: u32) {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(usize, i32, i32, *const u32, i32) -> i32 = imp(K::setsockopt);
        f(s, 0xFFFF, opt, &val, 4);
    }
}

#[repr(C)]
pub struct AddrInfoW {
    pub ai_flags: i32,
    pub ai_family: i32,
    pub ai_socktype: i32,
    pub ai_protocol: i32,
    pub ai_addrlen: usize,
    pub ai_canonname: *mut u16,
    pub ai_addr: *mut u8,
    pub ai_next: *mut AddrInfoW,
}

/// IPv4 (`AF_INET`/`SOCK_STREAM`) бойынша DNS сұрауы: табылса 0 қайтарады, `*out` тізімнің басына меңзейді.
pub unsafe fn getaddrinfo_ipv4(host: *const u16, out: &mut *mut AddrInfoW) -> i32 {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(*const u16, *const u16, *const AddrInfoW, *mut *mut AddrInfoW) -> i32 = imp(K::GetAddrInfoW);
        let hints = AddrInfoW { ai_flags: 0, ai_family: 2, ai_socktype: 1, ai_protocol: 6, ai_addrlen: 0, ai_canonname: core::ptr::null_mut(), ai_addr: core::ptr::null_mut(), ai_next: core::ptr::null_mut() };
        f(host, core::ptr::null(), &hints, out)
    }
}

pub unsafe fn freeaddrinfo(p: *mut AddrInfoW) {
    unsafe {
        use ws2_table::{imp, K};
        let f: extern "win64" fn(*mut AddrInfoW) = imp(K::FreeAddrInfoW);
        f(p);
    }
}

// ------------------------------------------------------------------ Schannel (secur32, TLS)

pub fn secur_keep() {
    secur_table::keep();
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SecHandle {
    pub lower: usize,
    pub upper: usize,
}

#[repr(C)]
pub struct SecBuffer {
    pub cb_buffer: u32,
    pub buffer_type: u32,
    pub pv_buffer: *mut u8,
}

#[repr(C)]
pub struct SecBufferDesc {
    pub ul_version: u32,
    pub c_buffers: u32,
    pub p_buffers: *mut SecBuffer,
}

#[repr(C)]
pub struct StreamSizes {
    pub cb_header: u32,
    pub cb_trailer: u32,
    pub cb_maximum_message: u32,
    pub c_buffers: u32,
    pub cb_block_size: u32,
}

#[repr(C)]
struct SchannelCred {
    dw_version: u32,
    c_creds: u32,
    pa_cred: usize,
    h_root_store: usize,
    c_mappers: u32,
    aph_mappers: usize,
    c_supported_algs: u32,
    palg_supported_algs: usize,
    grbit_enabled_protocols: u32,
    dw_minimum_cipher_strength: u32,
    dw_maximum_cipher_strength: u32,
    dw_session_lifespan: u32,
    dw_flags: u32,
    dw_cred_format: u32,
}

pub const SECBUFFER_EMPTY: u32 = 0;
pub const SECBUFFER_DATA: u32 = 1;
pub const SECBUFFER_TOKEN: u32 = 2;
pub const SECBUFFER_EXTRA: u32 = 5;
pub const SECBUFFER_STREAM_TRAILER: u32 = 6;
pub const SECBUFFER_STREAM_HEADER: u32 = 7;

pub const SEC_E_OK: i32 = 0x0000_0000u32 as i32;
pub const SEC_I_CONTINUE_NEEDED: i32 = 0x0009_0312u32 as i32;
pub const SEC_E_INCOMPLETE_MESSAGE: i32 = 0x8009_0318u32 as i32;
pub const SEC_I_CONTEXT_EXPIRED: i32 = 0x0009_0317u32 as i32;

const ISC_REQ_REPLAY_DETECT: u32 = 0x0000_0004;
const ISC_REQ_SEQUENCE_DETECT: u32 = 0x0000_0008;
const ISC_REQ_CONFIDENTIALITY: u32 = 0x0000_0010;
const ISC_REQ_ALLOCATE_MEMORY: u32 = 0x0000_0100;
const ISC_REQ_STREAM: u32 = 0x0000_8000;
const SECURITY_NATIVE_DREP: u32 = 0x0000_0010;
const SP_PROT_TLS1_2_CLIENT: u32 = 0x0000_0800;
const SCH_CRED_V3: u32 = 3;
const SCH_CRED_NO_DEFAULT_CREDS: u32 = 0x0000_0010;
const SCH_USE_STRONG_CRYPTO: u32 = 0x0040_0000;
const SECPKG_CRED_OUTBOUND: u32 = 2;
const SECPKG_ATTR_STREAM_SIZES: u32 = 4;
const SECPKG_ATTR_REMOTE_CERT_CONTEXT: u32 = 0x53;

/// Клиенттің тіркелгі деректерін алады (сертификатсыз, тек TLS 1.2 клиент режимі).
pub unsafe fn acquire_client_credentials() -> Option<SecHandle> {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*const u16, *const u16, u32, usize, *const SchannelCred, usize, usize, *mut SecHandle, *mut i64) -> i32 =
            imp(K::AcquireCredentialsHandleW);
        let unisp: [u16; 45] = [
            0x4D, 0x69, 0x63, 0x72, 0x6F, 0x73, 0x6F, 0x66, 0x74, 0x20, 0x55, 0x6E, 0x69, 0x66, 0x69, 0x65, 0x64, 0x20, 0x53, 0x65, 0x63, 0x75,
            0x72, 0x69, 0x74, 0x79, 0x20, 0x50, 0x72, 0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x20, 0x50, 0x72, 0x6F, 0x76, 0x69, 0x64, 0x65, 0x72, 0,
        ]; // "Microsoft Unified Security Protocol Provider"
        let cred = SchannelCred {
            dw_version: SCH_CRED_V3,
            c_creds: 0,
            pa_cred: 0,
            h_root_store: 0,
            c_mappers: 0,
            aph_mappers: 0,
            c_supported_algs: 0,
            palg_supported_algs: 0,
            grbit_enabled_protocols: SP_PROT_TLS1_2_CLIENT,
            dw_minimum_cipher_strength: 0,
            dw_maximum_cipher_strength: 0,
            dw_session_lifespan: 0,
            dw_flags: SCH_CRED_NO_DEFAULT_CREDS | SCH_USE_STRONG_CRYPTO,
            dw_cred_format: 0,
        };
        let mut h = SecHandle::default();
        let mut expiry = 0i64;
        let r = f(core::ptr::null(), unisp.as_ptr(), SECPKG_CRED_OUTBOUND, 0, &cred, 0, 0, &mut h, &mut expiry);
        if r == SEC_E_OK { Some(h) } else { None }
    }
}

pub unsafe fn free_credentials_handle(h: &mut SecHandle) {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle) -> i32 = imp(K::FreeCredentialsHandle);
        f(h);
    }
}

/// Тапсырысшы жағының дескан алмасуының бір қадамы. `first` — бірінші шақыру ма (ескі контекст жоқ).
/// `input`/`output` — бір `SecBuffer`-ден тұратын дескрипторлар (кіру/шығу токендері).
pub unsafe fn initialize_security_context(
    cred: &mut SecHandle,
    ctx: &mut SecHandle,
    first: bool,
    target: *const u16,
    input: Option<&mut SecBufferDesc>,
    output: &mut SecBufferDesc,
) -> i32 {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(
            *mut SecHandle,
            *mut SecHandle,
            *const u16,
            u32,
            u32,
            u32,
            *mut SecBufferDesc,
            u32,
            *mut SecHandle,
            *mut SecBufferDesc,
            *mut u32,
            *mut i64,
        ) -> i32 = imp(K::InitializeSecurityContextW);
        let flags = ISC_REQ_SEQUENCE_DETECT | ISC_REQ_REPLAY_DETECT | ISC_REQ_CONFIDENTIALITY | ISC_REQ_ALLOCATE_MEMORY | ISC_REQ_STREAM;
        let in_ptr = input.map(|i| i as *mut SecBufferDesc).unwrap_or(core::ptr::null_mut());
        let old_ctx = if first { core::ptr::null_mut() } else { ctx as *mut SecHandle };
        let mut attrs = 0u32;
        let mut expiry = 0i64;
        f(cred, old_ctx, target, flags, 0, SECURITY_NATIVE_DREP, in_ptr, 0, ctx, output, &mut attrs, &mut expiry)
    }
}

pub unsafe fn delete_security_context(ctx: &mut SecHandle) {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle) -> i32 = imp(K::DeleteSecurityContext);
        f(ctx);
    }
}

pub unsafe fn free_context_buffer(p: *mut u8) {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut u8) -> i32 = imp(K::FreeContextBuffer);
        f(p);
    }
}

pub unsafe fn query_stream_sizes(ctx: &mut SecHandle) -> Option<StreamSizes> {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle, u32, *mut StreamSizes) -> i32 = imp(K::QueryContextAttributesW);
        let mut sizes = StreamSizes { cb_header: 0, cb_trailer: 0, cb_maximum_message: 0, c_buffers: 0, cb_block_size: 0 };
        if f(ctx, SECPKG_ATTR_STREAM_SIZES, &mut sizes) == SEC_E_OK { Some(sizes) } else { None }
    }
}

/// Серверден алынған сертификаттың `CERT_CONTEXT` көрсеткішін қайтарады (бостаруды `verify_server_cert` жасайды).
pub unsafe fn query_remote_cert(ctx: &mut SecHandle) -> Option<*mut u8> {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle, u32, *mut *mut u8) -> i32 = imp(K::QueryContextAttributesW);
        let mut p: *mut u8 = core::ptr::null_mut();
        if f(ctx, SECPKG_ATTR_REMOTE_CERT_CONTEXT, &mut p) == SEC_E_OK && !p.is_null() { Some(p) } else { None }
    }
}

pub unsafe fn encrypt_message(ctx: &mut SecHandle, msg: &mut SecBufferDesc) -> i32 {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle, u32, *mut SecBufferDesc, u32) -> i32 = imp(K::EncryptMessage);
        f(ctx, 0, msg, 0)
    }
}

pub unsafe fn decrypt_message(ctx: &mut SecHandle, msg: &mut SecBufferDesc) -> i32 {
    unsafe {
        use secur_table::{imp, K};
        let f: extern "win64" fn(*mut SecHandle, *mut SecBufferDesc, u32, *mut u32) -> i32 = imp(K::DecryptMessage);
        let mut qop = 0u32;
        f(ctx, msg, 0, &mut qop)
    }
}

// ------------------------------------------------------------------ crypt32 (сертификат тізбегін тексеру)
//
// Schannel дескан алмасуында сертификатты өзі де тексереді (әдепкі бойынша, `SCH_CRED_MANUAL_CRED_VALIDATION`
// қойылмаса); мұндағы қосымша тексеру — екінші, тәуелсіз қабат: хост атауы мен сенім тізбегін crypt32 арқылы
// тікелей растайды, сол арқылы бір стектің (Schannel) ішкі мінез-құлқына толық тәуелді болмайды.

pub fn crypt_keep() {
    crypt_table::keep();
}

#[repr(C)]
struct CertUsageMatch {
    dw_type: u32,
    cusage_identifier: u32,
    rgpsz_usage_identifier: *const *const u8,
}

#[repr(C)]
struct CertChainPara {
    cb_size: u32,
    requested_usage: CertUsageMatch,
}

#[repr(C)]
struct CertChainPolicyPara {
    cb_size: u32,
    dw_flags: u32,
    pv_extra: *const SslExtraCertChainPolicyPara,
}

#[repr(C)]
struct SslExtraCertChainPolicyPara {
    cb_size: u32,
    dw_auth_type: u32,
    fdw_checks: u32,
    pwsz_server_name: *const u16,
}

#[repr(C)]
struct CertChainPolicyStatus {
    cb_size: u32,
    dw_error: u32,
    l_chain_index: i32,
    l_element_index: i32,
    pv_extra_status: usize,
}

const CERT_CHAIN_POLICY_SSL: usize = 4;
const AUTHTYPE_SERVER: u32 = 2;

/// Қолданыстағы байт тізбегін (UTF-8) кең (UTF-16) жолға айналдырады (хост атаулары ASCII болады деп есептейді).
fn ascii_to_wide(host: &[u8], out: &mut [u16; 256]) -> *const u16 {
    let n = host.len().min(255);
    for i in 0..n {
        out[i] = host[i] as u16;
    }
    out[n] = 0;
    out.as_ptr()
}

/// Schannel дескан алмасуынан кейін серверлің сертификат тізбегін crypt32 арқылы тексереді: сенім қоймасы
/// (trust store) мен хост атауының сәйкестігі. `cert_ctx` — `SECPKG_ATTR_REMOTE_CERT_CONTEXT` арқылы алынған.
/// Сәтсіз болса, қате кодын (`CertChainPolicyStatus.dwError`, нөл емес) қайтарады.
pub unsafe fn verify_server_cert(cert_ctx: *mut u8, hostname: &[u8]) -> Result<(), u32> {
    unsafe {
        use crypt_table::{imp, K};
        let get_chain: extern "win64" fn(usize, *mut u8, usize, usize, *const CertChainPara, u32, usize, *mut *mut u8) -> i32 =
            imp(K::CertGetCertificateChain);
        let verify: extern "win64" fn(usize, *mut u8, *const CertChainPolicyPara, *mut CertChainPolicyStatus) -> i32 =
            imp(K::CertVerifyCertificateChainPolicy);
        let free_chain: extern "win64" fn(*mut u8) = imp(K::CertFreeCertificateChain);
        let free_ctx: extern "win64" fn(*mut u8) -> i32 = imp(K::CertFreeCertificateContext);

        // "1.3.6.1.5.5.7.3.1" (szOID_PKIX_KP_SERVER_AUTH), серверді растау мақсаты
        let oid = b"1.3.6.1.5.5.7.3.1 ";
        let usage_ids = [oid.as_ptr()];
        let para = CertChainPara {
            cb_size: core::mem::size_of::<CertChainPara>() as u32,
            requested_usage: CertUsageMatch { dw_type: 0, cusage_identifier: 1, rgpsz_usage_identifier: usage_ids.as_ptr() },
        };
        let mut chain: *mut u8 = core::ptr::null_mut();
        let r = get_chain(0, cert_ctx, 0, 0, &para, 0, 0, &mut chain);
        free_ctx(cert_ctx);
        if r == 0 || chain.is_null() {
            return Err(0xFFFF_FFFF);
        }

        let mut wname = [0u16; 256];
        let extra = SslExtraCertChainPolicyPara {
            cb_size: core::mem::size_of::<SslExtraCertChainPolicyPara>() as u32,
            dw_auth_type: AUTHTYPE_SERVER,
            fdw_checks: 0,
            pwsz_server_name: ascii_to_wide(hostname, &mut wname),
        };
        let pol = CertChainPolicyPara { cb_size: core::mem::size_of::<CertChainPolicyPara>() as u32, dw_flags: 0, pv_extra: &extra };
        let mut status =
            CertChainPolicyStatus { cb_size: core::mem::size_of::<CertChainPolicyStatus>() as u32, dw_error: 0, l_chain_index: 0, l_element_index: 0, pv_extra_status: 0 };
        let ok = verify(CERT_CHAIN_POLICY_SSL, chain, &pol, &mut status);
        free_chain(chain);
        if ok != 0 && status.dw_error == 0 { Ok(()) } else { Err(if ok == 0 { 0xFFFF_FFFE } else { status.dw_error }) }
    }
}
