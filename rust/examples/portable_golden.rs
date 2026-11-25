//! A fingerprint of everything the portable translator computes, for checking that a change
//! to it (a refactor) changes nothing: run it before and after, then `diff` the two manifests.
//!
//!   portable_golden <rom> <out dir> <movie.m64>:<every>... [raw visual snapshot]...
//!
//! Sources are memory images: the headless game replayed through each movie, one image every
//! `<every>` frames, and the rendering build's raw snapshots ("SM64VISU", `save_state()`
//! bytes, as `portable_check quick` writes raw_<frame>.bin). For each source it records
//! `unsupported`, the heap walk, `walk` (every field), `export` -> `to_bytes`, `digest`,
//! `game_blocks`, `display_list_blocks`, the `from_bytes` round trip (and a version 1 copy of
//! states with no ghosts); and, loading the state into four target images (headless and
//! rendering, booted and mid game), `place`, `patches` and `untranslatable`. Finally, every
//! `LOAD_EVERY`-th headless state (default 20) is loaded into a fresh game with
//! `load_portable`, played 60 frames, and the report, `GameState` and export are recorded.
//!
//! The manifest (`<out dir>/manifest.txt`) has one line per output: source, what, SHA-1 and
//! length of the bytes (or the value, when it is short). Byte-identical outputs give
//! identical manifests.
use sha1::{Digest, Sha1};
use sm64_binds::portable::{
    Build, Portable, display_list_blocks, export, game_blocks, heap_all, heap_chunks, unsupported,
    walk,
};
use sm64_binds::{GamePad, SM64GameGenerator};
use std::fmt::Write as _;

fn pads(path: &str) -> Vec<GamePad> {
    let m64 = std::fs::read(path).unwrap();
    let n = u32::from_le_bytes(m64[0x18..0x1C].try_into().unwrap()) as usize;
    let n = if n == 0 || n > (m64.len() - 0x400) / 4 {
        (m64.len() - 0x400) / 4
    } else {
        n
    };
    m64[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(n)
        .map(|c| GamePad::new(u16::from_be_bytes([c[0], c[1]]), c[2] as i8, c[3] as i8))
        .collect()
}

fn raw_visual(path: &str) -> Vec<u8> {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[..8], b"SM64VISU", "{path}");
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (stack, heap) = (u(32), u(36));
    let ds = Portable::data_start(Build::Visual) as usize;
    let mut mem = vec![0u8; ds + heap];
    mem[..stack].copy_from_slice(&b[44..44 + stack]);
    mem[ds..].copy_from_slice(&b[44 + stack..]);
    mem
}

struct Manifest(String);

impl Manifest {
    fn bytes(&mut self, src: &str, what: &str, b: &[u8]) {
        let h = Sha1::digest(b);
        let hex: String = h.iter().map(|x| format!("{x:02x}")).collect();
        writeln!(self.0, "{src} {what} {hex} {}", b.len()).unwrap();
    }
    fn text(&mut self, src: &str, what: &str, s: &str) {
        if s.len() <= 120 && !s.contains('\n') {
            writeln!(self.0, "{src} {what} = {s}").unwrap();
        } else {
            self.bytes(src, what, s.as_bytes());
        }
    }
}

struct Target {
    name: String,
    build: Build,
    mem: Vec<u8>,
}

fn patches_bytes(p: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (a, d) in p {
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&(d.len() as u32).to_le_bytes());
        out.extend_from_slice(d);
    }
    out
}

fn one_source(
    m: &mut Manifest,
    src: &str,
    mem: &[u8],
    build: Build,
    targets: &[Target],
) -> Option<Portable> {
    m.text(
        src,
        "unsupported",
        &format!("{:?}", unsupported(mem, build)),
    );
    m.text(src, "heap_all", &format!("{:?}", heap_all(mem, build)));
    m.text(
        src,
        "heap_chunks",
        &format!("{:?}", heap_chunks(mem, build)),
    );
    m.text(
        src,
        "game_blocks",
        &format!("{:?}", game_blocks(mem, build)),
    );
    m.text(
        src,
        "display_list_blocks",
        &format!("{:?}", display_list_blocks(mem, build)),
    );
    match walk(mem, build) {
        Ok(w) => m.text(src, "walk", &format!("{w:?}")),
        Err(e) => m.text(src, "walk", &format!("Err({e:?})")),
    }
    let p = match export(mem, build) {
        Ok(p) => p,
        Err(e) => {
            m.text(src, "export", &format!("Err({e:?})"));
            return None;
        }
    };
    m.text(src, "export.debug", &format!("{p:?}"));
    let bytes = p.to_bytes();
    m.bytes(src, "to_bytes", &bytes);
    let d: Vec<u8> = p.digest().iter().flat_map(|x| x.to_le_bytes()).collect();
    m.bytes(src, "digest", &d);
    m.text(src, "ghost_bytes", &p.ghost_bytes().to_string());
    m.text(src, "chunk_sizes", &format!("{:?}", p.chunk_sizes()));
    let q = Portable::from_bytes(&bytes).unwrap();
    m.text(src, "from_bytes.debug", &format!("{q:?}"));
    m.text(src, "roundtrip", &(q.to_bytes() == bytes).to_string());
    m.text(
        src,
        "diff_self",
        &format!("{:?}", Portable::diff(&p, &q, 4)),
    );
    if p.ghosts == 0 {
        // The same state in format version 1: no ghost count in the header.
        let mut v1 = bytes.clone();
        v1[8..12].copy_from_slice(&1u32.to_le_bytes());
        v1.drain(48..52);
        match Portable::from_bytes(&v1) {
            Ok(r) => m.text(
                src,
                "v1",
                &format!("{:?}", (r.to_bytes() == bytes, r.ghosts)),
            ),
            Err(e) => m.text(src, "v1", &format!("Err({e:?})")),
        }
    }
    for t in targets {
        let what = format!("into:{}", t.name);
        let old = game_blocks(&t.mem, t.build).unwrap();
        let mut addrs = q.place(&old);
        m.text(src, &format!("{what}.place"), &format!("{addrs:?}"));
        // The blocks that fit none: at made-up addresses past the target's memory.
        let mut next = 0x0E00_0000u32;
        for (i, a) in addrs.iter_mut().enumerate() {
            if *a == 0 {
                *a = next;
                next += (q.chunks[i].data.len() as u32 + 15) & !15;
            }
        }
        match q.patches(t.build, &addrs) {
            Ok((pt, bad)) => {
                m.bytes(src, &format!("{what}.patches"), &patches_bytes(&pt));
                m.text(src, &format!("{what}.bad"), &bad.to_string());
            }
            Err(e) => m.text(src, &format!("{what}.patches"), &format!("Err({e:?})")),
        }
        m.text(
            src,
            &format!("{what}.untranslatable"),
            &format!("{:?}", q.untranslatable(t.build, &addrs)),
        );
    }
    Some(p)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let g = SM64GameGenerator::from_file(&a[1]).unwrap();
    let out = &a[2];
    std::fs::create_dir_all(out).unwrap();
    let movies: Vec<(String, usize)> = a[3..]
        .iter()
        .filter_map(|s| {
            s.rsplit_once(':')
                .and_then(|(p, n)| Some((p.to_string(), n.parse().ok()?)))
        })
        .collect();
    let raws: Vec<&String> = a[3..].iter().filter(|s| !s.contains(':')).collect();
    let load_every: usize = std::env::var("LOAD_EVERY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let mut m = Manifest(String::new());
    writeln!(
        m.0,
        "constants gm {:?} {:?} data {:?} {:?} tex {:?} renderer {:?} geo {:?} vars {}",
        Portable::gm(Build::Headless),
        Portable::gm(Build::Visual),
        Portable::data_start(Build::Headless),
        Portable::data_start(Build::Visual),
        Portable::texture_cache(),
        Portable::renderer(),
        Portable::geo_cache(),
        Portable::VAR_COUNT
    )
    .unwrap();
    for i in 0..Portable::VAR_COUNT {
        writeln!(
            m.0,
            "var {i} {} {:?} {:?}",
            Portable::var_name(i),
            Portable::var_at(i, Build::Headless),
            Portable::var_at(i, Build::Visual)
        )
        .unwrap();
    }

    // Targets: headless booted and mid game, the rendering build booted and mid game.
    let mut targets = Vec::new();
    {
        let game = g.create_game().unwrap();
        targets.push(Target {
            name: "h0".into(),
            build: Build::Headless,
            mem: game.with_memory(|x| x.to_vec()),
        });
    }
    if let Some((mv, _)) = movies.first() {
        let p = pads(mv);
        let mut game = g.create_game().unwrap();
        for pad in &p[..60000.min(p.len())] {
            game.step_game(*pad).unwrap();
        }
        targets.push(Target {
            name: "h60000".into(),
            build: Build::Headless,
            mem: game.with_memory(|x| x.to_vec()),
        });
    }
    if let Ok(t) = std::env::var("TARGETS_VISUAL") {
        for path in t.split(',') {
            let name = std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            targets.push(Target {
                name: format!("v:{name}"),
                build: Build::Visual,
                mem: raw_visual(path),
            });
        }
    }

    let mut loads = Vec::new();
    for (mv, every) in &movies {
        let p = pads(mv);
        let tag = std::path::Path::new(mv)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut game = g.create_game().unwrap();
        let mut k = 0;
        for f in 0..=p.len() {
            if f % every == 0 {
                let src = format!("h:{tag}@{f}");
                let st = game
                    .with_memory(|mem| one_source(&mut m, &src, mem, Build::Headless, &targets));
                if let Some(st) = st {
                    if k % load_every == 0 {
                        loads.push((src.clone(), st, p[f..(f + 60).min(p.len())].to_vec()));
                    }
                    k += 1;
                }
                eprint!("\r{src}   ");
            }
            if f < p.len() {
                game.step_game(p[f]).unwrap();
            }
        }
    }
    for r in &raws {
        let name = std::path::Path::new(r.as_str())
            .to_string_lossy()
            .replace('/', "_");
        let mem = raw_visual(r);
        one_source(&mut m, &format!("v:{name}"), &mem, Build::Visual, &targets);
        eprint!("\r{name}   ");
    }
    for (src, st, next) in &loads {
        let mut game = g.create_game().unwrap();
        let rep = game.load_portable(st).unwrap();
        m.text(src, "load_portable", &format!("{rep:?}"));
        for pad in next {
            game.step_game(*pad).unwrap();
        }
        m.text(
            src,
            "after60.state",
            &format!("{:?}", game.get_game_state().unwrap()),
        );
        match game.export_portable() {
            Ok(e) => m.bytes(src, "after60.export", &e.to_bytes()),
            Err(e) => m.text(src, "after60.export", &format!("Err({e:?})")),
        }
    }
    eprintln!();
    std::fs::write(format!("{out}/manifest.txt"), &m.0).unwrap();
    println!("{} lines -> {out}/manifest.txt", m.0.lines().count());
}
