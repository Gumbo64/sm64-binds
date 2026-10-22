//! Compiles c/oracle_dump.c against a local sm64_physics build (US) and links
//! its static library. SM64_PHYSICS_DIR is the sm64_physics checkout, built
//! with `cmake -S . -B build-us -DCMAKE_BUILD_TYPE=Release -DSM64_VERSION=us`.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SM64_PHYSICS_DIR");
    println!("cargo:rerun-if-changed=c/oracle_dump.c");
    let p = PathBuf::from(
        std::env::var("SM64_PHYSICS_DIR")
            .expect("set SM64_PHYSICS_DIR to an sm64_physics checkout"),
    );
    let (g, b) = (p.join("game"), p.join("build-us"));
    let gv = g.join("gen/us");
    let lib = b.join("libsm64_physics.a");
    // gRandomSeed16 is a static: address it from gGlobalTimer, in the same section.
    let nm = Command::new("nm").arg(&lib).output().expect("nm");
    let sym = |name: &str| -> i64 {
        String::from_utf8_lossy(&nm.stdout)
            .lines()
            .find_map(|l| {
                let c: Vec<&str> = l.split_whitespace().collect();
                (c.len() == 3 && c[2] == name && c[1] != "U")
                    .then(|| i64::from_str_radix(c[0], 16).unwrap())
            })
            .unwrap_or_else(|| panic!("{name} not in {}", lib.display()))
    };
    let seed_offset = sym("gRandomSeed16") - sym("gGlobalTimer");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let inc = |d: PathBuf| format!("-I{}", d.display());
    let st = Command::new("gcc")
        .args(["-O2", "-fPIC", "-c", "c/oracle_dump.c", "-o"])
        .arg(out.join("oracle_dump.o"))
        .arg(format!("-DSEED_OFFSET=({seed_offset})"))
        .args([
            inc(p.join("include")),
            inc(g.clone()),
            inc(gv.clone()),
            inc(gv.join("include")),
            inc(b.join("rom_stubs")),
            inc(g.join("include")),
            inc(g.join("src")),
            inc(p.join("platform")),
            inc(b.join("n64stack")),
        ])
        .args([
            "-DVERSION_US=1",
            "-DF3D_OLD=1",
            "-D_FINALROM=1",
            "-D_LANGUAGE_C",
            "-DNON_MATCHING",
            "-DAVOID_UB",
            "-DNO_SEGMENTED_MEMORY",
        ])
        .status()
        .expect("gcc");
    assert!(st.success(), "compiling c/oracle_dump.c failed");
    let st = Command::new("ar")
        .arg("rcs")
        .arg(out.join("liboracle_dump.a"))
        .arg(out.join("oracle_dump.o"))
        .status()
        .expect("ar");
    assert!(st.success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", b.display());
    println!("cargo:rustc-link-lib=static=oracle_dump");
    println!("cargo:rustc-link-lib=static=sm64_physics");
    println!("cargo:rustc-link-lib=m");
}
