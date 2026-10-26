//! ELF64 (статикалық, `winrt/win.ld` скриптімен жиналған) -> Windows x64 PE32+ консоль .exe.
//!
//! Әр `PT_LOAD` сегменті бір PE секциясына айналады (RVA = vaddr - ImageBase), ELF кірісі PE кірісі болады.
//! Импорт каталогы қосымша `.idata` секциясында құрылады; IAT рантаймның өз `tlp_iat_k32` кестесі (оны `.data` ішінде
//! рантайм бөлген), аттар тізімі `tlp_imp_k32` таңбасынан оқылады. Қайта орналастыру кестесі жоқ: ImageBase тұрақты.

const IMAGE_BASE: u64 = 0x1_4000_0000;
const SECTION_ALIGN: u64 = 0x1000;
const FILE_ALIGN: u64 = 0x200;
const STACK_RESERVE: u64 = 1 << 30; // 1 ГБ виртуал стек (физикалық жад қолданылған сайын ғана бөлінеді)
const STACK_COMMIT: u64 = 4 << 20;

fn align(v: u64, a: u64) -> u64 {
    (v + a - 1) / a * a
}

fn rd16(b: &[u8], o: usize) -> u64 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap()) as u64
}
fn rd32(b: &[u8], o: usize) -> u64 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as u64
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

struct Seg {
    flags: u64,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
}

fn put16(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&(x as u16).to_le_bytes());
}
fn put32(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&(x as u32).to_le_bytes());
}
fn put64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// ELF символының (мән, өлшем) жұбы.
fn find_symbol(elf: &[u8], name: &str) -> Result<(u64, u64), String> {
    let shoff = rd64(elf, 40) as usize;
    let shentsize = rd16(elf, 58) as usize;
    let shnum = rd16(elf, 60) as usize;
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        if rd32(elf, sh + 4) != 2 {
            continue; // SHT_SYMTAB
        }
        let (off, size, link, entsize) = (rd64(elf, sh + 24) as usize, rd64(elf, sh + 32) as usize, rd32(elf, sh + 40) as usize, rd64(elf, sh + 56) as usize);
        let str_sh = shoff + link * shentsize;
        let str_off = rd64(elf, str_sh + 24) as usize;
        for k in 0..size / entsize.max(24) {
            let s = off + k * 24;
            let n = rd32(elf, s) as usize;
            let nm = &elf[str_off + n..];
            let end = nm.iter().position(|&c| c == 0).unwrap_or(0);
            if &nm[..end] == name.as_bytes() {
                return Ok((rd64(elf, s + 8), rd64(elf, s + 16)));
            }
        }
    }
    Err(format!("ішкі қате: `{}` таңбасы рантаймда жоқ (libtulpar_win.a дұрыс па?)", name))
}

pub fn elf_to_pe(elf: &[u8]) -> Result<Vec<u8>, String> {
    if elf.len() < 64 || &elf[..4] != b"\x7fELF" || elf[4] != 2 {
        return Err("ішкі қате: ELF64 күтілді".into());
    }
    let entry = rd64(elf, 24);
    let (phoff, phentsize, phnum) = (rd64(elf, 32) as usize, rd16(elf, 54) as usize, rd16(elf, 56) as usize);
    let mut segs = Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        if rd32(elf, ph) == 1 {
            segs.push(Seg { flags: rd32(elf, ph + 4), offset: rd64(elf, ph + 8), vaddr: rd64(elf, ph + 16), filesz: rd64(elf, ph + 32), memsz: rd64(elf, ph + 40) });
        }
    }
    if segs.is_empty() || segs[0].vaddr < IMAGE_BASE + SECTION_ALIGN {
        return Err("ішкі қате: сегменттер ImageBase-тен төмен (win.ld)".into());
    }
    segs.sort_by_key(|s| s.vaddr);

    // импорт топтары: әр DLL үшін рантайм `tlp_iat_<топ>` және `tlp_imp_<топ>` таңбаларын береді (ws2 міндетті емес)
    struct Group {
        dll: Vec<u8>,
        names: Vec<Vec<u8>>,
        iat_addr: u64,
        iat_size: u64,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (tag, required) in [("k32", true), ("ws2", false), ("secur", false), ("crypt", false)] {
        let (iat_addr, iat_size) = match find_symbol(elf, &format!("tlp_iat_{}", tag)) {
            Ok(v) => v,
            Err(e) if required => return Err(e),
            Err(_) => continue,
        };
        let (imp_addr, imp_size) = match find_symbol(elf, &format!("tlp_imp_{}", tag)) {
            Ok(v) => v,
            Err(_) => continue, // аттар тізімі сызықтық жинауда алынып тасталса, топ да қолданылмайды
        };
        let seg = segs
            .iter()
            .find(|s| imp_addr >= s.vaddr && imp_addr + imp_size <= s.vaddr + s.filesz)
            .ok_or("ішкі қате: аттар тізімі файлда жоқ")?;
        let o = (seg.offset + imp_addr - seg.vaddr) as usize;
        let mut parts = elf[o..o + imp_size as usize].split(|&c| c == 0);
        let dll = parts.next().ok_or("ішкі қате: импорт тізімі бос")?.to_vec();
        let names: Vec<Vec<u8>> = parts.filter(|p| !p.is_empty()).map(|p| p.to_vec()).collect();
        if (names.len() as u64 + 1) * 8 != iat_size {
            return Err(format!("ішкі қате: IAT өлшемі ({}) мен импорт саны ({}) сәйкес емес", iat_size, names.len()));
        }
        groups.push(Group { dll, names, iat_addr, iat_size });
    }

    // секциялар
    let nsec = segs.len() + 1;
    let headers = align(0x80 + 4 + 20 + 240 + 40 * nsec as u64, FILE_ALIGN);
    let mut raw_ptr = headers;
    let mut rvas = Vec::new();
    for s in &segs {
        rvas.push((s.vaddr - IMAGE_BASE, raw_ptr));
        raw_ptr += align(s.filesz, FILE_ALIGN);
    }
    let last = segs.last().unwrap();
    let idata_rva = align(last.vaddr - IMAGE_BASE + last.memsz, SECTION_ALIGN);

    // .idata мазмұны: дескрипторлар (топ саны + нөлдік аяқтаушы), содан кейін әр топ үшін ILT, hint/name, DLL аты
    let mut idata = Vec::new();
    let ndesc_size = (groups.len() as u64 + 1) * 20;
    let mut layout = Vec::new(); // (ilt_off, dll_off, hint/name офсеттері)
    let mut cur = ndesc_size;
    for g in &groups {
        cur = align(cur, 8);
        let ilt_off = cur;
        cur += (g.names.len() as u64 + 1) * 8;
        let mut hn = Vec::new();
        for n in &g.names {
            hn.push(cur);
            cur += align(2 + n.len() as u64 + 1, 2);
        }
        let dll_off = cur;
        cur += align(g.dll.len() as u64 + 1, 2);
        layout.push((ilt_off, dll_off, hn));
    }
    for (g, l) in groups.iter().zip(&layout) {
        put32(&mut idata, idata_rva + l.0); // OriginalFirstThunk
        put32(&mut idata, 0); // TimeDateStamp
        put32(&mut idata, 0); // ForwarderChain
        put32(&mut idata, idata_rva + l.1); // Name
        put32(&mut idata, g.iat_addr - IMAGE_BASE); // FirstThunk
    }
    idata.extend_from_slice(&[0u8; 20]);
    for (g, l) in groups.iter().zip(&layout) {
        while (idata.len() as u64) < l.0 {
            idata.push(0);
        }
        for off in &l.2 {
            put64(&mut idata, idata_rva + off);
        }
        put64(&mut idata, 0);
        for n in &g.names {
            put16(&mut idata, 0);
            idata.extend_from_slice(n);
            idata.push(0);
            if idata.len() % 2 == 1 {
                idata.push(0);
            }
        }
        idata.extend_from_slice(&g.dll);
        idata.push(0);
        if idata.len() % 2 == 1 {
            idata.push(0);
        }
    }
    let idata_size = idata.len() as u64;
    let idata_raw = align(idata_size, FILE_ALIGN);
    let idata_raw_ptr = raw_ptr;
    let image_size = align(idata_rva + idata_size, SECTION_ALIGN);

    let mut size_code = 0;
    let mut size_idata = idata_raw;
    for s in &segs {
        if s.flags & 1 != 0 {
            size_code += align(s.filesz, FILE_ALIGN);
        } else {
            size_idata += align(s.filesz, FILE_ALIGN);
        }
    }
    let base_of_code = segs.iter().find(|s| s.flags & 1 != 0).map(|s| s.vaddr - IMAGE_BASE).unwrap_or(0x1000);

    let mut out = Vec::new();
    // DOS тақырыбы
    out.extend_from_slice(b"MZ");
    out.resize(0x3C, 0);
    put32(&mut out, 0x80);
    out.resize(0x80, 0);
    out.extend_from_slice(b"PE\0\0");
    // COFF
    put16(&mut out, 0x8664);
    put16(&mut out, nsec as u64);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put16(&mut out, 240);
    put16(&mut out, 0x0023); // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE | RELOCS_STRIPPED
    // PE32+ қосымша тақырып
    put16(&mut out, 0x20B);
    out.push(14);
    out.push(0);
    put32(&mut out, size_code);
    put32(&mut out, size_idata);
    put32(&mut out, 0);
    put32(&mut out, entry - IMAGE_BASE);
    put32(&mut out, base_of_code);
    put64(&mut out, IMAGE_BASE);
    put32(&mut out, SECTION_ALIGN);
    put32(&mut out, FILE_ALIGN);
    put16(&mut out, 6);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, 6);
    put16(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, image_size);
    put32(&mut out, headers);
    put32(&mut out, 0); // CheckSum
    put16(&mut out, 3); // Subsystem: консоль
    put16(&mut out, 0x8100); // TERMINAL_SERVER_AWARE | NX_COMPAT (DYNAMIC_BASE жоқ: ImageBase тұрақты)
    put64(&mut out, STACK_RESERVE);
    put64(&mut out, STACK_COMMIT);
    put64(&mut out, 0x10_0000);
    put64(&mut out, 0x1000);
    put32(&mut out, 0);
    put32(&mut out, 16);
    for i in 0..16 {
        match i {
            1 => {
                put32(&mut out, idata_rva);
                put32(&mut out, ndesc_size);
            }
            12 => {
                let lo = groups.iter().map(|g| g.iat_addr).min().unwrap();
                let hi = groups.iter().map(|g| g.iat_addr + g.iat_size).max().unwrap();
                put32(&mut out, lo - IMAGE_BASE);
                put32(&mut out, hi - lo);
            }
            _ => {
                put32(&mut out, 0);
                put32(&mut out, 0);
            }
        }
    }
    // секция тақырыптары
    let mut counts = [0usize; 3];
    for (s, (rva, ptr)) in segs.iter().zip(&rvas) {
        let (name, ch, k) = if s.flags & 1 != 0 {
            (".text", 0x6000_0020u64, 0)
        } else if s.flags & 2 != 0 {
            (".data", 0xC000_0040, 2)
        } else {
            (".rdata", 0x4000_0040, 1)
        };
        counts[k] += 1;
        let mut nm = name.as_bytes().to_vec();
        if counts[k] > 1 {
            nm.push(b'0' + counts[k] as u8);
        }
        nm.resize(8, 0);
        out.extend_from_slice(&nm);
        put32(&mut out, s.memsz.max(1));
        put32(&mut out, *rva);
        put32(&mut out, align(s.filesz, FILE_ALIGN));
        put32(&mut out, if s.filesz == 0 { 0 } else { *ptr });
        put32(&mut out, 0);
        put32(&mut out, 0);
        put32(&mut out, 0);
        put32(&mut out, ch);
    }
    out.extend_from_slice(b".idata\0\0");
    put32(&mut out, idata_size);
    put32(&mut out, idata_rva);
    put32(&mut out, idata_raw);
    put32(&mut out, idata_raw_ptr);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, 0);
    put32(&mut out, 0x4000_0040);
    out.resize(headers as usize, 0);
    // секция деректері
    for (s, (_, ptr)) in segs.iter().zip(&rvas) {
        if s.filesz == 0 {
            continue;
        }
        debug_assert_eq!(out.len() as u64, *ptr);
        out.extend_from_slice(&elf[s.offset as usize..(s.offset + s.filesz) as usize]);
        out.resize(align(out.len() as u64, FILE_ALIGN) as usize, 0);
    }
    debug_assert_eq!(out.len() as u64, idata_raw_ptr);
    out.extend_from_slice(&idata);
    out.resize(align(out.len() as u64, FILE_ALIGN) as usize, 0);
    Ok(out)
}
